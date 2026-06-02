use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{ChatAction, ReactionType};
use tracing::{error, info};

use crate::ai::classify::{ClassifiedNote, NoteCategory};
use crate::ai::AiService;
use crate::config::{Config, LogAckMode};
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
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
    transcript_pending: TranscriptPending,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return Ok(()),
    };

    // Check user authorization first (before any processing)
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring message");
            return Ok(());
        }
    }

    // Intercept Git commands
    if text == "/git_refresh" {
        if let Some(ref notifier) = sync_notifier {
            if notifier.is_busy() {
                bot.send_message(
                    msg.chat.id,
                    "⏳ Git operation is already in progress. Please try again in a moment.",
                )
                .await?;
                return Ok(());
            }

            let bot_clone = bot.clone();
            let chat_id = msg.chat.id;
            let notifier_clone = notifier.clone();

            bot.send_message(chat_id, "🔄 Starting manual Git synchronization...")
                .await?;

            tokio::spawn(async move {
                match notifier_clone.manual_sync(chat_id).await {
                    Ok(crate::git::sync::SyncResult::NothingToSync) => {
                        let _ = bot_clone
                            .send_message(chat_id, "✅ Git sync completed: Nothing to sync.")
                            .await;
                    }
                    Ok(crate::git::sync::SyncResult::Pushed) => {
                        let _ = bot_clone
                            .send_message(chat_id, "✅ Git sync completed: Local changes pushed to remote.")
                            .await;
                    }
                    Ok(crate::git::sync::SyncResult::PushedWithoutFetch) => {
                        let _ = bot_clone
                            .send_message(chat_id, "✅ Git sync completed: Pushed without fetch (remote offline).")
                            .await;
                    }
                    Ok(crate::git::sync::SyncResult::RebasedAndPushed) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id,
                                "✅ Git sync completed: Rebased local changes on top of remote and pushed successfully.",
                            )
                            .await;
                    }
                    Ok(crate::git::sync::SyncResult::ConflictDetected(_)) => {
                        // Conflict resolution was already handled interactively inside manual_sync
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(chat_id, format!("❌ Git sync failed: {}", e))
                            .await;
                    }
                }
            });
        } else {
            bot.send_message(
                msg.chat.id,
                "⚠️ Git synchronization is disabled in configuration.",
            )
            .await?;
        }
        return Ok(());
    } else if text == "/git_force_refresh" {
        if let Some(ref notifier) = sync_notifier {
            if notifier.is_busy() {
                bot.send_message(
                    msg.chat.id,
                    "⏳ Git operation is already in progress. Please try again in a moment.",
                )
                .await?;
                return Ok(());
            }

            let bot_clone = bot.clone();
            let chat_id = msg.chat.id;
            let notifier_clone = notifier.clone();

            bot.send_message(
                chat_id,
                "🔄 Performing force refresh: fetching and hard resetting local vault to the remote branch, discarding all local changes...",
            )
            .await?;

            tokio::spawn(async move {
                match notifier_clone.force_refresh().await {
                    Ok(()) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id,
                                "✅ Force refresh completed successfully. The agent vault is now aligned with the latest remote version!",
                            )
                            .await;
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(chat_id, format!("❌ Force refresh failed: {}", e))
                            .await;
                    }
                }
            });
        } else {
            bot.send_message(
                msg.chat.id,
                "⚠️ Git synchronization is disabled in configuration.",
            )
            .await?;
        }
        return Ok(());
    } else if text == "/finance_tokens" {
        let classify = config.max_tokens_classify.load(std::sync::atomic::Ordering::SeqCst);
        let query = config.max_tokens_query.load(std::sync::atomic::Ordering::SeqCst);
        let transaction = config.max_tokens_transaction.load(std::sync::atomic::Ordering::SeqCst);
        let reply = format!(
            "<b>📊 Current Finance Bot max_tokens limits:</b>\n\
             • Classify: <code>{}</code>\n\
             • Query: <code>{}</code>\n\
             • Transaction Update: <code>{}</code>\n\n\
             To change a limit, use:\n\
             <code>/set_finance_tokens &lt;classify|query|transaction&gt; &lt;value&gt;</code>",
            classify, query, transaction
        );
        bot.send_message(msg.chat.id, reply)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
        return Ok(());
    } else if text.starts_with("/set_finance_tokens") {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let reply = if parts.len() == 3 {
            let target = parts[1].to_lowercase();
            if let Ok(val) = parts[2].parse::<u32>() {
                match target.as_str() {
                    "classify" => {
                        config.max_tokens_classify.store(val, std::sync::atomic::Ordering::SeqCst);
                        format!("✅ Updated <b>classify</b> max_tokens to <code>{}</code>", val)
                    }
                    "query" => {
                        config.max_tokens_query.store(val, std::sync::atomic::Ordering::SeqCst);
                        format!("✅ Updated <b>query</b> max_tokens to <code>{}</code>", val)
                    }
                    "transaction" | "update" => {
                        config.max_tokens_transaction.store(val, std::sync::atomic::Ordering::SeqCst);
                        format!("✅ Updated <b>transaction</b> max_tokens to <code>{}</code>", val)
                    }
                    _ => "❌ Unknown limit target. Use <code>classify</code>, <code>query</code>, or <code>transaction</code>.".to_string(),
                }
            } else {
                "❌ Invalid token value. Must be a positive integer.".to_string()
            }
        } else {
            "❌ Usage: <code>/set_finance_tokens &lt;classify|query|transaction&gt; &lt;value&gt;</code>".to_string()
        };
        bot.send_message(msg.chat.id, reply)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
        return Ok(());
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
            config,
            ai_service,
            vault,
            sync_notifier,
            chat_tracker,
            transcript_pending,
            detected_urls,
            surrounding_text,
        )
        .await;
    }

    // Track chat_id for conflict notifications (after auth check)
    chat_tracker.set(msg.chat.id).await;

    info!(text_length = text.len(), "Processing text message");

    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    // Process the text note entry (classify, write to daily note, notify sync)
    let process_result =
        process_text_entry(&text, &config, &ai_service, &vault, sync_notifier.as_ref()).await;

    match process_result {
        Ok((classified, ai_success)) => {
            if !ai_success {
                // Sent as raw entry since AI failed, but notify user
                bot.send_message(
                    msg.chat.id,
                    "📝 Saved as raw log entry (AI classification unavailable)",
                )
                .await?;
            } else if let Err(error) = send_confirmation(&bot, &msg, &config, &classified).await {
                error!(error = %error, "Failed to send text confirmation");
            }
        }
        Err(e) => {
            error!(error = %e, "Failed to process text entry");
            bot.send_message(msg.chat.id, format!("❌ Failed to save: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Process a text entry: classify → format → write to vault → update frontmatter → notify sync.
/// Returns the ClassifiedNote and a boolean indicating if AI classification succeeded.
pub async fn process_text_entry(
    text: &str,
    config: &Config,
    ai_service: &AiService,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
) -> Result<(ClassifiedNote, bool), Box<dyn std::error::Error + Send + Sync>> {
    let guide = crate::ai::guide::load_guide(&config.guide_path).await;
    match ai_service
        .classify_text(text, &config.openrouter_model_classify, guide.as_deref())
        .await
    {
        Ok(c) => {
            // Format and write to vault
            let (section, content) = writer::format_for_daily_note(&c);
            vault.append_to_section(section, &content).await?;

            // Update frontmatter if AI provided any
            if let Some(ref frontmatter) = c.frontmatter {
                if !frontmatter.is_empty() {
                    vault.update_frontmatter(frontmatter).await?;
                }
            }

            // Notify git sync
            if let Some(notifier) = sync_notifier {
                notifier.notify();
            }

            Ok((c, true))
        }
        Err(e) => {
            error!(error = %e, "AI classification failed, using raw format");
            // Fallback: write as raw log entry
            let (section, content) = writer::format_raw_entry(text);
            vault.append_to_section(section, &content).await?;

            if let Some(notifier) = sync_notifier {
                notifier.notify();
            }

            Ok((
                ClassifiedNote {
                    category: NoteCategory::Log,
                    markdown: format!("- {}", text),
                    tags: Vec::new(),
                    summary: text.to_string(),
                    frontmatter: None,
                },
                false,
            ))
        }
    }
}

fn build_confirmation_message(classified: &ClassifiedNote) -> String {
    match classified.category {
        NoteCategory::Log => "👍".to_string(),
        NoteCategory::Todo | NoteCategory::Note => {
            let tags_display = if classified.tags.is_empty() {
                String::new()
            } else {
                format!(
                    "\nTags: {}",
                    classified
                        .tags
                        .iter()
                        .map(|tag| format!("#{}", tag))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            };

            format!(
                "✅ {} saved as {}\n{}{}",
                match classified.category {
                    NoteCategory::Todo => "📌",
                    NoteCategory::Note => "📝",
                    NoteCategory::Log => unreachable!("log confirmations use thumbs up"),
                },
                classified.category,
                classified.summary,
                tags_display,
            )
        }
    }
}

async fn send_confirmation(
    bot: &Bot,
    msg: &Message,
    config: &Config,
    classified: &ClassifiedNote,
) -> Result<(), teloxide::RequestError> {
    match classified.category {
        NoteCategory::Log => send_log_acknowledgement(bot, msg, config).await,
        NoteCategory::Todo | NoteCategory::Note => {
            let confirmation = build_confirmation_message(classified);
            bot.send_message(msg.chat.id, confirmation)
                .await
                .map(|_| ())
        }
    }
}

async fn send_log_acknowledgement(
    bot: &Bot,
    msg: &Message,
    config: &Config,
) -> Result<(), teloxide::RequestError> {
    match config.ack.log_mode {
        LogAckMode::Reaction => {
            let reaction_result = bot
                .set_message_reaction(msg.chat.id, msg.id)
                .reaction(vec![ReactionType::Emoji {
                    emoji: config.ack.reaction_emoji.clone(),
                }])
                .is_big(false)
                .send()
                .await;

            match reaction_result {
                Ok(_) => Ok(()),
                Err(error) => {
                    error!(error = %error, "Failed to set log reaction, falling back to text acknowledgement");
                    bot.send_message(msg.chat.id, config.ack.log_text.clone())
                        .await
                        .map(|_| ())
                }
            }
        }
        LogAckMode::Text => bot
            .send_message(msg.chat.id, config.ack.log_text.clone())
            .await
            .map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::build_confirmation_message;
    use crate::ai::classify::{ClassifiedNote, NoteCategory};
    use crate::config::{AckConfig, LogAckMode};

    #[test]
    fn log_confirmation_is_plain_thumbs_up() {
        let classified = ClassifiedNote {
            category: NoteCategory::Log,
            markdown: "- Weight logged".to_string(),
            tags: vec!["health".to_string()],
            summary: "Weight logged".to_string(),
            frontmatter: None,
        };

        assert_eq!(build_confirmation_message(&classified), "👍");
    }

    #[test]
    fn todo_confirmation_keeps_summary_and_tags() {
        let classified = ClassifiedNote {
            category: NoteCategory::Todo,
            markdown: "- [ ] Buy milk".to_string(),
            tags: vec!["shopping".to_string(), "home".to_string()],
            summary: "Buy milk".to_string(),
            frontmatter: None,
        };

        assert_eq!(
            build_confirmation_message(&classified),
            "✅ 📌 saved as todo\nBuy milk\nTags: #shopping #home"
        );
    }

    #[test]
    fn log_ack_config_defaults_to_small_reaction_mode() {
        let ack = AckConfig::default();

        assert_eq!(ack.log_mode, LogAckMode::Reaction);
        assert_eq!(ack.log_text, "Done 👍");
        assert_eq!(ack.reaction_emoji, "👍");
    }
}
