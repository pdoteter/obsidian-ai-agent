use std::path::PathBuf;

/// Top-level application error type
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(#[from] crate::config::ConfigError),

    #[error("Telegram error: {0}")]
    Telegram(#[from] teloxide::RequestError),

    #[error("Audio processing error: {0}")]
    Audio(#[from] AudioError),

    #[error("AI/OpenRouter error: {0}")]
    Ai(#[from] AiError),

    #[error("Vault error: {0}")]
    Vault(#[from] VaultError),

    #[error("Git error: {0}")]
    Git(#[from] GitError),
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("Failed to download file from Telegram: {0}")]
    Download(String),

    #[error("Failed to convert audio with ffmpeg: {0}")]
    Conversion(String),

    #[error("ffmpeg not found in PATH")]
    FfmpegNotFound,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("API returned error {status}: {message}")]
    ApiError { status: u16, message: String },

    #[error("Rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("Failed to parse AI response: {0}")]
    ParseError(String),

    #[error("Transcription failed: {0}")]
    TranscriptionFailed(String),

    #[error("Classification failed: {0}")]
    ClassificationFailed(String),

    #[error("Max retries ({0}) exceeded")]
    MaxRetriesExceeded(u32),
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum VaultError {
    #[error("IO error writing to vault: {0}")]
    Io(#[from] std::io::Error),

    #[error("Daily note path could not be created: {0}")]
    PathError(PathBuf),

    #[error("Template error: {0}")]
    Template(String),
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum GitError {
    #[error("Git operation failed: {0}")]
    Git2(#[from] git2::Error),

    #[error("Repository not found at: {0}")]
    RepoNotFound(PathBuf),

    #[error("Conflict detected in {file_count} file(s)")]
    ConflictDetected { file_count: usize },

    #[error("Rebase aborted by user")]
    RebaseAborted,

    #[error("Push failed: {0}")]
    PushFailed(String),

    #[error("SSH authentication failed: {0}")]
    SshAuthFailed(String),
}

/// Convenience type alias
#[allow(dead_code)]
pub type AppResult<T> = Result<T, AppError>;
