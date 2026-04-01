use teloxide::types::ChatId;
use tokio::sync::watch;

/// Tracks the last active Telegram chat_id from message handlers.
/// Used to determine where to send conflict notifications.
///
/// Uses a `watch` channel instead of `Mutex` for better performance:
/// - Writes are infrequent (once per message)
/// - Reads happen only during conflict resolution
/// - `watch` has cheaper reads with no lock contention
#[derive(Debug, Clone)]
pub struct ChatIdTracker {
    tx: watch::Sender<Option<ChatId>>,
    rx: watch::Receiver<Option<ChatId>>,
}

impl ChatIdTracker {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(None);
        Self { tx, rx }
    }

    /// Update the last active chat_id
    pub fn set(&self, chat_id: ChatId) {
        // send() only fails if all receivers are dropped, which won't happen
        // since we hold a receiver in self.rx
        let _ = self.tx.send(Some(chat_id));
    }

    /// Get the last active chat_id (returns None if no message has been processed yet)
    pub fn get(&self) -> Option<ChatId> {
        *self.rx.borrow()
    }
}

impl Default for ChatIdTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_id_tracker_initially_none() {
        let tracker = ChatIdTracker::new();
        assert_eq!(tracker.get(), None);
    }

    #[test]
    fn test_chat_id_tracker_set_and_get() {
        let tracker = ChatIdTracker::new();
        tracker.set(ChatId(12345));
        assert_eq!(tracker.get(), Some(ChatId(12345)));
    }

    #[test]
    fn test_chat_id_tracker_overwrites() {
        let tracker = ChatIdTracker::new();
        tracker.set(ChatId(111));
        tracker.set(ChatId(222));
        assert_eq!(tracker.get(), Some(ChatId(222)));
    }

    #[test]
    fn test_chat_id_tracker_clone_shares_state() {
        let tracker = ChatIdTracker::new();
        let clone = tracker.clone();
        tracker.set(ChatId(999));
        assert_eq!(clone.get(), Some(ChatId(999)));
    }

    #[test]
    fn test_chat_id_tracker_default() {
        let tracker = ChatIdTracker::default();
        assert_eq!(tracker.get(), None);
    }
}
