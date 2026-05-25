use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatAction;

use tracing::{debug, error, info};

use crate::ai::classify::{ClassifiedNote, NoteCategory};
use crate::ai::AiService;
use crate::audio::download;
use crate::config::Config;
use crate::git::chat_tracker::ChatIdTracker;
use crate::git::debounce::SyncNotifier;
use crate::vault::daily_note::DailyNoteManager;
use crate::vault::writer;

/// Handle incoming voice messages: download → transcribe → classify → write to vault
#[allow(clippy::too_many_arguments)]
pub async fn handle_voice_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Extract voice from the message
    let voice = match msg.voice() {
        Some(v) => v.clone(),
        None => return Ok(()),
    };

    // Check user authorization
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(
                user_id = user.id.0,
                "Unauthorized user, ignoring voice message"
            );
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    chat_tracker.set(msg.chat.id).await;

    info!(
        duration_secs = %voice.duration,
        file_size = voice.file.size,
        "Processing voice message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    // Step 1: Download voice to memory (Ogg Opus bytes)
    let audio_bytes = download::download_voice_to_memory(&bot, &voice)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download voice message");
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

    // Step 2: Transcribe and process via process_voice_entry
    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    let process_result = process_voice_entry(
        &audio_bytes,
        &config,
        &ai_service,
        &vault,
        sync_notifier.as_ref(),
    )
    .await;

    match process_result {
        Ok((transcript, classified)) => {
            // Step 5: Send confirmation
            bot.send_message(
                msg.chat.id,
                format!(
                    "✅ Voice → {} saved as {}\n\nTranscript: \"{}\"",
                    classified.category,
                    classified.summary,
                    truncate(&transcript, 200),
                ),
            )
            .await?;
        }
        Err(e) => {
            error!(error = %e, "Failed to process voice entry");
            bot.send_message(msg.chat.id, format!("❌ Voice message processing failed: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Process a voice entry: transcribe → classify → format → write to vault → update frontmatter → notify sync.
/// Returns the transcript text and the ClassifiedNote.
pub async fn process_voice_entry(
    audio_bytes: &[u8],
    config: &Config,
    ai_service: &AiService,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
) -> Result<(String, ClassifiedNote), Box<dyn std::error::Error + Send + Sync>> {
    let transcript = ai_service.transcribe(audio_bytes).await.map_err(|e| {
        error!(error = %e, "Voice transcription failed");
        e
    })?;

    info!(
        transcript_length = transcript.len(),
        "Voice transcription complete"
    );
    debug!(transcript = %transcript, "Full transcript");

    let guide = crate::ai::guide::load_guide(&config.guide_path).await;
    let classified = match ai_service
        .classify_text(
            &transcript,
            &config.openrouter_model_classify,
            guide.as_deref(),
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Voice classification failed, saving as raw log");
            let (section, content) = writer::format_raw_entry(&transcript);
            vault
                .append_to_section(section, &content)
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            if let Some(n) = sync_notifier {
                n.notify();
            }

            return Ok((
                transcript.clone(),
                ClassifiedNote {
                    category: NoteCategory::Log,
                    markdown: format!("- {}", transcript),
                    tags: Vec::new(),
                    summary: transcript,
                    frontmatter: None,
                },
            ));
        }
    };

    // Format and write to vault
    let (section, content) = writer::format_for_daily_note(&classified);
    vault
        .append_to_section(section, &content)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    // Update frontmatter if AI provided any
    if let Some(ref frontmatter) = classified.frontmatter {
        if !frontmatter.is_empty() {
            vault
                .update_frontmatter(frontmatter)
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        }
    }

    // Notify git sync
    if let Some(ref notifier) = sync_notifier {
        notifier.notify();
    }

    Ok((transcript, classified))
}

fn truncate(s: &str, max_len: usize) -> &str {
    crate::utils::safe_truncate(s, max_len)
}
