use std::sync::Arc;
use teloxide::prelude::*;

use tracing::{error, info};

use crate::ai::client::OpenRouterClient;
use crate::audio::{convert, download};
use crate::config::Config;
use crate::git::debounce::SyncNotifier;
use crate::vault::daily_note::DailyNoteManager;
use crate::vault::writer;

/// Handle incoming voice messages: download → convert → transcribe → classify → write to vault
pub async fn handle_voice_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_client: Arc<OpenRouterClient>,
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

    bot.send_message(msg.chat.id, "🎙️ Processing your voice message...")
        .await?;

    // Step 1: Download the voice file
    let (oga_path, _tmp_dir) = download::download_voice(&bot, &voice).await.map_err(|e| {
        error!(error = %e, "Failed to download voice message");
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // Step 2: Convert .oga to .wav
    let wav_path = convert::convert_oga_to_wav(&oga_path).await.map_err(|e| {
        error!(error = %e, "Failed to convert audio");
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // Step 3: Transcribe the audio
    bot.send_message(msg.chat.id, "🔊 Transcribing audio...")
        .await?;

    let transcript = match ai_client
        .transcribe_audio(&wav_path, &config.openrouter_model_transcribe)
        .await
    {
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

    // Step 4: Classify the transcribed text
    let classified = match ai_client
        .classify_text(&transcript, &config.openrouter_model_classify)
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

    // Step 5: Write to vault
    let (section, content) = writer::format_for_daily_note(&classified);
    vault.append_to_section(section, &content).await.map_err(|e| {
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    // Notify git sync
    if let Some(ref notifier) = sync_notifier {
        notifier.notify();
    }

    // Step 6: Send confirmation
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
