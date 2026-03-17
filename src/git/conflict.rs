use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use tokio::sync::{Mutex, oneshot};
use tracing::info;

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
    bot: Bot,
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
    ) -> Result<ConflictResolution, Box<dyn std::error::Error + Send + Sync>> {
        let conflict_id = uuid::Uuid::new_v4().to_string();
        let files_display = conflicted_files.join("\n  • ");

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

        let message = format!(
            "⚠️ **Git Conflict Detected**\n\nConflicting files:\n  • {}\n\nHow would you like to resolve this?",
            files_display
        );

        self.bot
            .send_message(chat_id, message)
            .reply_markup(keyboard)
            .await?;

        // Create a oneshot channel to wait for the user's response
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(conflict_id.clone(), tx);
        }

        // Wait for the user's response (with a timeout)
        let resolution = tokio::time::timeout(
            std::time::Duration::from_secs(300), // 5 minute timeout
            rx,
        )
        .await
        .map_err(|_| "Conflict resolution timed out after 5 minutes")?
        .map_err(|_| "Conflict resolution channel closed")?;

        // Clean up
        {
            let mut pending = self.pending.lock().await;
            pending.remove(&conflict_id);
        }

        info!(resolution = ?resolution, "User resolved conflict");
        Ok(resolution)
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
