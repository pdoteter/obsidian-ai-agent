use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::{Mutex, oneshot};
use tracing::{info, warn};

/// Resolution strategy for a git conflict
#[derive(Debug, Clone, PartialEq)]
pub enum ConflictResolution {
    Ours,
    Theirs,
    Abort,
}

/// Manages conflict resolution via Telegram inline keyboard
#[allow(dead_code)]
pub struct ConflictResolver {
    pub bot: Bot,
    /// Pending conflict resolutions: callback_data_prefix → sender
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ConflictResolution>>>>,
}

impl ConflictResolver {
    pub fn new(bot: Bot) -> Self {
        Self {
            bot,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get a clone of the pending map for the callback handler
    pub fn pending_map(&self) -> Arc<Mutex<HashMap<String, oneshot::Sender<ConflictResolution>>>> {
        self.pending.clone()
    }

    /// Ask the user to resolve a conflict via Telegram inline keyboard.
    /// Returns the user's chosen resolution.
    #[allow(dead_code)]
    pub async fn ask_resolution(
        &self,
        chat_id: ChatId,
        conflicted_files: &[String],
        ai_analysis: Option<String>,
        diff_preview: &str,
    ) -> Result<ConflictResolution, Box<dyn std::error::Error + Send + Sync>> {
        let conflict_id = uuid::Uuid::new_v4().to_string();

        // Build message components
        let mut message_parts = vec![
            "⚠️ <b>Git Conflict Detected</b>\n".to_string(),
        ];

        // File list section
        let files_display = conflicted_files.iter()
            .map(|f| format!("  • {}", f))
            .collect::<Vec<_>>()
            .join("\n");
        message_parts.push(format!("\n<b>Conflicting files:</b>\n{}\n", files_display));

        // AI analysis section (if available)
        if let Some(ref analysis) = ai_analysis {
            message_parts.push(format!("\n🤖 <b>AI Analysis:</b>\n{}\n", analysis));
        }

        // Diff preview section (truncated to fit within Telegram's 4096 char limit)
        // Reserve: ~100 for header, ~200 for file list, ~800 for AI, ~100 for keyboard = 2800 remaining for diff
        let max_diff_len = 2500;
        let truncated_diff = if diff_preview.len() > max_diff_len {
            let mut end = max_diff_len;
            while !diff_preview.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            format!("{}\n... (truncated)", &diff_preview[..end])
        } else {
            diff_preview.to_string()
        };

        message_parts.push(format!("\n<b>Diff preview:</b>\n<pre>{}</pre>\n", truncated_diff));

        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(
                "✅ Use my version (Ours)",
                format!("conflict:{}:ours", conflict_id),
            ),
            InlineKeyboardButton::callback(
                "📥 Use server version (Theirs)",
                format!("conflict:{}:theirs", conflict_id),
            ),
            InlineKeyboardButton::callback(
                "❌ Abort",
                format!("conflict:{}:abort", conflict_id),
            ),
        ]]);

        let full_message = message_parts.join("");

        // Check if message fits within Telegram's 4096 char limit
        if full_message.len() > 4096 {
            // Send diff preview as separate message first, then send resolution message without diff
            let diff_msg = format!("<b>Diff preview:</b>\n<pre>{}</pre>", truncated_diff);
            self.bot
                .send_message(chat_id, diff_msg)
                .parse_mode(ParseMode::Html)
                .await?;

            // Build simplified message without diff
            let mut simple_parts = vec![
                "⚠️ <b>Git Conflict Detected</b>\n".to_string(),
                format!("\n<b>Conflicting files:</b>\n{}\n", files_display),
            ];
            if let Some(analysis) = ai_analysis {
                simple_parts.push(format!("\n🤖 <b>AI Analysis:</b>\n{}\n", analysis));
            }
            simple_parts.push("\n(See diff above)".to_string());
            let simple_message = simple_parts.join("");

            self.bot
                .send_message(chat_id, simple_message)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        } else {
            // Send single message with all content
            self.bot
                .send_message(chat_id, full_message)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }

        // Create a oneshot channel to wait for the user's response
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(conflict_id.clone(), tx);
        }

        // Wait for the user's response (with a 30 minute timeout)
        let resolution = tokio::time::timeout(
            std::time::Duration::from_secs(1800), // 30 minute timeout
            rx,
        )
        .await;

        match resolution {
            Ok(Ok(res)) => {
                // Clean up
                {
                    let mut pending = self.pending.lock().await;
                    pending.remove(&conflict_id);
                }

                info!(resolution = ?res, "User resolved conflict");
                Ok(res)
            }
            Ok(Err(_)) => {
                // Channel closed
                {
                    let mut pending = self.pending.lock().await;
                    pending.remove(&conflict_id);
                }
                Err("Conflict resolution channel closed".into())
            }
            Err(_) => {
                // Timeout
                {
                    let mut pending = self.pending.lock().await;
                    pending.remove(&conflict_id);
                }

                // Send timeout notification
                let timeout_msg = "⏰ Conflict resolution timed out after 30 minutes. The conflict is still present — next sync attempt will re-detect it.";
                if let Err(e) = self.bot.send_message(chat_id, timeout_msg).await {
                    warn!(error = %e, "Failed to send timeout notification");
                }

                Err("Conflict resolution timed out after 30 minutes".into())
            }
        }
    }
}

/// Handle callback queries for conflict resolution
pub async fn handle_conflict_callback(
    bot: Bot,
    q: CallbackQuery,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ConflictResolution>>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = match q.data {
        Some(ref d) if d.starts_with("conflict:") => d.clone(),
        _ => return Ok(()),
    };

    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 3 {
        return Ok(());
    }

    let conflict_id = parts[1].to_string();
    let resolution = match parts[2] {
        "ours" => ConflictResolution::Ours,
        "theirs" => ConflictResolution::Theirs,
        "abort" => ConflictResolution::Abort,
        _ => return Ok(()),
    };

    // Send the resolution to the waiting task
    let mut pending = pending.lock().await;
    if let Some(sender) = pending.remove(&conflict_id) {
        let resolution_text = match &resolution {
            ConflictResolution::Ours => "Using your local version",
            ConflictResolution::Theirs => "Using server version",
            ConflictResolution::Abort => "Rebase aborted",
        };

        // Answer the callback query
        bot.answer_callback_query(&q.id)
            .text(resolution_text)
            .await?;

        // Update the message
        if let Some(msg) = q.message {
            bot.edit_message_text(
                msg.chat().id,
                msg.id(),
                format!("✅ Conflict resolved: {}", resolution_text),
            )
            .await
            .ok();
        }

        let _ = sender.send(resolution);
    }

    Ok(())
}
