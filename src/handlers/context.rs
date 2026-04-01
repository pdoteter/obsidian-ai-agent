//! Shared context for message handlers
//!
//! Groups common dependencies to reduce argument count in handler functions.

use std::sync::Arc;

use crate::ai::client::OpenRouterClient;
use crate::config::Config;
use crate::git::chat_tracker::ChatIdTracker;
use crate::git::debounce::SyncNotifier;
use crate::vault::daily_note::DailyNoteManager;

/// Shared context for all message handlers.
///
/// Groups the common dependencies that every handler needs, reducing
/// the number of function arguments from 7+ to a single context parameter.
#[derive(Clone)]
pub struct HandlerContext {
    pub config: Arc<Config>,
    pub ai_client: Arc<OpenRouterClient>,
    pub vault: Arc<DailyNoteManager>,
    pub sync_notifier: Option<SyncNotifier>,
    pub chat_tracker: ChatIdTracker,
}

impl HandlerContext {
    pub fn new(
        config: Arc<Config>,
        ai_client: Arc<OpenRouterClient>,
        vault: Arc<DailyNoteManager>,
        sync_notifier: Option<SyncNotifier>,
        chat_tracker: ChatIdTracker,
    ) -> Self {
        Self {
            config,
            ai_client,
            vault,
            sync_notifier,
            chat_tracker,
        }
    }

    /// Notify git sync if enabled
    pub fn notify_sync(&self) {
        if let Some(ref notifier) = self.sync_notifier {
            notifier.notify();
        }
    }
}
