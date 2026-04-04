use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use teloxide::prelude::Requester;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::error::GitError;
use super::chat_tracker::ChatIdTracker;
use super::conflict::ConflictResolver;
use super::sync::{GitSync, SyncResult};
use crate::ai::client::OpenRouterClient;
use crate::ai::conflict::analyze_conflicts;
use crate::config::Config;

/// A handle to notify the debounced git sync that changes occurred
#[derive(Debug, Clone)]
pub struct SyncNotifier {
    tx: mpsc::UnboundedSender<()>,
    git_sync: Arc<GitSync>,
    debounce_pending: Arc<AtomicBool>,
    sync_running: Arc<AtomicBool>,
    resolving: Arc<AtomicBool>,
    git_lock: Arc<Mutex<()>>,
}

impl SyncNotifier {
    /// Notify that the vault has changed and should be synced
    pub fn notify(&self) {
        let _ = self.tx.send(());
    }

    pub fn is_busy(&self) -> bool {
        self.debounce_pending.load(Ordering::SeqCst)
            || self.sync_running.load(Ordering::SeqCst)
            || self.resolving.load(Ordering::SeqCst)
    }

    pub async fn pull_if_idle(&self) -> Result<Option<super::sync::PreWriteSyncResult>, GitError> {
        if self.is_busy() {
            return Ok(None);
        }

        let Ok(_git_guard) = self.git_lock.try_lock() else {
            return Ok(None);
        };

        let git_sync = self.git_sync.clone();
        let sync_running = self.sync_running.clone();

        sync_running.store(true, Ordering::SeqCst);

        let result = tokio::task::spawn_blocking(move || git_sync.pull_if_clean()).await;

        sync_running.store(false, Ordering::SeqCst);

        match result {
            Ok(inner) => inner.map(Some),
            Err(error) => Err(GitError::CommandFailed {
                command: "pre_write_pull".to_string(),
                message: format!("spawn_blocking failed: {}", error),
            }),
        }
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
    conflict_resolver: ConflictResolver,
    ai_client: Arc<OpenRouterClient>,
    config: Arc<Config>,
    chat_tracker: ChatIdTracker,
) -> SyncNotifier {
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();

    // Guard to prevent nested conflict resolution
    let resolving = Arc::new(AtomicBool::new(false));
    let debounce_pending = Arc::new(AtomicBool::new(false));
    let sync_running = Arc::new(AtomicBool::new(false));
    let git_lock = Arc::new(Mutex::new(()));

    let notifier = SyncNotifier {
        tx,
        git_sync: git_sync.clone(),
        debounce_pending: debounce_pending.clone(),
        sync_running: sync_running.clone(),
        resolving: resolving.clone(),
        git_lock: git_lock.clone(),
    };

    tokio::spawn(async move {
        let debounce_duration = Duration::from_secs(debounce_secs);
        let mut pending = false;

        loop {
            if !pending {
                // Wait for the first notification
                match rx.recv().await {
                    Some(()) => {
                        pending = true;
                        debounce_pending.store(true, Ordering::SeqCst);
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
                let sleep = tokio::time::sleep(debounce_duration);
                tokio::pin!(sleep);
                loop {
                    tokio::select! {
                        result = rx.recv() => {
                            match result {
                                Some(()) => {
                                    // Reset the timer — more changes incoming
                                    info!("Additional change detected, resetting debounce timer");
                                    sleep.as_mut().reset(Instant::now() + debounce_duration);
                                    continue;
                                }
                                None => {
                                    info!("Channel closed during debounce");
                                    // Still perform final sync
                                    break;
                                }
                            }
                        }
                        _ = &mut sleep => {
                            break;
                        }
                    }
                }

                // Debounce period expired — perform sync
                info!("Debounce period expired, performing git sync");
                pending = false;
                debounce_pending.store(false, Ordering::SeqCst);

                // Check if already resolving a conflict
                if resolving.load(Ordering::SeqCst) {
                    warn!("Skipping sync — conflict resolution in progress");
                    continue;
                }

                // Run git sync in a blocking task (git2 is not async)
                let git = git_sync.clone();
                let _git_guard = git_lock.lock().await;
                sync_running.store(true, Ordering::SeqCst);
                let result = tokio::task::spawn_blocking(move || git.full_sync()).await;
                sync_running.store(false, Ordering::SeqCst);

                match result {
                    Ok(Ok(sync_result)) => {
                        match sync_result {
                            SyncResult::NothingToSync => {
                                info!("Git sync: nothing to sync");
                            }
                            SyncResult::ConflictDetected(info) => {
                                warn!(files = ?info.files, "Git sync: conflict detected");
                                
                                // Set resolving flag
                                resolving.store(true, Ordering::SeqCst);

                                // Get chat_id from tracker
                                let chat_id = match chat_tracker.get().await {
                                    Some(id) => id,
                                    None => {
                                        error!("No chat_id available for conflict notification — skipping Telegram notification");
                                        resolving.store(false, Ordering::SeqCst);
                                        continue;
                                    }
                                };

                                // Call AI analysis with timeout
                                let ai_analysis = {
                                    let ai_client = ai_client.clone();
                                    let config = config.clone();
                                    let info_clone = info.clone();
                                    
                                    match tokio::time::timeout(
                                        Duration::from_secs(25),
                                        analyze_conflicts(&ai_client, &config.openrouter_model_classify, &info_clone)
                                    ).await {
                                        Ok(Ok(analysis)) => Some(analysis),
                                        Ok(Err(e)) => {
                                            warn!(error = %e, "AI conflict analysis failed");
                                            None
                                        }
                                        Err(_) => {
                                            warn!("AI conflict analysis timed out after 25s");
                                            None
                                        }
                                    }
                                };

                                // Prepare diff preview (truncate to 2500 chars)
                                let diff_preview = if info.diff_output.len() > 2500 {
                                    let mut end = 2500;
                                    while !info.diff_output.is_char_boundary(end) && end > 0 {
                                        end -= 1;
                                    }
                                    format!("{}... (truncated)", &info.diff_output[..end])
                                } else {
                                    info.diff_output.clone()
                                };

                                // Ask user for resolution
                                let resolution_result = conflict_resolver
                                    .ask_resolution(chat_id, &info.files, ai_analysis, &diff_preview)
                                    .await;

                                match resolution_result {
                                    Ok(resolution) => {
                                        info!(resolution = ?resolution, "User chose conflict resolution");

                                        // Execute resolution in blocking task
                                        let git = git_sync.clone();
                                        let resolution_clone = resolution.clone();
                                        let exec_result = tokio::task::spawn_blocking(move || {
                                            match resolution_clone {
                                                super::conflict::ConflictResolution::Ours => git.resolve_ours(),
                                                super::conflict::ConflictResolution::Theirs => git.resolve_theirs(),
                                                super::conflict::ConflictResolution::Abort => git.resolve_abort(),
                                            }
                                        }).await;

                                        match exec_result {
                                            Ok(Ok(())) => {
                                                info!("Conflict resolution executed successfully");

                                                // Retry sync if resolution was Ours or Theirs
                                                if resolution != super::conflict::ConflictResolution::Abort {
                                                    info!("Retrying sync after conflict resolution");
                                                    let git = git_sync.clone();
                                                    let retry_result = tokio::task::spawn_blocking(move || git.full_sync()).await;

                                                    match retry_result {
                                                        Ok(Ok(retry_sync_result)) => {
                                                            info!(result = %retry_sync_result, "Retry sync completed");
                                                        }
                                                        Ok(Err(e)) => {
                                                            error!(error = %e, "Retry sync failed after conflict resolution");
                                                        }
                                                        Err(e) => {
                                                            error!(error = %e, "Retry sync task panicked");
                                                        }
                                                    }
                                                }
                                            }
                                            Ok(Err(e)) => {
                                                error!(error = %e, "Failed to execute conflict resolution");
                                                // Try to notify user
                                                if let Err(send_err) = conflict_resolver.bot.send_message(
                                                    chat_id,
                                                    format!("❌ Failed to execute conflict resolution: {}", e)
                                                ).await {
                                                    error!(error = %send_err, "Failed to send error notification");
                                                }
                                            }
                                            Err(e) => {
                                                error!(error = %e, "Conflict resolution task panicked");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Failed to get conflict resolution from user");
                                    }
                                }

                                // Clear resolving flag
                                resolving.store(false, Ordering::SeqCst);
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

    notifier
}
