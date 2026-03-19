use std::env;
use std::path::PathBuf;

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
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let teloxide_token =
            env::var("TELOXIDE_TOKEN").map_err(|_| ConfigError::Missing("TELOXIDE_TOKEN"))?;

        let openrouter_api_key = env::var("OPENROUTER_API_KEY")
            .map_err(|_| ConfigError::Missing("OPENROUTER_API_KEY"))?;

        let vault_path = env::var("VAULT_PATH")
            .map(PathBuf::from)
            .map_err(|_| ConfigError::Missing("VAULT_PATH"))?;

        if !vault_path.exists() {
            return Err(ConfigError::InvalidPath(vault_path));
        }

        let git_sync_enabled = env::var("GIT_SYNC_ENABLED")
            .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no"))
            .unwrap_or(true); // Enabled by default for backward compat

        let git_path = if git_sync_enabled {
            Some(env::var("GIT_PATH").map(PathBuf::from).map_err(|_| {
                ConfigError::Missing("GIT_PATH (required when GIT_SYNC_ENABLED=true)")
            })?)
        } else {
            env::var("GIT_PATH").ok().map(PathBuf::from)
        };

        let git_ssh_key_path = env::var("GIT_SSH_KEY_PATH").ok().map(PathBuf::from);

        let git_remote_name = env::var("GIT_REMOTE_NAME").unwrap_or_else(|_| "origin".to_string());

        let git_branch = env::var("GIT_BRANCH").unwrap_or_else(|_| "main".to_string());

        let git_sync_debounce_secs = env::var("GIT_SYNC_DEBOUNCE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300); // 5 minutes default

        let openai_api_key =
            env::var("OPENAI_API_KEY").map_err(|_| ConfigError::Missing("OPENAI_API_KEY"))?;

        let whisper_model = env::var("WHISPER_MODEL").unwrap_or_else(|_| "whisper-1".to_string());

        let whisper_language = env::var("WHISPER_LANGUAGE").ok().filter(|v| !v.is_empty());

        let openrouter_model_classify = env::var("OPENROUTER_MODEL_CLASSIFY")
            .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());

        let allowed_user_ids = env::var("ALLOWED_USER_IDS")
            .ok()
            .map(|ids| {
                ids.split(',')
                    .filter_map(|id| id.trim().parse::<u64>().ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Config {
            teloxide_token,
            openrouter_api_key,
            openai_api_key,
            vault_path,
            git_sync_enabled,
            git_path,
            git_ssh_key_path,
            git_remote_name,
            git_branch,
            git_sync_debounce_secs,
            whisper_model,
            whisper_language,
            openrouter_model_classify,
            allowed_user_ids,
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

    #[error("Invalid path: {0} does not exist")]
    InvalidPath(PathBuf),
}
