use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("Failed to download file from Telegram: {0}")]
    Download(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("Failed to download image from Telegram: {0}")]
    Download(String),

    #[error("Image resize failed: {0}")]
    ResizeFailed(String),

    #[allow(dead_code)]
    #[error("EXIF extraction failed: {0}")]
    ExifFailed(String),

    #[error("Failed to save image: {0}")]
    SaveFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum UrlError {
    #[error("Failed to fetch URL: {url} — {reason}")]
    FetchFailed { url: String, reason: String },

    #[error("Failed to parse page content: {0}")]
    ParseFailed(String),

    #[error("Failed to fetch transcript for: {video_id} — {reason}")]
    TranscriptFailed { video_id: String, reason: String },

    #[error("URL fetch timed out after {timeout_secs}s: {url}")]
    Timeout { url: String, timeout_secs: u64 },

    #[error("Content too large ({size} bytes): {url}")]
    ContentTooLarge { url: String, size: usize },
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
pub enum GitError {
    #[error("Git command failed: `{command}` — {message}")]
    CommandFailed { command: String, message: String },

    #[allow(dead_code)]
    #[error("Repository not found at: {0}")]
    RepoNotFound(PathBuf),

    #[allow(dead_code)]
    #[error("Conflict detected in {file_count} file(s)")]
    ConflictDetected { file_count: usize },

    #[allow(dead_code)]
    #[error("Rebase aborted by user")]
    RebaseAborted,

    #[allow(dead_code)]
    #[error("Push failed: {0}")]
    PushFailed(String),

    #[allow(dead_code)]
    #[error("SSH authentication failed: {0}")]
    SshAuthFailed(String),
}

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

    #[error("Image processing error: {0}")]
    Image(#[from] ImageError),

    #[error("URL processing error: {0}")]
    Url(#[from] UrlError),
}

/// Convenience type alias
#[allow(dead_code)]
pub type AppResult<T> = Result<T, AppError>;
