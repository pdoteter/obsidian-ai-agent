use std::env;
use std::path::PathBuf;

use serde::Deserialize;

use crate::vault::daily_note::momentjs_to_chrono;

/// Settings loaded from the YAML config file (non-secret values).
#[derive(Debug, Deserialize)]
struct FileConfig {
    vault_path: String,

    #[serde(default)]
    git: GitConfig,

    #[serde(default)]
    ai: AiConfig,

    #[serde(default)]
    access: AccessConfig,

    #[serde(default = "default_timezone")]
    timezone: String,

    #[serde(default = "default_log_level")]
    log_level: String,

    /// Moment.js format for {{date}} in daily note templates (default: "YYYY/MM/DD")
    #[serde(default = "default_date_display_format")]
    date_display_format: String,
}

#[derive(Debug, Deserialize)]
struct GitConfig {
    #[serde(default = "default_true")]
    sync_enabled: bool,

    path: Option<String>,

    #[serde(default = "default_git_remote")]
    remote_name: String,

    #[serde(default = "default_git_branch")]
    branch: String,

    ssh_key_path: Option<String>,

    #[serde(default = "default_debounce_secs")]
    sync_debounce_secs: u64,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            sync_enabled: true,
            path: None,
            remote_name: "origin".to_string(),
            branch: "main".to_string(),
            ssh_key_path: None,
            sync_debounce_secs: 300,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AiConfig {
    #[serde(default = "default_whisper_model")]
    whisper_model: String,

    whisper_language: Option<String>,

    #[serde(default = "default_classify_model")]
    classify_model: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            whisper_model: "whisper-1".to_string(),
            whisper_language: None,
            classify_model: "google/gemini-2.5-flash".to_string(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct AccessConfig {
    #[serde(default)]
    allowed_user_ids: Vec<u64>,
}

// Serde default helpers
fn default_true() -> bool {
    true
}
fn default_timezone() -> String {
    "Europe/Brussels".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_git_remote() -> String {
    "origin".to_string()
}
fn default_git_branch() -> String {
    "main".to_string()
}
fn default_debounce_secs() -> u64 {
    300
}
fn default_whisper_model() -> String {
    "whisper-1".to_string()
}
fn default_classify_model() -> String {
    "google/gemini-2.5-flash".to_string()
}
fn default_date_display_format() -> String {
    "YYYY/MM/DD".to_string()
}

/// Runtime config used by the application. Built from YAML file + env-var secrets.
#[derive(Debug, Clone)]
pub struct Config {
    pub teloxide_token: String,
    pub openrouter_api_key: String,
    pub openai_api_key: String,
    pub vault_path: PathBuf,
    pub git_sync_enabled: bool,
    pub git_path: Option<PathBuf>,
    pub git_ssh_key_path: Option<PathBuf>,
    pub git_remote_name: String,
    pub git_branch: String,
    pub git_sync_debounce_secs: u64,
    pub whisper_model: String,
    pub whisper_language: Option<String>,
    pub openrouter_model_classify: String,
    pub allowed_user_ids: Vec<u64>,
    pub timezone: String,
    /// Chrono strftime format for {{date}} in daily note templates
    pub date_display_format: String,
}

impl Config {
    /// Load configuration from YAML file (settings) + environment variables (secrets).
    ///
    /// Config file path is resolved from `CONFIG_PATH` env var, defaulting to `./config.yaml`.
    pub fn load() -> Result<Self, ConfigError> {
        // Read secrets from environment
        let teloxide_token =
            env::var("TELOXIDE_TOKEN").map_err(|_| ConfigError::Missing("TELOXIDE_TOKEN"))?;
        let openrouter_api_key = env::var("OPENROUTER_API_KEY")
            .map_err(|_| ConfigError::Missing("OPENROUTER_API_KEY"))?;
        let openai_api_key =
            env::var("OPENAI_API_KEY").map_err(|_| ConfigError::Missing("OPENAI_API_KEY"))?;

        // Load settings from YAML config file
        let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "config.yaml".to_string());
        let config_path = PathBuf::from(&config_path);

        let yaml_content = std::fs::read_to_string(&config_path)
            .map_err(|e| ConfigError::FileRead(config_path.clone(), e.to_string()))?;

        let file: FileConfig = serde_yml::from_str(&yaml_content)
            .map_err(|e| ConfigError::Parse(config_path.clone(), e.to_string()))?;

        // Validate vault path
        let vault_path = PathBuf::from(&file.vault_path);
        if !vault_path.exists() {
            return Err(ConfigError::InvalidPath(vault_path));
        }

        // Validate git path when sync is enabled
        let git_path = file.git.path.map(PathBuf::from);
        if file.git.sync_enabled && git_path.is_none() {
            return Err(ConfigError::MissingSetting(
                "git.path (required when git.sync_enabled is true)",
            ));
        }

        let git_ssh_key_path = file.git.ssh_key_path.map(PathBuf::from);

        // Set timezone for chrono::Local
        env::set_var("TZ", &file.timezone);

        // Set log level so tracing picks it up
        if env::var("RUST_LOG").is_err() {
            env::set_var("RUST_LOG", &file.log_level);
        }

        Ok(Config {
            teloxide_token,
            openrouter_api_key,
            openai_api_key,
            vault_path,
            git_sync_enabled: file.git.sync_enabled,
            git_path,
            git_ssh_key_path,
            git_remote_name: file.git.remote_name,
            git_branch: file.git.branch,
            git_sync_debounce_secs: file.git.sync_debounce_secs,
            whisper_model: file.ai.whisper_model,
            whisper_language: file.ai.whisper_language.filter(|v| !v.is_empty()),
            openrouter_model_classify: file.ai.classify_model,
            allowed_user_ids: file.access.allowed_user_ids,
            timezone: file.timezone,
            date_display_format: momentjs_to_chrono(&file.date_display_format),
        })
    }

    /// Check if a user is allowed (empty list = allow all)
    pub fn is_user_allowed(&self, user_id: u64) -> bool {
        self.allowed_user_ids.is_empty() || self.allowed_user_ids.contains(&user_id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Missing required environment variable: {0}")]
    Missing(&'static str),

    #[error("Missing required config setting: {0}")]
    MissingSetting(&'static str),

    #[error("Failed to read config file {0}: {1}")]
    FileRead(PathBuf, String),

    #[error("Failed to parse config file {0}: {1}")]
    Parse(PathBuf, String),

    #[error("Invalid path: {0} does not exist")]
    InvalidPath(PathBuf),
}
