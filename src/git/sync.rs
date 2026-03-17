use chrono::Local;
use git2::{
    Cred, FetchOptions, PushOptions, RemoteCallbacks, Repository, Signature, StatusOptions,
};
use std::path::PathBuf;
use tracing::{error, info, warn};

use crate::error::GitError;

/// Manages git operations for the Obsidian vault
pub struct GitSync {
    repo_path: PathBuf,
    remote_name: String,
    branch: String,
    ssh_key_path: Option<PathBuf>,
}

impl GitSync {
    pub fn new(
        repo_path: PathBuf,
        remote_name: String,
        branch: String,
        ssh_key_path: Option<PathBuf>,
    ) -> Self {
        Self {
            repo_path,
            remote_name,
            branch,
            ssh_key_path,
        }
    }

    /// Open the git repository
    fn open_repo(&self) -> Result<Repository, GitError> {
        Repository::open(&self.repo_path)
            .map_err(|_| GitError::RepoNotFound(self.repo_path.clone()))
    }

    /// Create authentication callbacks for SSH
    fn make_callbacks(&self) -> RemoteCallbacks<'_> {
        let mut callbacks = RemoteCallbacks::new();
        let ssh_key_path = self.ssh_key_path.clone();

        callbacks.credentials(move |_url, username_from_url, _allowed_types| {
            let username = username_from_url.unwrap_or("git");
            if let Some(ref key_path) = ssh_key_path {
                Cred::ssh_key(username, None, key_path, None)
            } else {
                // Try default SSH key locations
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                let default_key = PathBuf::from(&home).join(".ssh").join("id_rsa");
                let ed25519_key = PathBuf::from(&home).join(".ssh").join("id_ed25519");

                if ed25519_key.exists() {
                    Cred::ssh_key(username, None, &ed25519_key, None)
                } else if default_key.exists() {
                    Cred::ssh_key(username, None, &default_key, None)
                } else {
                    Cred::ssh_key_from_agent(username)
                }
            }
        });

