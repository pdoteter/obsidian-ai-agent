use std::path::{Path, PathBuf};
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{error, info};

use crate::config::Config;
use crate::git::chat_tracker::ChatIdTracker;
use crate::git::debounce::SyncNotifier;
use crate::vault::daily_note::DailyNoteManager;

/// Process an unrecognized or generic file: save file bytes directly to the vault assets folder
/// and create a log entry in the Obsidian Daily Note under `## 📋 Log`.
/// Returns (saved_filename, display_title).
pub async fn process_generic_file_entry(
    bytes: &[u8],
    original_filename: Option<&str>,
    caption: Option<&str>,
    config: &Config,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    // Ensure today's daily note exists
    let note_path = vault.ensure_today().await.map_err(|e| {
        error!(error = %e, "Failed to ensure target daily note before saving file asset");
        e
    })?;

    let note_dir = note_path
        .parent()
        .ok_or("Daily note has no parent directory")?;

    // Determine filename stem and extension
    let orig_name = original_filename.unwrap_or("file.bin");
    let orig_path = Path::new(orig_name);

    let stem = orig_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");

    let ext = orig_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let saved_filename = generate_file_asset_name(&today, stem, ext);
    let display_title = orig_name.to_string();

    // 1. Save file bytes to assets folder
    save_file_asset(
        bytes,
        note_dir,
        &config.image.assets_folder,
        &saved_filename,
    )
    .await
    .map_err(|e| {
        error!(error = %e, filename = %saved_filename, "Failed to save file to assets folder");
        e
    })?;

    info!(
        saved_filename = %saved_filename,
        original_name = %display_title,
        size_bytes = bytes.len(),
        "Saved unrecognized file to assets folder"
    );

    // 2. Format the log entry under '## 📋 Log'
    let time = chrono::Local::now().format("%H:%M").to_string();
    let log_content = format_file_log_entry(
        &time,
        &display_title,
        &config.image.assets_folder,
        &saved_filename,
        caption,
    );

    // 3. Append to Vault Daily Note under '## 📋 Log'
    vault
        .append_to_section_for_date("## 📋 Log", &log_content, None)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to append generic file entry to daily note");
            e
        })?;

    // 4. Notify Git sync
    if let Some(notifier) = sync_notifier {
        notifier.notify();
    }

    Ok((saved_filename, display_title))
}

/// Helper to format the daily note log entry for an unrecognized file
pub fn format_file_log_entry(
    time: &str,
    title: &str,
    assets_folder: &str,
    saved_filename: &str,
    caption: Option<&str>,
) -> String {
    let mut entry = format!(
        "- {} — 📁 **File: {}** (⚠️ Unrecognized file type: stored as-is)\n  - **Attachment**: [[{}/{}]]",
        time, title, assets_folder, saved_filename
    );

    if let Some(cap) = caption {
        let trimmed = cap.trim();
        if !trimmed.is_empty() {
            entry.push_str(&format!("\n  - **Caption**: {}", trimmed));
        }
    }

    entry
}

/// Helper to generate unique date-slug filenames preserving the extension
pub fn generate_file_asset_name(date: &str, stem: &str, ext: &str) -> String {
    let safe_date = date.replace(['/', '\\'], "-");
    let sanitized = crate::image::process::sanitize_slug(stem);
    let slug = if sanitized.is_empty() {
        "file".to_string()
    } else {
        sanitized
    };
    let uuid_str = uuid::Uuid::new_v4().to_string();
    let uuid_suffix = crate::utils::safe_truncate(&uuid_str, 4);

    if ext.is_empty() {
        format!("{}-{}-{}", safe_date, slug, uuid_suffix)
    } else {
        format!("{}-{}-{}.{}", safe_date, slug, uuid_suffix, ext)
    }
}

