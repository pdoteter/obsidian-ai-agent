use std::sync::Arc;
use teloxide::types::ChatId;
use tokio::sync::Mutex;

/// Tracks the last active Telegram chat_id from message handlers.
/// Used to determine where to send conflict notifications.
#[derive(Debug, Clone)]
pub struct ChatIdTracker {
    last_chat_id: Arc<Mutex<Option<ChatId>>>,
}

impl ChatIdTracker {
    pub fn new() -> Self {
        Self {
            last_chat_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Update the last active chat_id
    pub async fn set(&self, chat_id: ChatId) {
        let mut guard = self.last_chat_id.lock().await;
        *guard = Some(chat_id);
    }

    /// Get the last active chat_id (returns None if no message has been processed yet)
    pub async fn get(&self) -> Option<ChatId> {
        let guard = self.last_chat_id.lock().await;
        *guard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_chat_id_tracker_initially_none() {
        let tracker = ChatIdTracker::new();
        assert_eq!(tracker.get().await, None);
    }

    #[tokio::test]
    async fn test_chat_id_tracker_set_and_get() {
        let tracker = ChatIdTracker::new();
        tracker.set(ChatId(12345)).await;
        assert_eq!(tracker.get().await, Some(ChatId(12345)));
    }

    #[tokio::test]
    async fn test_chat_id_tracker_overwrites() {
        let tracker = ChatIdTracker::new();
        tracker.set(ChatId(111)).await;
        tracker.set(ChatId(222)).await;
        assert_eq!(tracker.get().await, Some(ChatId(222)));
    }

    #[tokio::test]
    async fn test_chat_id_tracker_clone_shares_state() {
        let tracker = ChatIdTracker::new();
        let clone = tracker.clone();
        tracker.set(ChatId(999)).await;
        assert_eq!(clone.get().await, Some(ChatId(999)));
    }
}