        callbacks
    }

    /// Stage all changed files, commit, and return whether anything was committed
    pub fn stage_and_commit(&self) -> Result<bool, GitError> {
        let repo = self.open_repo()?;

        // Check for changes
        let mut status_opts = StatusOptions::new();
        status_opts
            .include_untracked(true)
            .recurse_untracked_dirs(true);

        let statuses = repo.statuses(Some(&mut status_opts))?;

        if statuses.is_empty() {
            info!("No changes to commit");
            return Ok(false);
        }

        info!(changed_files = statuses.len(), "Staging changes");

        // Stage all changes
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;

        // Create commit
        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;

        let sig = Signature::now("Obsidian AI Agent", "bot@obsidian-ai-agent")?;

        let timestamp = Local::now().format("%Y-%m-%d %H:%M").to_string();
        let message = format!("Telegram sync: {}", timestamp);

        let parent = match repo.head() {
            Ok(head) => {
                let target = head
                    .target()
                    .ok_or_else(|| GitError::Git2(git2::Error::from_str("HEAD has no target")))?;
                Some(repo.find_commit(target)?)
            }
            Err(_) => None, // Initial commit
        };

        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &parents)?;

        info!(message = %message, "Created commit");
        Ok(true)
    }

    /// Fetch from remote
    pub fn fetch(&self) -> Result<(), GitError> {
        let repo = self.open_repo()?;
        let mut remote = repo.find_remote(&self.remote_name)?;

        let callbacks = self.make_callbacks();
        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(callbacks);

        remote.fetch(&[&self.branch], Some(&mut fetch_opts), None)?;

        info!(remote = %self.remote_name, branch = %self.branch, "Fetched from remote");
        Ok(())
    }

    /// Check if local is behind remote and needs rebase
    pub fn needs_rebase(&self) -> Result<bool, GitError> {
        let repo = self.open_repo()?;

        let local_ref = format!("refs/heads/{}", self.branch);
        let remote_ref = format!("refs/remotes/{}/{}", self.remote_name, self.branch);

        let local_oid = match repo.refname_to_id(&local_ref) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };

        let remote_oid = match repo.refname_to_id(&remote_ref) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };

        if local_oid == remote_oid {
            return Ok(false);
        }

        // Check if remote has commits not in local
        let (ahead, behind) = repo.graph_ahead_behind(local_oid, remote_oid)?;

        info!(ahead = ahead, behind = behind, "Divergence check");
        Ok(behind > 0)
    }

    /// Perform rebase of local commits on top of remote.
    /// Returns Ok(true) if rebase succeeded, Ok(false) if conflicts detected.
    pub fn rebase(&self) -> Result<bool, GitError> {
        let repo = self.open_repo()?;

        let remote_ref = format!("refs/remotes/{}/{}", self.remote_name, self.branch);
        let upstream_oid = repo.refname_to_id(&remote_ref)?;
        let upstream = repo.find_annotated_commit(upstream_oid)?;

        let local_ref = format!("refs/heads/{}", self.branch);
        let local_oid = repo.refname_to_id(&local_ref)?;
        let branch = repo.find_annotated_commit(local_oid)?;

        let mut rebase = repo.rebase(Some(&branch), Some(&upstream), None, None)?;

        let sig = Signature::now("Obsidian AI Agent", "bot@obsidian-ai-agent")?;

        while let Some(op) = rebase.next() {
            match op {
                Ok(_operation) => {
                    let index = rebase.inmemory_index()?;
                    if index.has_conflicts() {
                        warn!("Conflict detected during rebase");
                        rebase.abort()?;
                        return Ok(false);
                    }
                    rebase.commit(None, &sig, None)?;
                }
                Err(e) => {
                    error!(error = %e, "Rebase operation failed");
                    rebase.abort()?;
                    return Err(GitError::Git2(e));
                }
            }
        }

        rebase.finish(Some(&sig))?;
        info!("Rebase completed successfully");
        Ok(true)
    }

    /// Push to remote
    pub fn push(&self) -> Result<(), GitError> {
        let repo = self.open_repo()?;
        let mut remote = repo.find_remote(&self.remote_name)?;

        let refspec = format!("refs/heads/{}", self.branch);

        let push_error: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let push_error_clone = push_error.clone();

        let mut callbacks = self.make_callbacks();
        callbacks.push_update_reference(move |refname, status| {
            if let Some(msg) = status {
                *push_error_clone.lock().unwrap() =
                    Some(format!("Push rejected for {}: {}", refname, msg));
            }
            Ok(())
        });
        let mut push_opts = PushOptions::new();
        push_opts.remote_callbacks(callbacks);

        remote.push(&[&refspec], Some(&mut push_opts))?;

        if let Some(err) = push_error.lock().unwrap().take() {
            return Err(GitError::PushFailed(err));
        }

        info!(remote = %self.remote_name, branch = %self.branch, "Pushed to remote");
        Ok(())
    }

    /// Force push (after rebase)
    pub fn force_push(&self) -> Result<(), GitError> {
        let repo = self.open_repo()?;
        let mut remote = repo.find_remote(&self.remote_name)?;

        let callbacks = self.make_callbacks();
        let mut push_opts = PushOptions::new();
        push_opts.remote_callbacks(callbacks);

        let refspec = format!("+refs/heads/{}", self.branch); // + prefix = force

        remote.push(&[&refspec], Some(&mut push_opts))?;

        info!(remote = %self.remote_name, branch = %self.branch, "Force pushed to remote");
        Ok(())
    }

    /// Full sync cycle: stage → commit → fetch → rebase → push
    pub fn full_sync(&self) -> Result<SyncResult, GitError> {
        // Stage and commit local changes
        let committed = self.stage_and_commit()?;

        // Try to fetch (may fail if no remote configured or offline)
        match self.fetch() {
            Ok(()) => {}
            Err(e) => {
                warn!(error = %e, "Fetch failed, pushing without rebase");
                if committed {
                    self.push()?;
                }
                return Ok(SyncResult::PushedWithoutFetch);
            }
        }

        // Check if rebase is needed
        if self.needs_rebase()? {
            info!("Remote has new commits, rebasing");
            let rebase_ok = self.rebase()?;
            if !rebase_ok {
                return Ok(SyncResult::ConflictDetected);
            }
            // After rebase, force push
            self.force_push()?;
            return Ok(SyncResult::RebasedAndPushed);
        }

        // Simple push (fast-forward)
        if committed {
            self.push()?;
            Ok(SyncResult::Pushed)
        } else {
            Ok(SyncResult::NothingToSync)
        }
    }

    /// Get list of conflicted files (for conflict resolution UI)
    #[allow(dead_code)]
    pub fn get_conflicted_files(&self) -> Result<Vec<String>, GitError> {
        let repo = self.open_repo()?;
        let index = repo.index()?;

        let conflicts: Vec<String> = index
            .conflicts()?
            .filter_map(|c| c.ok())
            .filter_map(|c| {
                c.our
                    .as_ref()
                    .map(|e| String::from_utf8_lossy(&e.path).to_string())
            })
            .collect();

        Ok(conflicts)
    }
}

#[derive(Debug, Clone)]
pub enum SyncResult {
    NothingToSync,
    Pushed,
    PushedWithoutFetch,
    RebasedAndPushed,
    ConflictDetected,
}

impl std::fmt::Display for SyncResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncResult::NothingToSync => write!(f, "Nothing to sync"),
            SyncResult::Pushed => write!(f, "Changes pushed"),
            SyncResult::PushedWithoutFetch => write!(f, "Pushed (fetch failed)"),
            SyncResult::RebasedAndPushed => write!(f, "Rebased and pushed"),
            SyncResult::ConflictDetected => write!(f, "Conflict detected"),
        }
    }
}
