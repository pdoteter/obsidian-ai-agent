use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{error, info};

use crate::ai::client::OpenRouterClient;
use crate::config::Config;
use crate::git::chat_tracker::ChatIdTracker;
use crate::git::debounce::SyncNotifier;
use crate::handlers::url::TranscriptPending;
use crate::vault::daily_note::DailyNoteManager;
use crate::vault::writer;

/// Handle incoming text messages: classify → format → write to vault
#[allow(clippy::too_many_arguments)]
pub async fn handle_text_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_client: Arc<OpenRouterClient>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
    transcript_pending: TranscriptPending,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return Ok(()),
    };

    // Check for URLs first and delegate to URL handler if present.
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
            config,
            ai_client,
            vault,
            sync_notifier,
            chat_tracker,
            transcript_pending,
            detected_urls,
            surrounding_text,
        )
        .await;
    }

    // Check user authorization
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring message");
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    chat_tracker.set(msg.chat.id).await;

    info!(text_length = text.len(), "Processing text message");

    bot.send_chat_action(msg.chat.id, ChatAction::Typing).await?;

    // Classify the text with AI
    let guide = crate::ai::guide::load_guide(&config.guide_path);
    let classified = match ai_client
        .classify_text(&text, &config.openrouter_model_classify, guide.as_deref())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "AI classification failed, using raw format");
            // Fallback: write as raw log entry
            let (section, content) = writer::format_raw_entry(&text);
            vault.append_to_section(section, &content).await?;
            bot.send_message(msg.chat.id, format!("📝 Saved as raw log entry (AI unavailable: {})", e))
                .await?;
            if let Some(ref notifier) = sync_notifier {
                notifier.notify();
            }
            return Ok(());
        }
    };

    // Format and write to vault
    let (section, content) = writer::format_for_daily_note(&classified);
    let _path = vault.append_to_section(section, &content).await?;

    // Update frontmatter if AI provided any
    if let Some(ref frontmatter) = classified.frontmatter {
        if !frontmatter.is_empty() {
            vault.update_frontmatter(frontmatter).await?;
        }
    }

    // Notify git sync
    if let Some(ref notifier) = sync_notifier {
        notifier.notify();
    }

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
            "✅ {} saved as **{}**\n_{}_{}",
            match classified.category {
                crate::ai::classify::NoteCategory::Todo => "📌",
                crate::ai::classify::NoteCategory::Log => "📋",
                crate::ai::classify::NoteCategory::Note => "📝",
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
