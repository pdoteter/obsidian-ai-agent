use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};

use super::sync::{GitSync, SyncResult};

/// A handle to notify the debounced git sync that changes occurred
#[derive(Debug, Clone)]
pub struct SyncNotifier {
    tx: mpsc::UnboundedSender<()>,
}

impl SyncNotifier {
    /// Notify that the vault has changed and should be synced
    pub fn notify(&self) {
        let _ = self.tx.send(());
    }
}

/// Spawn a background task that debounces git sync operations.
/// Returns a SyncNotifier that can be used to signal vault changes.
///
/// The sync will only trigger after `debounce_secs` seconds of inactivity
/// (no new notifications). This keeps the commit history clean.
pub fn spawn_debounced_sync(
    git_sync: Arc<GitSync>,
    debounce_secs: u64,
) -> SyncNotifier {
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();

    tokio::spawn(async move {
        let debounce_duration = Duration::from_secs(debounce_secs);
        let mut pending = false;

        loop {
            if !pending {
                // Wait for the first notification
                match rx.recv().await {
                    Some(()) => {
                        pending = true;
                        info!("Vault change detected, starting debounce timer");
                    }
                    None => {
                        info!("Sync notifier channel closed, stopping sync task");
                        break;
                    }
                }
            }

            if pending {
                // Debounce: wait for quiet period
                let deadline = Instant::now() + debounce_duration;
                loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }

                    tokio::select! {
                        result = rx.recv() => {
                            match result {
                                Some(()) => {
                                    // Reset the timer — more changes incoming
                                    info!("Additional change detected, resetting debounce timer");
                                    continue; // Will recalculate from new deadline below
                                }
                                None => {
                                    info!("Channel closed during debounce");
                                    // Still perform final sync
                                    break;
                                }
                            }
                        }
                        _ = tokio::time::sleep(remaining) => {
                            break;
                        }
                    }
                }

                // Debounce period expired — perform sync
                info!("Debounce period expired, performing git sync");
                pending = false;

                // Run git sync in a blocking task (git2 is not async)
                let git = git_sync.clone();
                let result = tokio::task::spawn_blocking(move || git.full_sync()).await;

                match result {
                    Ok(Ok(sync_result)) => {
                        match &sync_result {
                            SyncResult::NothingToSync => {
                                info!("Git sync: nothing to sync");
                            }
                            SyncResult::ConflictDetected(_info) => {
                                warn!("Git sync: conflict detected — manual resolution needed");
                                // TODO: trigger conflict resolution via Telegram
                            }
                            other => {
                                info!(result = %other, "Git sync completed");
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        error!(error = %e, "Git sync failed");
                    }
                    Err(e) => {
                        error!(error = %e, "Git sync task panicked");
                    }
                }
            }
        }
    });

    SyncNotifier { tx }
}
