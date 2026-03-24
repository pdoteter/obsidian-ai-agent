use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatAction;

use tracing::{debug, error, info};

use crate::ai::client::OpenRouterClient;
use crate::ai::transcribe::WhisperClient;
use crate::audio::download;
use crate::config::Config;
use crate::git::debounce::SyncNotifier;
use crate::vault::daily_note::DailyNoteManager;
use crate::vault::writer;

/// Handle incoming voice messages: download → transcribe (Whisper) → classify → write to vault
pub async fn handle_voice_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_client: Arc<OpenRouterClient>,
    whisper_client: Arc<WhisperClient>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Extract voice from the message
    let voice = match msg.voice() {
        Some(v) => v.clone(),
        None => return Ok(()),
    };

    // Check user authorization
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring voice message");
            return Ok(());
        }
    }

    info!(
        duration_secs = %voice.duration,
        file_size = voice.file.size,
        "Processing voice message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::Typing).await?;

    // Step 1: Download voice to memory (Ogg Opus bytes)
    let audio_bytes = download::download_voice_to_memory(&bot, &voice).await.map_err(|e| {
        error!(error = %e, "Failed to download voice message");
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // Step 2: Transcribe via OpenAI Whisper (accepts .oga natively, no ffmpeg needed)
    bot.send_chat_action(msg.chat.id, ChatAction::Typing).await?;

    let transcript = match whisper_client.transcribe(&audio_bytes).await {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "Transcription failed");
            bot.send_message(
                msg.chat.id,
                format!("❌ Transcription failed: {}", e),
            )
            .await?;
            return Ok(());
        }
    };

    info!(transcript_length = transcript.len(), "Transcription complete");
    debug!(transcript = %transcript, "Full transcript");

    // Step 3: Classify the transcribed text
    let guide = crate::ai::guide::load_guide(&config.guide_path);
    let classified = match ai_client
        .classify_text(&transcript, &config.openrouter_model_classify, guide.as_deref())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Classification failed, saving as raw log");
            let (section, content) = writer::format_raw_entry(&transcript);
            vault.append_to_section(section, &content).await.map_err(|e| {
                Box::new(e) as Box<dyn std::error::Error + Send + Sync>
            })?;
            bot.send_message(
                msg.chat.id,
                format!(
                    "📝 Transcribed & saved as raw log (classification failed)\n\n\"{}\"",
                    truncate(&transcript, 200),
                ),
            )
            .await?;
            sync_notifier.as_ref().map(|n| n.notify());
            return Ok(());
        }
    };

    // Step 4: Write to vault
    let (section, content) = writer::format_for_daily_note(&classified);
    vault.append_to_section(section, &content).await.map_err(|e| {
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // Notify git sync
    if let Some(ref notifier) = sync_notifier {
        notifier.notify();
    }

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

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}
