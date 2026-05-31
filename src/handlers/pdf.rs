use std::path::{Path, PathBuf};
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{error, info};

use crate::ai::classify::ClassifiedNote;
use crate::ai::AiService;
use crate::config::Config;
use crate::git::chat_tracker::ChatIdTracker;
use crate::git::debounce::SyncNotifier;
use crate::vault::daily_note::DailyNoteManager;

/// Handle incoming PDF document messages from Telegram: download → transcribe/OCR via Gemini → save PDF & transcript → log in vault
#[allow(clippy::too_many_arguments)]
pub async fn handle_pdf_message(
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
                "Unauthorized user, ignoring PDF document"
            );
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    chat_tracker.set(msg.chat.id).await;

    // 2. Extract document payload
    let doc = msg.document().ok_or("No document in message")?;
    let original_name = doc
        .file_name
        .clone()
        .unwrap_or_else(|| "document.pdf".to_string());

    info!(
        filename = %original_name,
        file_size = doc.file.size,
        "Processing Telegram PDF message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadDocument)
        .await?;

    // 3. Download from Telegram
    let file = bot.get_file(&doc.file.id).await.map_err(|e| {
        error!(error = %e, "Failed to fetch Telegram file metadata for PDF");
        e
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download PDF bytes from Telegram");
            e
        })?;

    // 4. Process the entry
    let caption = msg.caption().map(|s| s.to_string());

    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    let process_result = process_pdf_entry(
        &bytes,
        Some(&original_name),
        caption.as_deref(),
        &config,
        &ai_service,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((pdf_filename, transcript_filename, title, _summary, gemini_success)) => {
            if gemini_success {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "📄 **PDF Transcribed & Saved!**\n\n**Title**: {}\n**PDF**: `{}`\n**Transcript**: `{}`\n\nLogged in Daily Note.",
                        title, pdf_filename, transcript_filename
                    ),
                )
                .await?;
            } else {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "⚠️ **PDF Saved!**\n\n*Transcription skipped*: Gemini client not configured or transcription failed.\n**Saved as**: `{}`\n\nLogged original reference only.",
                        pdf_filename
                    ),
                )
                .await?;
            }
        }
        Err(e) => {
            error!(error = %e, "Failed to process PDF entry");
            bot.send_message(
                msg.chat.id,
                format!("❌ Failed to save and process PDF: {}", e),
            )
            .await?;
        }
    }

    Ok(())
}

