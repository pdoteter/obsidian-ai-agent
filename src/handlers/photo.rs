use std::sync::Arc;

use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{error, info};

use crate::ai::AiService;
use crate::config::Config;
use crate::error::ImageError;
use crate::git::chat_tracker::ChatIdTracker;
use crate::git::debounce::SyncNotifier;
use crate::vault::daily_note::DailyNoteManager;

/// Check whether a Telegram Document represents an image based on mime_type or file extension.
pub fn is_image_document(doc: &teloxide::types::Document) -> bool {
    if let Some(mime) = &doc.mime_type {
        if mime.as_ref().starts_with("image/") {
            return true;
        }
    }
    if let Some(filename) = &doc.file_name {
        let lower = filename.to_lowercase();
        if lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".png")
            || lower.ends_with(".webp")
            || lower.ends_with(".heic")
            || lower.ends_with(".heif")
            || lower.ends_with(".tiff")
            || lower.ends_with(".tif")
            || lower.ends_with(".bmp")
            || lower.ends_with(".gif")
        {
            return true;
        }
    }
    false
}

/// Handle incoming photo messages: download → resize → EXIF → classify → save → append to vault
pub async fn handle_photo_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Auth check
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring photo");
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    chat_tracker.set(msg.chat.id).await;

    // 2. Extract photo (highest resolution)
    let photos = msg.photo().ok_or("No photo in message").map_err(|e| {
        error!(error = %e, "Photo message missing photo payload");
        Box::new(ImageError::Download(e.to_string())) as Box<dyn std::error::Error + Send + Sync>
    })?;
    let photo = photos.last().ok_or("Empty photo array").map_err(|e| {
        error!(error = %e, "Photo array was empty");
        Box::new(ImageError::Download(e.to_string())) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // 3. Extract caption
    let caption = msg.caption().map(|s| s.to_string());

    // 4. Download to memory
    let file = bot.get_file(&photo.file.id).await.map_err(|e| {
        error!(error = %e, "Failed to fetch Telegram file metadata for photo");
        Box::new(ImageError::Download(e.to_string())) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download photo bytes from Telegram");
            Box::new(ImageError::Download(e.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

    info!(
        size_bytes = bytes.len(),
        has_caption = caption.is_some(),
        "Downloaded photo"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadPhoto)
        .await?;

    // Process the photo entry (resize, EXIF extract, classify, save, append, notify sync)
    let process_result = process_photo_entry(
        &bytes,
        caption.as_deref(),
        &config,
        &ai_service,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((_filename, summary, exif_missing)) => {
            // 16. Send confirmation
            let reply = if exif_missing {
                format!(
                    "📸 Foto opgeslagen — {}\n⚠️ Geen EXIF-datum gevonden. Tijdstip van verzenden gebruikt. (Tip: verstuur foto's als bestand/document om EXIF te behouden)",
                    summary
                )
            } else {
                format!("📸 Foto opgeslagen — {}", summary)
            };
            bot.send_message(msg.chat.id, reply).await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to process photo entry");
            bot.send_message(msg.chat.id, format!("❌ Failed to save photo: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Handle incoming photo document messages (sent uncompressed as a file): download → resize → EXIF → classify → save → append to vault
pub async fn handle_photo_document_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Auth check
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(
                user_id = user.id.0,
                "Unauthorized user, ignoring photo document"
            );
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    chat_tracker.set(msg.chat.id).await;

    // 2. Extract document payload
    let doc = msg
        .document()
        .ok_or("No document in message")
        .map_err(|e| {
            error!(error = %e, "Photo document message missing document payload");
            Box::new(ImageError::Download(e.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

    // 3. Extract caption
    let caption = msg.caption().map(|s| s.to_string());

    // 4. Download to memory
    let file = bot.get_file(&doc.file.id).await.map_err(|e| {
        error!(
            error = %e,
            "Failed to fetch Telegram file metadata for photo document"
        );
        Box::new(ImageError::Download(e.to_string())) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(
                error = %e,
                "Failed to download photo document bytes from Telegram"
            );
            Box::new(ImageError::Download(e.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

    info!(
        size_bytes = bytes.len(),
        has_caption = caption.is_some(),
        "Downloaded photo document"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadPhoto)
        .await?;

    // Process the photo entry (resize, EXIF extract, classify, save, append, notify sync)
    let process_result = process_photo_entry(
        &bytes,
        caption.as_deref(),
        &config,
        &ai_service,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((_filename, summary, exif_missing)) => {
            let reply = if exif_missing {
                format!(
                    "📸 Foto opgeslagen — {}\n⚠️ Geen EXIF-datum gevonden. Tijdstip van verzenden gebruikt. (Tip: verstuur foto's als bestand/document om EXIF te behouden)",
                    summary
                )
            } else {
                format!("📸 Foto opgeslagen — {}", summary)
            };
            bot.send_message(msg.chat.id, reply).await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to process photo document entry");
            bot.send_message(msg.chat.id, format!("❌ Failed to save photo: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Process a photo entry: resize → EXIF → classify via Vision AI → save to vault → write to note.
/// Returns the saved filename, classification summary, and a boolean indicating if EXIF was missing.
pub async fn process_photo_entry(
    bytes: &[u8],
    caption: Option<&str>,
    config: &Config,
    ai_service: &AiService,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
) -> Result<(String, String, bool), Box<dyn std::error::Error + Send + Sync>> {
    // 5. Resize
    let resized =
        crate::image::process::resize_image(bytes, config.image.max_dimension).map_err(|e| {
            error!(error = %e, "Failed to resize photo");
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

    // 6. EXIF from original bytes
    let exif_data = crate::image::exif::extract_exif(bytes);
    let exif_missing = exif_data.date_taken.is_none();
    let parsed_dt = exif_data.parsed_date_time();

    // 7. Format EXIF context
    let exif_context = crate::image::exif::format_exif_context(&exif_data);

    // 8. Base64 encode resized bytes
    let base64 = crate::image::process::encode_base64(&resized);

    // 9. AI vision classification with guide
    let guide = crate::ai::guide::load_guide(&config.guide_path).await;
    let classified = ai_service
        .classify_image(
            &base64,
            caption,
            &exif_context,
            &config.openrouter_model_classify,
            guide.as_deref(),
        )
        .await;

    // 10. Generate filename (using EXIF date if available, fallback to today)
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let note_date_str = if let Some((ref d, _)) = parsed_dt {
        d.clone()
    } else if let Ok(ref c) = classified {
        c.date.clone().unwrap_or_else(|| today.clone())
    } else {
        today.clone()
    };

    let target_naive_date = chrono::NaiveDate::parse_from_str(&note_date_str, "%Y-%m-%d").ok();

    let (filename, summary) = match &classified {
        Ok(c) => {
            let slug = crate::ai::classify::slug_from_summary(&c.summary);
            (
                crate::image::process::generate_filename(&note_date_str, &slug),
                c.summary.clone(),
            )
        }
        Err(e) => {
            error!(error = %e, "Image classification failed, using fallback filename/content");
            (
                generate_fallback_filename(&note_date_str),
                caption.unwrap_or("Photo").to_string(),
            )
        }
    };

    // 11. Get daily note directory
    let note_path = match target_naive_date {
        Some(d) => vault.ensure_date(&d).await,
        None => vault.ensure_today().await,
    }
    .map_err(|e| {
        error!(error = %e, "Failed to ensure target daily note before saving photo");
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let note_dir = note_path
        .parent()
        .ok_or("Daily note has no parent directory")
        .map_err(|e| {
            error!(error = %e, "Failed to resolve daily note parent directory");
            Box::new(ImageError::SaveFailed(e.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

    // 12. Save image
    let saved_path = crate::image::process::save_image(
        &resized,
        note_dir,
        &config.image.assets_folder,
        &filename,
    )
    .await
    .map_err(|e| {
        error!(error = %e, "Failed to save photo to assets folder");
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    info!(
        path = %saved_path.display(),
        filename = %filename,
        "Saved photo to assets"
    );

    // 13. Determine section and format content
    let time_str = if let Some((_, ref t)) = parsed_dt {
        t.clone()
    } else {
        chrono::Local::now().format("%H:%M").to_string()
    };

    let (section, content) = match &classified {
        Ok(c) => {
            let section = match c.category {
                crate::ai::classify::NoteCategory::Todo => "## ✅ Todos",
                crate::ai::classify::NoteCategory::Log => "## 📋 Log",
                crate::ai::classify::NoteCategory::Note => "## 📝 Notes",
            };

            let geo_link = exif_data.format_geo_link(None);
            let formatted = format_photo_content(
                &config.image.assets_folder,
                &filename,
                Some(&c.markdown),
                None,
                Some(&time_str),
                geo_link.as_deref(),
            );
            (section, formatted)
        }
        Err(_) => {
            let geo_link = exif_data.format_geo_link(None);
            let formatted = format_photo_content(
                &config.image.assets_folder,
                &filename,
                None,
                caption,
                Some(&time_str),
                geo_link.as_deref(),
            );
            ("## 📝 Notes", formatted)
        }
    };

    vault
        .append_to_section_for_date(section, &content, target_naive_date)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to append photo entry to daily note");
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

    // 14. Update frontmatter if present
    if let Ok(c) = &classified {
        if let Some(ref frontmatter) = c.frontmatter {
            if !frontmatter.is_empty() {
                vault
                    .update_frontmatter_for_date(frontmatter, target_naive_date)
                    .await
                    .map_err(|e| {
                        error!(error = %e, "Failed to update frontmatter from photo classification");
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                    })?;
            }
        }
    }

    // 15. Notify git sync
    if let Some(notifier) = sync_notifier {
        notifier.notify();
    }

    Ok((filename, summary, exif_missing))
}

/// Sanitize any legacy or accidental `(geo:lat,lon)` links into `(https://www.google.com/maps?q=lat,lon)`.
pub fn sanitize_geo_links(text: &str) -> String {
    static GEO_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = GEO_RE.get_or_init(|| {
        regex::Regex::new(r"\(geo:([0-9.-]+),\s*([0-9.-]+)\)").expect("Invalid geo regex")
    });
    re.replace_all(text, "(https://www.google.com/maps?q=$1,$2)")
        .to_string()
}

fn format_photo_content(
    assets_folder: &str,
    filename: &str,
    markdown: Option<&str>,
    caption: Option<&str>,
    time: Option<&str>,
    geo_link: Option<&str>,
) -> String {
    let wiki_link = format!("![[{}/{}]]", assets_folder, filename);
    let time_prefix = match time {
        Some(t) => format!("- {} — ", t),
        None => "- ".to_string(),
    };

    if let Some(md) = markdown {
        let sanitized = sanitize_geo_links(md);
        let entry_text = sanitized.trim_start_matches("- ").trim();
        let geo_suffix = match geo_link {
            Some(g) if !entry_text.contains(g) && !entry_text.contains("google.com/maps") => {
                format!(" — {}", g)
            }
            _ => String::new(),
        };
        format!("{}\n{}{}{}", wiki_link, time_prefix, entry_text, geo_suffix)
    } else if let Some(cap) = caption {
        let sanitized = sanitize_geo_links(cap);
        let geo_suffix = match geo_link {
            Some(g) if !sanitized.contains(g) && !sanitized.contains("google.com/maps") => {
                format!(" — {}", g)
            }
            _ => String::new(),
        };
        format!("{}\n{}{}{}", wiki_link, time_prefix, sanitized, geo_suffix)
    } else if let Some(g) = geo_link {
        format!("{}\n{}{}", wiki_link, time_prefix, g)
    } else {
        wiki_link
    }
}

fn generate_fallback_filename(date: &str) -> String {
    let safe_date = date.replace(['/', '\\'], "-");
    let uuid_str = uuid::Uuid::new_v4().to_string();
    let uuid_suffix = crate::utils::safe_truncate(&uuid_str, 4);
    format!("{}-photo-{}.jpg", safe_date, uuid_suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_photo_content_format() {
        let filename = "2026-03-24-sunset-a1b2.jpg";
        let markdown = "Beautiful sunset over the harbor at golden hour.";

        let content = format_photo_content(
            "assets",
            filename,
            Some(markdown),
            None,
            Some("18:30"),
            None,
        );

        assert_eq!(
            content,
            "![[assets/2026-03-24-sunset-a1b2.jpg]]\n- 18:30 — Beautiful sunset over the harbor at golden hour."
        );
    }

    #[test]
    fn test_photo_content_format_with_geo() {
        let filename = "2026-03-24-pizza.jpg";
        let markdown = "Heerlijke pizza margherita";
        let geo = "[Pizzeria](https://www.google.com/maps?q=41.902800,12.496400)";

        let content = format_photo_content(
            "assets",
            filename,
            Some(markdown),
            None,
            Some("19:45"),
            Some(geo),
        );

        assert_eq!(
            content,
            "![[assets/2026-03-24-pizza.jpg]]\n- 19:45 — Heerlijke pizza margherita — [Pizzeria](https://www.google.com/maps?q=41.902800,12.496400)"
        );
        // Ensure no space between lat and lon
        assert!(!content.contains("maps?q=41.902800, "));
    }

    #[test]
    fn test_sanitize_geo_links() {
        let input = "Bezoek aan [KMSKA, Antwerpen](geo:51.208505,4.395479) was fantastisch.";
        let output = sanitize_geo_links(input);
        assert_eq!(
            output,
            "Bezoek aan [KMSKA, Antwerpen](https://www.google.com/maps?q=51.208505,4.395479) was fantastisch."
        );
    }

    #[test]
    fn test_photo_content_sanitizes_embedded_geo_and_avoids_duplicate() {
        let filename = "2026-08-08-kmska.jpg";
        let markdown = "Bezoek aan het KMSKA [KMSKA, Antwerpen](geo:51.208505,4.395479)";
        let geo = "[KMSKA, Antwerpen](https://www.google.com/maps?q=51.208505,4.395479)";

        let content = format_photo_content(
            "assets",
            filename,
            Some(markdown),
            None,
            Some("12:11"),
            Some(geo),
        );

        assert_eq!(
            content,
            "![[assets/2026-08-08-kmska.jpg]]\n- 12:11 — Bezoek aan het KMSKA [KMSKA, Antwerpen](https://www.google.com/maps?q=51.208505,4.395479)"
        );
        assert!(!content.contains("(geo:"));
    }

    #[test]
    fn test_photo_fallback_filename() {
        let date = "2026-03-24";
        let filename = generate_fallback_filename(date);

        assert!(
            filename.starts_with("2026-03-24-photo-"),
            "fallback filename should use 'photo' slug"
        );
        assert!(filename.ends_with(".jpg"));

        let suffix = filename
            .trim_start_matches("2026-03-24-photo-")
            .trim_end_matches(".jpg");

        assert_eq!(suffix.len(), 4, "uuid suffix should be 4 chars");
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "uuid suffix should be hexadecimal"
        );
    }

    #[test]
    fn test_generate_fallback_filename_with_slashes_in_date() {
        let filename = generate_fallback_filename("2026/03/24");
        assert!(
            !filename.contains('/'),
            "filename should not contain forward slashes"
        );
        assert!(
            !filename.contains('\\'),
            "filename should not contain backslashes"
        );
        assert!(
            filename.starts_with("2026-03-24-photo-"),
            "slashes should be replaced with dashes"
        );
    }

    #[test]
    fn test_photo_filename_generation_uses_today() {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let filename = generate_fallback_filename(&today);
        assert!(filename.starts_with(&today));
    }

    #[test]
    fn test_is_image_document() {
        let mut doc = teloxide::types::Document {
            file: teloxide::types::FileMeta {
                id: "test".into(),
                unique_id: "u_test".into(),
                size: 1234,
            },
            thumbnail: None,
            file_name: Some("photo.JPG".to_string()),
            mime_type: None,
        };

        assert!(is_image_document(&doc));

        doc.file_name = Some("document.pdf".to_string());
        assert!(!is_image_document(&doc));

        doc.mime_type = "image/png".parse().ok();
        assert!(is_image_document(&doc));

        doc.mime_type = "image/webp".parse().ok();
        assert!(is_image_document(&doc));
    }
}
