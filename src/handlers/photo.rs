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
        Ok((_filename, summary)) => {
            // 16. Send confirmation
            bot.send_message(msg.chat.id, format!("📸 Photo saved — {}", summary))
                .await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to process photo entry");
            bot.send_message(msg.chat.id, format!("❌ Failed to save photo: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Process a photo entry: resize → EXIF → classify via Vision AI → save to vault → write to note.
/// Returns the saved filename and classification summary.
pub async fn process_photo_entry(
    bytes: &[u8],
    caption: Option<&str>,
    config: &Config,
    ai_service: &AiService,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    // 5. Resize
    let resized =
        crate::image::process::resize_image(bytes, config.image.max_dimension).map_err(|e| {
            error!(error = %e, "Failed to resize photo");
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

    // 6. EXIF from original bytes
    let exif_data = crate::image::exif::extract_exif(bytes);

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

    // 10. Generate filename (with fallback on AI failure)
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let (filename, summary) = match &classified {
        Ok(c) => {
            let slug = crate::ai::classify::slug_from_summary(&c.summary);
            (
                crate::image::process::generate_filename(&today, &slug),
                c.summary.clone(),
            )
        }
        Err(e) => {
            error!(error = %e, "Image classification failed, using fallback filename/content");
            (
                generate_fallback_filename(&today),
                caption.unwrap_or("Photo").to_string(),
            )
        }
    };

    // 11. Get daily note directory
    let note_path = vault.ensure_today().await.map_err(|e| {
        error!(error = %e, "Failed to ensure today's daily note before saving photo");
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

    // 13. Write to daily note
    let content = match &classified {
        Ok(c) => format_photo_content(
            &config.image.assets_folder,
            &filename,
            Some(&c.markdown),
            None,
        ),
        Err(_) => format_photo_content(
            &config.image.assets_folder,
            &filename,
            None,
            caption,
        ),
    };

    vault
        .append_to_section("## 📝 Notes", &content)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to append photo entry to daily note");
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

    // 14. Update frontmatter if present
    if let Ok(c) = &classified {
        if let Some(ref frontmatter) = c.frontmatter {
            if !frontmatter.is_empty() {
                vault.update_frontmatter(frontmatter).await.map_err(|e| {
                    error!(error = %e, "Failed to update frontmatter from photo classification");
                    Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                })?;
            }
        }
    }

    // 15. Notify git sync
    if let Some(ref notifier) = sync_notifier {
        notifier.notify();
    }

    Ok((filename, summary))
}

fn format_photo_content(
    assets_folder: &str,
    filename: &str,
    markdown: Option<&str>,
    caption: Option<&str>,
) -> String {
    let wiki_link = format!("![[{}/{}]]", assets_folder, filename);
    if let Some(md) = markdown {
        format!("{}\n{}", wiki_link, md)
    } else if let Some(cap) = caption {
        format!("{}\n{}", wiki_link, cap)
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

        let content = format_photo_content("assets", filename, Some(markdown), None);

        assert_eq!(
            content,
            "![[assets/2026-03-24-sunset-a1b2.jpg]]\nBeautiful sunset over the harbor at golden hour."
        );
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
}
