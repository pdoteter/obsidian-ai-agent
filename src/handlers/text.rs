use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{error, info};

use crate::error::AppResult;
use crate::handlers::url::TranscriptPending;
use crate::handlers::HandlerContext;
use crate::vault::writer;

/// Handle incoming text messages: classify -> format -> write to vault
pub async fn handle_text_message(
    bot: Bot,
    msg: Message,
    ctx: HandlerContext,
    transcript_pending: TranscriptPending,
) -> AppResult<()> {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return Ok(()),
    };

    // Check user authorization first (before any processing)
    if let Some(user) = msg.from.as_ref() {
        if !ctx.config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring message");
            return Ok(());
        }
    }

    // Check for URLs and delegate to URL handler if present
    let detected_urls = crate::url::detect::detect_urls(&text);
    if !detected_urls.is_empty() {
        let surrounding_text = if detected_urls.iter().all(|u| text.trim() == u.url) {
            None
        } else {
            Some(text.clone())
        };

        return crate::handlers::url::handle_url_message(
            bot,
            msg,
            ctx,
            transcript_pending,
            detected_urls,
            surrounding_text,
        )
        .await;
    }

    // Track chat_id for conflict notifications (after auth check)
    ctx.chat_tracker.set(msg.chat.id);

    info!(text_length = text.len(), "Processing text message");

    bot.send_chat_action(msg.chat.id, ChatAction::Typing).await?;

    // Classify the text with AI
    let guide = crate::ai::guide::load_guide(&ctx.config.guide_path);
    let classified = match ctx.ai_client
        .classify_text(&text, &ctx.config.openrouter_model_classify, guide.as_deref())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "AI classification failed, using raw format");
            // Fallback: write as raw log entry
            let (section, content) = writer::format_raw_entry(&text);
            ctx.vault.append_to_section(section, &content).await?;
            bot.send_message(msg.chat.id, format!("Saved as raw log entry (AI unavailable: {})", e))
                .await?;
            ctx.notify_sync();
            return Ok(());
        }
    };

    // Format and write to vault
    let (section, content) = writer::format_for_daily_note(&classified);
    let _path = ctx.vault.append_to_section(section, &content).await?;

    // Update frontmatter if AI provided any
    if let Some(ref frontmatter) = classified.frontmatter {
        if !frontmatter.is_empty() {
            ctx.vault.update_frontmatter(frontmatter).await?;
        }
    }

    // Notify git sync
    ctx.notify_sync();

    // Send confirmation
    let tags_display = if classified.tags.is_empty() {
        String::new()
    } else {
        format!(
            "\nTags: {}",
            classified.tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" ")
        )
    };

    bot.send_message(
        msg.chat.id,
        format!(
            "{} saved as **{}**\n_{}_{}",
            match classified.category {
                crate::ai::classify::NoteCategory::Todo => "",
                crate::ai::classify::NoteCategory::Log => "",
                crate::ai::classify::NoteCategory::Note => "",
            },
            classified.category,
            classified.summary,
            tags_display,
        ),
    )
    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
    .await
    .ok(); // Don't fail on formatting issues

    Ok(())
}
