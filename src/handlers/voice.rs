use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatAction;

use tracing::{debug, error, info};

use crate::ai::transcribe::WhisperClient;
use crate::audio::download;
use crate::error::AppResult;
use crate::handlers::HandlerContext;
use crate::vault::writer;

/// Handle incoming voice messages: download -> transcribe (Whisper) -> classify -> write to vault
pub async fn handle_voice_message(
    bot: Bot,
    msg: Message,
    ctx: HandlerContext,
    whisper_client: Arc<WhisperClient>,
) -> AppResult<()> {
    // Extract voice from the message
    let voice = match msg.voice() {
        Some(v) => v.clone(),
        None => return Ok(()),
    };

    // Check user authorization
    if let Some(user) = msg.from.as_ref() {
        if !ctx.config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring voice message");
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    ctx.chat_tracker.set(msg.chat.id);

    info!(
        duration_secs = %voice.duration,
        file_size = voice.file.size,
        "Processing voice message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::Typing).await?;

    // Step 1: Download voice to memory (Ogg Opus bytes)
    let audio_bytes = download::download_voice_to_memory(&bot, &voice).await
        .inspect_err(|e| error!(error = %e, "Failed to download voice message"))?;

    // Step 2: Transcribe via OpenAI Whisper (accepts .oga natively, no ffmpeg needed)
    bot.send_chat_action(msg.chat.id, ChatAction::Typing).await?;

    let transcript = match whisper_client.transcribe(&audio_bytes).await {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "Transcription failed");
            bot.send_message(
                msg.chat.id,
                format!("Transcription failed: {}", e),
            )
            .await?;
            return Ok(());
        }
    };

    info!(transcript_length = transcript.len(), "Transcription complete");
    debug!(transcript = %transcript, "Full transcript");

    // Step 3: Classify the transcribed text
    let guide = crate::ai::guide::load_guide(&ctx.config.guide_path);
    let classified = match ctx.ai_client
        .classify_text(&transcript, &ctx.config.openrouter_model_classify, guide.as_deref())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Classification failed, saving as raw log");
            let (section, content) = writer::format_raw_entry(&transcript);
            ctx.vault.append_to_section(section, &content).await?;
            bot.send_message(
                msg.chat.id,
                format!(
                    "Transcribed & saved as raw log (classification failed)\n\n\"{}\"",
                    truncate(&transcript, 200),
                ),
            )
            .await?;
            ctx.notify_sync();
            return Ok(());
        }
    };

    // Step 4: Write to vault
    let (section, content) = writer::format_for_daily_note(&classified);
    ctx.vault.append_to_section(section, &content).await?;

    // Update frontmatter if AI provided any
    if let Some(ref frontmatter) = classified.frontmatter {
        if !frontmatter.is_empty() {
            ctx.vault.update_frontmatter(frontmatter).await?;
        }
    }

    // Notify git sync
    ctx.notify_sync();

    // Step 5: Send confirmation
    bot.send_message(
        msg.chat.id,
        format!(
            "Voice -> {} saved as {}\n\nTranscript: \"{}\"",
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