/// Process a PDF entry: save original PDF → call Gemini Multimodal OCR → save transcript md → write log to vault daily note.
/// Returns (pdf_filename, transcript_filename, document_title, summary, gemini_success)
pub async fn process_pdf_entry(
    bytes: &[u8],
    original_filename: Option<&str>,
    user_prompt: Option<&str>,
    config: &Config,
    ai_service: &AiService,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
) -> Result<(String, String, String, String, bool), Box<dyn std::error::Error + Send + Sync>> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    // Ensure daily note exists and resolve its parent directory
    let note_path = vault.ensure_today().await.map_err(|e| {
        error!(error = %e, "Failed to ensure today's daily note before saving PDF");
        e
    })?;

    let note_dir = note_path
        .parent()
        .ok_or("Daily note has no parent directory")?;

    // Try Gemini Multimodal transcription first
    let gemini_model = &config.openrouter_model_classify; // Gemini or configured model
    let mut classified_res: Option<ClassifiedNote> = None;
    let mut gemini_success = false;

    // We can infer if Gemini is being used or try/catch UnsupportedCapability
    match ai_service
        .transcribe_pdf(bytes, user_prompt, gemini_model)
        .await
    {
        Ok(note) => {
            classified_res = Some(note);
            gemini_success = true;
        }
        Err(e) => {
            error!(error = %e, "Gemini PDF transcription failed, falling back to original-only log");
        }
    }

    // Determine filenames, titles and summary
    let (pdf_filename, transcript_filename, title, summary, content_markdown) = if gemini_success {
        let note = classified_res.as_ref().unwrap();

        // Clean original filename to derive the slug
        let orig_clean = original_filename
            .unwrap_or("document")
            .trim_end_matches(".pdf");
        let slug = crate::image::process::sanitize_slug(orig_clean);
        let slug_final = if slug.is_empty() {
            crate::ai::classify::slug_from_summary(&note.summary)
        } else {
            slug
        };

        let pdf_name = generate_filename(&today, &slug_final, "pdf");
        let trans_name = generate_filename(&today, &slug_final, "md");

        (
            pdf_name,
            trans_name,
            original_filename.unwrap_or("PDF Document").to_string(),
            note.summary.clone(),
            note.markdown.clone(),
        )
    } else {
        // Fallback filenames
        let orig_clean = original_filename
            .unwrap_or("document")
            .trim_end_matches(".pdf");
        let slug = crate::image::process::sanitize_slug(orig_clean);
        let slug_final = if slug.is_empty() {
            "document".to_string()
        } else {
            slug
        };

        let pdf_name = generate_filename(&today, &slug_final, "pdf");
        (
            pdf_name,
            String::new(),
            original_filename.unwrap_or("PDF Document").to_string(),
            String::new(),
            String::new(),
        )
    };

    // 1. Save original PDF bytes
    save_file_asset(bytes, note_dir, &config.image.assets_folder, &pdf_filename)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to save PDF to assets folder");
            e
        })?;

    // 2. Save transcript md (if Gemini succeeded)
    if gemini_success {
        save_file_asset(
            content_markdown.as_bytes(),
            note_dir,
            &config.image.assets_folder,
            &transcript_filename,
        )
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to save transcript to assets folder");
            e
        })?;
    }

    // 3. Format the log entry (PDF entries are always logs under '## 📋 Log')
    let time = chrono::Local::now().format("%H:%M").to_string();
    let log_content = if gemini_success {
        let note = classified_res.as_ref().unwrap();
        let tags_str = if note.tags.is_empty() {
            String::new()
        } else {
            let tags: Vec<String> = note.tags.iter().map(|t| format!("#{}", t)).collect();
            format!(" {}", tags.join(" "))
        };

        format!(
            "- {} — 📄 **Document: {}**\n  - **Original PDF**: [[{}/{}]]\n  - **Transcription**: [[{}/{}]]\n  - **Summary**:\n    > {}\n  {}",
            time,
            title,
            config.image.assets_folder,
            pdf_filename,
            config.image.assets_folder,
            transcript_filename,
            summary.replace("\n", "\n    "),
            tags_str
        )
    } else {
        format!(
            "- {} — 📄 **Document: {}** (⚠️ Transcription unavailable: Gemini not configured or transcription failed)\n  - **Original PDF**: [[{}/{}]]",
            time,
            title,
            config.image.assets_folder,
            pdf_filename
        )
    };

    // 4. Append to Vault Log section
    vault
        .append_to_section("## 📋 Log", &log_content)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to append PDF entry to daily note");
            e
        })?;

    // 5. Update frontmatter if present (and Gemini succeeded)
    if let Some(ref note) = classified_res {
        if let Some(ref frontmatter) = note.frontmatter {
            if !frontmatter.is_empty() {
                let _ = vault.update_frontmatter(frontmatter).await.map_err(|e| {
                    error!(error = %e, "Failed to update frontmatter from PDF transcription");
                    e
                });
            }
        }
    }

    // 6. Notify Git sync
    if let Some(notifier) = sync_notifier {
        notifier.notify();
    }

    Ok((
        pdf_filename,
        transcript_filename,
        title,
        summary,
        gemini_success,
    ))
}

/// Helper to generate unique date-slug filenames
fn generate_filename(date: &str, slug: &str, ext: &str) -> String {
    let safe_date = date.replace(['/', '\\'], "-");
    let sanitized = crate::image::process::sanitize_slug(slug);
    let uuid_str = uuid::Uuid::new_v4().to_string();
    let uuid_suffix = crate::utils::safe_truncate(&uuid_str, 4);
    format!("{}-{}-{}.{}", safe_date, sanitized, uuid_suffix, ext)
}

/// Save file bytes directly to the daily note assets folder
async fn save_file_asset(
    bytes: &[u8],
    daily_note_dir: &Path,
    assets_folder: &str,
    filename: &str,
) -> Result<PathBuf, std::io::Error> {
    let assets_dir = daily_note_dir.join(assets_folder);
    tokio::fs::create_dir_all(&assets_dir).await?;
    let full_path = assets_dir.join(filename);
    tokio::fs::write(&full_path, bytes).await?;
    Ok(full_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdf_filename_generation() {
        let date = "2026-05-31";
        let slug = "invoice-receipt";
        let name = generate_filename(date, slug, "pdf");

        assert!(name.starts_with("2026-05-31-"));
        assert!(name.contains("invoice-receipt"));
        assert!(name.ends_with(".pdf"));

        let suffix = name
            .trim_start_matches("2026-05-31-invoice-receipt-")
            .trim_end_matches(".pdf");
        assert_eq!(suffix.len(), 4);
    }
}
