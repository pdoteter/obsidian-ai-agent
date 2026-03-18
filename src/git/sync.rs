use chrono::Local;
use std::path::PathBuf;
use std::process::Command;
use tracing::{error, info, warn};

use crate::error::GitError;

/// Manages git operations for the Obsidian vault using system git CLI
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

    /// Build a git Command with the repo path and optional SSH key configured
    fn git_cmd(&self) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.repo_path);

        // Configure SSH key via GIT_SSH_COMMAND if specified
        if let Some(ref key_path) = self.ssh_key_path {
            let ssh_command = format!(
                "ssh -i \"{}\" -o StrictHostKeyChecking=accept-new",
                key_path.display()
            );
            cmd.env("GIT_SSH_COMMAND", &ssh_command);
            info!(ssh_command = %ssh_command, "Configured GIT_SSH_COMMAND");
        }

        cmd
    }

    /// Run a git command and return stdout on success, or GitError on failure
    fn run_git(&self, args: &[&str]) -> Result<String, GitError> {
        let args_display = args.join(" ");
        info!(command = %format!("git {}", args_display), repo = %self.repo_path.display(), "Running git command");

        let output = self.git_cmd().args(args).output().map_err(|e| {
            error!(error = %e, "Failed to execute git command");
            GitError::CommandFailed {
                command: format!("git {}", args_display),
                message: format!("Failed to execute: {}", e),
            }
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let exit_code = output.status.code().unwrap_or(-1);
            error!(
                command = %format!("git {}", args_display),
                exit_code = exit_code,
                stderr = %stderr.trim(),
                stdout = %stdout.trim(),
                "Git command failed"
            );
            return Err(GitError::CommandFailed {
                command: format!("git {}", args_display),
                message: stderr.trim().to_string(),
            });
        }

        if !stderr.trim().is_empty() {
            // Git often writes informational messages to stderr
            info!(stderr = %stderr.trim(), "Git stderr output");
        }

        Ok(stdout)
    }

    /// Check if there are any uncommitted changes in the repo
    fn has_changes(&self) -> Result<bool, GitError> {
        let output = self.run_git(&["status", "--porcelain"])?;
        Ok(!output.trim().is_empty())
    }

    /// Stage all changed files, commit, and return whether anything was committed
    pub fn stage_and_commit(&self) -> Result<bool, GitError> {
        if !self.has_changes()? {
            info!("No changes to commit");
            return Ok(false);
        }

        // Stage all changes
        self.run_git(&["add", "--all"])?;
        info!("Staged all changes");

        // Create commit
        let timestamp = Local::now().format("%Y-%m-%d %H:%M").to_string();
        let message = format!("Telegram sync: {}", timestamp);

        self.run_git(&[
            "commit",
            "-m",
            &message,
            "--author",
            "Obsidian AI Agent <bot@obsidian-ai-agent>",
        ])?;

        info!(message = %message, "Created commit");
        Ok(true)
    }

    /// Fetch from remote
    pub fn fetch(&self) -> Result<(), GitError> {
        info!(
            remote = %self.remote_name,
            branch = %self.branch,
            "Fetching from remote"
        );

        self.run_git(&["fetch", &self.remote_name, &self.branch])?;

        info!(remote = %self.remote_name, branch = %self.branch, "Fetched from remote");
        Ok(())
    }

    /// Check if local is behind remote and needs rebase
    pub fn needs_rebase(&self) -> Result<bool, GitError> {
        let local_ref = format!("refs/heads/{}", self.branch);
        let remote_ref = format!("refs/remotes/{}/{}", self.remote_name, self.branch);

        // Get local and remote commit hashes
        let local_oid = match self.run_git(&["rev-parse", &local_ref]) {
            Ok(oid) => oid.trim().to_string(),
            Err(_) => return Ok(false),
        };

        let remote_oid = match self.run_git(&["rev-parse", &remote_ref]) {
            Ok(oid) => oid.trim().to_string(),
            Err(_) => return Ok(false),
        };

        if local_oid == remote_oid {
            return Ok(false);
        }

        // Count commits behind remote
        let behind_output = self.run_git(&[
            "rev-list",
            "--count",
            &format!("{}..{}", local_ref, remote_ref),
        ])?;
        let behind: usize = behind_output.trim().parse().unwrap_or(0);

        let ahead_output = self.run_git(&[
            "rev-list",
            "--count",
            &format!("{}..{}", remote_ref, local_ref),
        ])?;
        let ahead: usize = ahead_output.trim().parse().unwrap_or(0);

        info!(ahead = ahead, behind = behind, "Divergence check");
        Ok(behind > 0)
    }

    /// Perform rebase of local commits on top of remote.
    /// Returns Ok(true) if rebase succeeded, Ok(false) if conflicts detected.
    pub fn rebase(&self) -> Result<bool, GitError> {
        let remote_ref = format!("{}/{}", self.remote_name, self.branch);

        info!(upstream = %remote_ref, "Rebasing onto remote");

        match self.run_git(&["rebase", &remote_ref]) {
            Ok(_) => {
                info!("Rebase completed successfully");
                Ok(true)
            }
            Err(e) => {
                // Check if it's a conflict
                let status = self.run_git(&["status", "--porcelain"]).unwrap_or_default();
                if status.contains("UU ") || status.contains("AA ") || status.contains("DD ") {
                    warn!("Conflict detected during rebase");
                    // Abort the rebase
                    if let Err(abort_err) = self.run_git(&["rebase", "--abort"]) {
                        error!(error = %abort_err, "Failed to abort rebase");
                    }
                    return Ok(false);
                }

                // Not a conflict — actual error
                // Try to abort rebase if one is in progress
                let _ = self.run_git(&["rebase", "--abort"]);
                Err(e)
            }
        }
    }

    /// Push to remote
    pub fn push(&self) -> Result<(), GitError> {
        info!(
            remote = %self.remote_name,
            branch = %self.branch,
            "Pushing to remote"
        );

        self.run_git(&["push", &self.remote_name, &self.branch])?;

        info!(remote = %self.remote_name, branch = %self.branch, "Pushed to remote");
        Ok(())
    }

    /// Force push (after rebase)
    pub fn force_push(&self) -> Result<(), GitError> {
        info!(
            remote = %self.remote_name,
            branch = %self.branch,
            "Force pushing to remote"
        );

        self.run_git(&[
            "push",
            "--force-with-lease",
            &self.remote_name,
            &self.branch,
        ])?;

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
        let output = self.run_git(&["diff", "--name-only", "--diff-filter=U"])?;
        let conflicts: Vec<String> = output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
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