/// Save file bytes directly to the daily note assets folder
pub async fn save_file_asset(
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

/// Handle incoming generic Telegram document messages (files not recognized as PDF or image).
pub async fn handle_generic_document_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Auth check
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(
                user_id = user.id.0,
                "Unauthorized user, ignoring generic document"
            );
            return Ok(());
        }
    }

    chat_tracker.set(msg.chat.id).await;

    let doc = msg.document().ok_or("No document in message")?;
    let original_name = doc
        .file_name
        .clone()
        .unwrap_or_else(|| "attachment.bin".to_string());

    info!(
        filename = %original_name,
        file_size = doc.file.size,
        "Processing generic Telegram document"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadDocument)
        .await?;

    let file = bot.get_file(&doc.file.id).await.map_err(|e| {
        error!(error = %e, "Failed to fetch Telegram file metadata for document");
        e
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download document bytes from Telegram");
            e
        })?;

    let caption = msg.caption().map(|s| s.to_string());

    let process_result = process_generic_file_entry(
        &bytes,
        Some(&original_name),
        caption.as_deref(),
        &config,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((saved_filename, display_title)) => {
            let response = format!(
                "⚠️ **File Stored As-Is**\n\nThis file type is not recognized for AI processing. It has been stored directly in your assets folder and logged in today's Daily Note.\n\n- **File**: `{}`\n- **Original**: `{}`",
                saved_filename, display_title
            );
            bot.send_message(msg.chat.id, response).await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to store generic document");
            bot.send_message(msg.chat.id, format!("❌ Failed to store file: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Handle incoming Telegram audio files (music or audio files sent as audio, not voice note).
pub async fn handle_audio_file_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring audio");
            return Ok(());
        }
    }

    chat_tracker.set(msg.chat.id).await;

    let audio = msg.audio().ok_or("No audio in message")?;
    let original_name = audio
        .file_name
        .clone()
        .unwrap_or_else(|| "audio.mp3".to_string());

    info!(
        filename = %original_name,
        file_size = audio.file.size,
        "Processing Telegram audio message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadDocument)
        .await?;

    let file = bot.get_file(&audio.file.id).await.map_err(|e| {
        error!(error = %e, "Failed to fetch Telegram audio file metadata");
        e
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download audio file bytes from Telegram");
            e
        })?;

    let caption = msg.caption().map(|s| s.to_string());

    let process_result = process_generic_file_entry(
        &bytes,
        Some(&original_name),
        caption.as_deref(),
        &config,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((saved_filename, display_title)) => {
            let response = format!(
                "⚠️ **Audio File Stored As-Is**\n\nAudio file stored directly in your assets folder and logged in today's Daily Note.\n\n- **File**: `{}`\n- **Original**: `{}`",
                saved_filename, display_title
            );
            bot.send_message(msg.chat.id, response).await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to store audio file");
            bot.send_message(msg.chat.id, format!("❌ Failed to store audio file: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Handle incoming Telegram video messages.
pub async fn handle_video_file_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring video");
            return Ok(());
        }
    }

    chat_tracker.set(msg.chat.id).await;

    let video = msg.video().ok_or("No video in message")?;
    let original_name = video
        .file_name
        .clone()
        .unwrap_or_else(|| "video.mp4".to_string());

    info!(
        filename = %original_name,
        file_size = video.file.size,
        "Processing Telegram video message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadVideo)
        .await?;

    let file = bot.get_file(&video.file.id).await.map_err(|e| {
        error!(error = %e, "Failed to fetch Telegram video metadata");
        e
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download video bytes from Telegram");
            e
        })?;

    let caption = msg.caption().map(|s| s.to_string());

    let process_result = process_generic_file_entry(
        &bytes,
        Some(&original_name),
        caption.as_deref(),
        &config,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((saved_filename, display_title)) => {
            let response = format!(
                "⚠️ **Video File Stored As-Is**\n\nVideo stored directly in your assets folder and logged in today's Daily Note.\n\n- **File**: `{}`\n- **Original**: `{}`",
                saved_filename, display_title
            );
            bot.send_message(msg.chat.id, response).await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to store video file");
            bot.send_message(msg.chat.id, format!("❌ Failed to store video file: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Handle incoming Telegram animation messages (GIFs / animations).
pub async fn handle_animation_file_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring animation");
            return Ok(());
        }
    }

    chat_tracker.set(msg.chat.id).await;

    let animation = msg.animation().ok_or("No animation in message")?;
    let original_name = animation
        .file_name
        .clone()
        .unwrap_or_else(|| "animation.gif".to_string());

    info!(
        filename = %original_name,
        file_size = animation.file.size,
        "Processing Telegram animation message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadVideo)
        .await?;

    let file = bot.get_file(&animation.file.id).await.map_err(|e| {
        error!(error = %e, "Failed to fetch Telegram animation metadata");
        e
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download animation bytes from Telegram");
            e
        })?;

    let caption = msg.caption().map(|s| s.to_string());

    let process_result = process_generic_file_entry(
        &bytes,
        Some(&original_name),
        caption.as_deref(),
        &config,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((saved_filename, display_title)) => {
            let response = format!(
                "⚠️ **Animation Stored As-Is**\n\nAnimation stored directly in your assets folder and logged in today's Daily Note.\n\n- **File**: `{}`\n- **Original**: `{}`",
                saved_filename, display_title
            );
            bot.send_message(msg.chat.id, response).await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to store animation");
            bot.send_message(msg.chat.id, format!("❌ Failed to store animation: {}", e))
                .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_file_asset_name() {
        let date = "2026-08-29";
        let stem = "Quarterly Report Final";
        let ext = "docx";

        let name = generate_file_asset_name(date, stem, ext);
        assert!(name.starts_with("2026-08-29-quarterly-report-final-"));
        assert!(name.ends_with(".docx"));

        let suffix = name
            .trim_start_matches("2026-08-29-quarterly-report-final-")
            .trim_end_matches(".docx");
        assert_eq!(suffix.len(), 4);
    }

    #[test]
    fn test_generate_file_asset_name_empty_stem() {
        let date = "2026-08-29";
        let stem = "---";
        let ext = "zip";

        let name = generate_file_asset_name(date, stem, ext);
        assert!(name.starts_with("2026-08-29-file-"));
        assert!(name.ends_with(".zip"));
    }

    #[test]
    fn test_generate_file_asset_name_no_ext() {
        let date = "2026-08-29";
        let stem = "data_file";
        let ext = "";

        let name = generate_file_asset_name(date, stem, ext);
        assert!(name.starts_with("2026-08-29-data-file-"));
        assert!(!name.contains('.'));
    }

    #[test]
    fn test_format_file_log_entry_without_caption() {
        let entry = format_file_log_entry(
            "14:30",
            "spreadsheet.xlsx",
            "assets",
            "2026-08-29-spreadsheet-a1b2.xlsx",
            None,
        );

        assert_eq!(
            entry,
            "- 14:30 — 📁 **File: spreadsheet.xlsx** (⚠️ Unrecognized file type: stored as-is)\n  - **Attachment**: [[assets/2026-08-29-spreadsheet-a1b2.xlsx]]"
        );
    }

    #[test]
    fn test_format_file_log_entry_with_caption() {
        let entry = format_file_log_entry(
            "14:30",
            "archive.tar.gz",
            "assets",
            "2026-08-29-archive-c3d4.tar.gz",
            Some("Backup of project files"),
        );

        assert_eq!(
            entry,
            "- 14:30 — 📁 **File: archive.tar.gz** (⚠️ Unrecognized file type: stored as-is)\n  - **Attachment**: [[assets/2026-08-29-archive-c3d4.tar.gz]]\n  - **Caption**: Backup of project files"
        );
    }

    #[tokio::test]
    async fn test_process_generic_file_entry_saves_asset_and_logs() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let vault_path = temp_dir.path().to_path_buf();

        let vault = Arc::new(
            DailyNoteManager::new(vault_path.clone(), "%Y-%m-%d".to_string(), None, None).await,
        );

        let mut config = Config::default();
        config.vault_path = vault_path.clone();
        config.image.assets_folder = "assets".to_string();

        let file_bytes = b"Hello, this is raw binary data for a custom format file.";
        let original_name = "data_export.csv";
        let caption = "Quarterly CSV export";

        let (saved_filename, title) = process_generic_file_entry(
            file_bytes,
            Some(original_name),
            Some(caption),
            &config,
            &vault,
            None,
        )
        .await
        .expect("process generic file");

        assert_eq!(title, "data_export.csv");
        assert!(saved_filename.ends_with(".csv"));

        let asset_file_path = vault_path.join("assets").join(&saved_filename);
        assert!(
            asset_file_path.exists(),
            "Asset file should be written to assets folder"
        );

        let written_bytes = tokio::fs::read(&asset_file_path).await.expect("read asset");
        assert_eq!(written_bytes, file_bytes);

        let note_path = vault.ensure_today().await.expect("ensure note");
        let note_content = tokio::fs::read_to_string(&note_path)
            .await
            .expect("read note");

        assert!(note_content.contains("## 📋 Log"));
        assert!(note_content
            .contains("📁 **File: data_export.csv** (⚠️ Unrecognized file type: stored as-is)"));
        assert!(note_content.contains(&format!("[[assets/{}]]", saved_filename)));
        assert!(note_content.contains("Quarterly CSV export"));
    }
}
