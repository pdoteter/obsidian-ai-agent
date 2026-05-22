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

    #[serde(default = "default_guide_path")]
    guide_path: Option<PathBuf>,

    #[serde(default)]
    image: ImageConfig,

    #[serde(default)]
    url: UrlConfig,

    #[serde(default)]
    ack: AckConfig,

    #[serde(default)]
    finance: FinanceConfig,
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

/// URL handling configuration settings
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TranscriptionConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub language: Option<String>,
}

/// AI configuration settings for a specific task
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TaskAiConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AiConfig {
    #[serde(default = "default_provider")]
    provider: String,

    #[serde(default = "default_whisper_model")]
    whisper_model: String,

    whisper_language: Option<String>,

    #[serde(default = "default_classify_model")]
    classify_model: String,

    #[serde(default)]
    transcription: TranscriptionConfig,

    #[serde(default)]
    classification: TaskAiConfig,

    #[serde(default)]
    summarization: TaskAiConfig,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            whisper_model: default_whisper_model(),
            whisper_language: None,
            classify_model: default_classify_model(),
            transcription: TranscriptionConfig::default(),
            classification: TaskAiConfig::default(),
            summarization: TaskAiConfig::default(),
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
fn default_provider() -> String {
    "openrouter".to_string()
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
fn default_guide_path() -> Option<PathBuf> {
    Some(PathBuf::from("./system-guide.md"))
}

/// Image configuration settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ImageConfig {
    pub max_dimension: u32,
    pub jpeg_quality: u8,
    pub assets_folder: String,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            max_dimension: 1280,
            jpeg_quality: 85,
            assets_folder: "assets".to_string(),
        }
    }
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
    pub ai_provider: String,
    pub whisper_model: String,
    pub whisper_language: Option<String>,
    pub openrouter_model_classify: String,
    pub transcription: TranscriptionConfig,
    pub classification: TaskAiConfig,
    pub summarization: TaskAiConfig,
    pub allowed_user_ids: Vec<u64>,
    pub timezone: String,
    /// Chrono strftime format for {{date}} in daily note templates
    pub date_display_format: String,
    pub guide_path: Option<PathBuf>,
    pub image: ImageConfig,
    pub url: UrlConfig,
    pub ack: AckConfig,
    pub finance: FinanceConfig,
    pub finance_teloxide_token: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            teloxide_token: String::new(),
            openrouter_api_key: String::new(),
            openai_api_key: String::new(),
            vault_path: PathBuf::from("."),
            git_sync_enabled: false,
            git_path: None,
            git_ssh_key_path: None,
            git_remote_name: "origin".to_string(),
            git_branch: "main".to_string(),
            git_sync_debounce_secs: 60,
            ai_provider: "openrouter".to_string(),
            whisper_model: "whisper-1".to_string(),
            whisper_language: None,
            openrouter_model_classify: "google/gemini-2.5-flash".to_string(),
            transcription: TranscriptionConfig::default(),
            classification: TaskAiConfig::default(),
            summarization: TaskAiConfig::default(),
            allowed_user_ids: Vec::new(),
            timezone: "UTC".to_string(),
            date_display_format: "%Y-%m-%d".to_string(),
            guide_path: None,
            image: ImageConfig::default(),
            url: UrlConfig::default(),
            ack: AckConfig::default(),
            finance: FinanceConfig::default(),
            finance_teloxide_token: None,
        }
    }
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

        // Extract config directory for resolving relative paths (e.g., guide_path)
        let config_dir = config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

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

        // Resolve guide_path relative to config directory if relative
        let guide_path = file.guide_path.map(|p| {
            if p.is_absolute() {
                p
            } else {
                config_dir.join(p)
            }
        });

        let mut finance = file.finance.clone();
        finance.guide_path = file.finance.guide_path.map(|p| {
            if p.is_absolute() {
                p
            } else {
                config_dir.join(p)
            }
        });

        // Read finance secret from environment if enabled
        let finance_teloxide_token = if file.finance.enabled {
            Some(
                env::var("FINANCE_TELOXIDE_TOKEN")
                    .map_err(|_| ConfigError::Missing("FINANCE_TELOXIDE_TOKEN"))?,
            )
        } else {
            env::var("FINANCE_TELOXIDE_TOKEN").ok()
        };

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
            ai_provider: file.ai.provider,
            whisper_model: file.ai.whisper_model,
            whisper_language: file.ai.whisper_language.filter(|v| !v.is_empty()),
            openrouter_model_classify: file.ai.classify_model,
            transcription: file.ai.transcription,
            classification: file.ai.classification,
            summarization: file.ai.summarization,
            allowed_user_ids: file.access.allowed_user_ids,
            timezone: file.timezone,
            date_display_format: momentjs_to_chrono(&file.date_display_format),
            guide_path,
            image: file.image,
            url: file.url,
            ack: file.ack,
            finance,
            finance_teloxide_token,
        })
    }

    /// Check if a user is allowed (empty list = allow all)
    pub fn is_user_allowed(&self, user_id: u64) -> bool {
        self.allowed_user_ids.is_empty() || self.allowed_user_ids.contains(&user_id)
    }

    /// Check if a user is allowed on the finance bot (falls back to general allowed users if empty)
    pub fn is_finance_user_allowed(&self, user_id: u64) -> bool {
        if self.finance.allowed_user_ids.is_empty() {
            self.is_user_allowed(user_id)
        } else {
            self.finance.allowed_user_ids.contains(&user_id)
        }
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

/// URL handling configuration settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UrlConfig {
    pub transcript_folder: String,
    pub fetch_timeout_secs: u64,
    pub max_content_bytes: usize,
    pub max_urls_per_message: usize,
}

impl Default for UrlConfig {
    fn default() -> Self {
        Self {
            transcript_folder: "transcripts".to_string(),
            fetch_timeout_secs: 15,
            max_content_bytes: 524288,
            max_urls_per_message: 5,
        }
    }
}

/// Telegram acknowledgement settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AckConfig {
    pub log_mode: LogAckMode,
    pub log_text: String,
    pub reaction_emoji: String,
}

impl Default for AckConfig {
    fn default() -> Self {
        Self {
            log_mode: LogAckMode::Reaction,
            log_text: "Done 👍".to_string(),
            reaction_emoji: "👍".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogAckMode {
    Reaction,
    Text,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct FinanceConfig {
    pub enabled: bool,
    pub folder: String,
    pub assets_folder: String,
    pub guide_path: Option<PathBuf>,
    pub allowed_user_ids: Vec<u64>,
}

impl Default for FinanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            folder: "Finance".to_string(),
            assets_folder: "Assets".to_string(),
            guide_path: None,
            allowed_user_ids: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_url_config_defaults() {
        // Verify UrlConfig::default() has correct default values
        let url_config = UrlConfig::default();
        assert_eq!(url_config.transcript_folder, "transcripts");
        assert_eq!(url_config.fetch_timeout_secs, 15);
        assert_eq!(url_config.max_content_bytes, 524288);
        assert_eq!(url_config.max_urls_per_message, 5);
    }

    #[test]
    fn test_custom_url_config() {
        // Deserialize config with custom url section → verify fields
        let file_config: FileConfig = serde_yml::from_str(
            "vault_path: /tmp/vault\nurl:\n  transcript_folder: custom_transcripts\n  fetch_timeout_secs: 30\n  max_content_bytes: 1048576\n  max_urls_per_message: 10\n"
        ).unwrap();

        assert_eq!(file_config.url.transcript_folder, "custom_transcripts");
        assert_eq!(file_config.url.fetch_timeout_secs, 30);
        assert_eq!(file_config.url.max_content_bytes, 1048576);
        assert_eq!(file_config.url.max_urls_per_message, 10);
    }

    #[test]
    fn test_ack_config_defaults() {
        let file_config: FileConfig = serde_yml::from_str("vault_path: /tmp/vault\n").unwrap();

        assert_eq!(file_config.ack.log_mode, LogAckMode::Reaction);
        assert_eq!(file_config.ack.log_text, "Done 👍");
        assert_eq!(file_config.ack.reaction_emoji, "👍");
    }

    #[test]
    fn test_custom_ack_config() {
        let file_config: FileConfig = serde_yml::from_str(
            "vault_path: /tmp/vault\nack:\n  log_mode: text\n  log_text: Logged 👍\n  reaction_emoji: ✅\n",
        )
        .unwrap();

        assert_eq!(file_config.ack.log_mode, LogAckMode::Text);
        assert_eq!(file_config.ack.log_text, "Logged 👍");
        assert_eq!(file_config.ack.reaction_emoji, "✅");
    }

    #[test]
    fn test_backward_compat_missing_url_section() {
        // Deserialize config WITHOUT url section → verify defaults applied, no errors
        let file_config: FileConfig = serde_yml::from_str("vault_path: /tmp/vault\n").unwrap();

        // url section should have defaults
        assert_eq!(file_config.url.transcript_folder, "transcripts");
        assert_eq!(file_config.url.fetch_timeout_secs, 15);
        assert_eq!(file_config.url.max_content_bytes, 524288);
        assert_eq!(file_config.url.max_urls_per_message, 5);
    }

    #[test]
    fn test_default_guide_path() {
        // Verify Config::default() has guide_path = Some("./system-guide.md")
        let file_config: FileConfig = serde_yml::from_str("vault_path: /tmp/vault\n").unwrap();

        assert!(file_config.guide_path.is_some());
        assert_eq!(
            file_config.guide_path.as_ref().unwrap(),
            &PathBuf::from("./system-guide.md")
        );
    }

    #[test]
    fn test_custom_guide_path() {
        // Deserialize config with guide_path: "/custom/path.md" → verify field set
        let file_config: FileConfig =
            serde_yml::from_str("vault_path: /tmp/vault\nguide_path: /custom/path.md\n").unwrap();

        assert!(file_config.guide_path.is_some());
        assert_eq!(
            file_config.guide_path.as_ref().unwrap(),
            &PathBuf::from("/custom/path.md")
        );
    }

    #[test]
    fn test_guide_path_none() {
        // Deserialize config with guide_path: null → verify None
        let file_config: FileConfig =
            serde_yml::from_str("vault_path: /tmp/vault\nguide_path: null\n").unwrap();

        assert!(file_config.guide_path.is_none());
    }

    #[test]
    fn test_image_config_defaults() {
        // Verify ImageConfig::default() has max_dimension=1280, jpeg_quality=85, assets_folder="assets"
        let image_config = ImageConfig::default();
        assert_eq!(image_config.max_dimension, 1280);
        assert_eq!(image_config.jpeg_quality, 85);
        assert_eq!(image_config.assets_folder, "assets");
    }

    #[test]
    fn test_custom_image_config() {
        // Deserialize config with custom image section → verify fields
        let file_config: FileConfig = serde_yml::from_str(
            "vault_path: /tmp/vault\nimage:\n  max_dimension: 2048\n  jpeg_quality: 90\n  assets_folder: my_assets\n"
        ).unwrap();

        assert_eq!(file_config.image.max_dimension, 2048);
        assert_eq!(file_config.image.jpeg_quality, 90);
        assert_eq!(file_config.image.assets_folder, "my_assets");
    }

    #[test]
    fn test_backward_compat_missing_guide_and_image() {
        // Deserialize config WITHOUT guide_path or image fields → verify defaults applied, no errors
        let file_config: FileConfig = serde_yml::from_str("vault_path: /tmp/vault\n").unwrap();

        // guide_path should have default
        assert!(file_config.guide_path.is_some());
        assert_eq!(
            file_config.guide_path.as_ref().unwrap(),
            &PathBuf::from("./system-guide.md")
        );

        // image should have defaults
        assert_eq!(file_config.image.max_dimension, 1280);
        assert_eq!(file_config.image.jpeg_quality, 85);
        assert_eq!(file_config.image.assets_folder, "assets");
    }

    #[test]
    #[serial]
    fn test_guide_path_resolves_relative_to_config() {
        // Verify guide_path is resolved relative to config file directory when relative
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let config_subdir = temp_dir.path().join("subdir");
        std::fs::create_dir_all(&config_subdir).unwrap();
        let config_file = config_subdir.join("config.yaml");
        let vault_dir = temp_dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();

        std::fs::write(
            &config_file,
            format!(
                r#"vault_path: {}
guide_path: ./my-guide.md
git:
  sync_enabled: false
"#,
                vault_dir.display()
            ),
        )
        .unwrap();

        // Clear any previous CONFIG_PATH
        let old_config_path = env::var("CONFIG_PATH").ok();
        env::set_var("CONFIG_PATH", config_file.to_str().unwrap());
        env::set_var("TELOXIDE_TOKEN", "test_token");
        env::set_var("OPENROUTER_API_KEY", "test_key");
        env::set_var("OPENAI_API_KEY", "test_key");

        let config = Config::load().unwrap();

        // Should resolve to config_file.parent() / my-guide.md
        let expected = config_subdir.join("my-guide.md");
        assert_eq!(config.guide_path, Some(expected));

        // Restore old state
        env::remove_var("CONFIG_PATH");
        env::remove_var("TELOXIDE_TOKEN");
        env::remove_var("OPENROUTER_API_KEY");
        env::remove_var("OPENAI_API_KEY");
        if let Some(path) = old_config_path {
            env::set_var("CONFIG_PATH", path);
        }
    }

    #[test]
    #[serial]
    fn test_guide_path_absolute_unchanged() {
        // Verify absolute guide_path values are not modified
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let config_file = temp_dir.path().join("config.yaml");
        let vault_dir = temp_dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();

        // Create an absolute path (works on all platforms)
        let abs_guide_path = temp_dir.path().join("absolute_guide.md");

        std::fs::write(
            &config_file,
            format!(
                r#"vault_path: {}
guide_path: {}
git:
  sync_enabled: false
"#,
                vault_dir.display(),
                abs_guide_path.display()
            ),
        )
        .unwrap();

        // Clear any previous CONFIG_PATH
        let old_config_path = env::var("CONFIG_PATH").ok();
        env::set_var("CONFIG_PATH", config_file.to_str().unwrap());
        env::set_var("TELOXIDE_TOKEN", "test_token");
        env::set_var("OPENROUTER_API_KEY", "test_key");
        env::set_var("OPENAI_API_KEY", "test_key");

        let config = Config::load().unwrap();

        // Should remain unchanged (absolute)
        assert_eq!(config.guide_path, Some(abs_guide_path));

        // Restore old state
        env::remove_var("CONFIG_PATH");
        env::remove_var("TELOXIDE_TOKEN");
        env::remove_var("OPENROUTER_API_KEY");
        env::remove_var("OPENAI_API_KEY");
        if let Some(path) = old_config_path {
            env::set_var("CONFIG_PATH", path);
        }
    }

    fn setup_env() -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let old_config = env::var("CONFIG_PATH").ok();
        let old_teloxide = env::var("TELOXIDE_TOKEN").ok();
        let old_openrouter = env::var("OPENROUTER_API_KEY").ok();
        let old_openai = env::var("OPENAI_API_KEY").ok();

        env::remove_var("CONFIG_PATH");
        env::remove_var("TELOXIDE_TOKEN");
        env::remove_var("OPENROUTER_API_KEY");
        env::remove_var("OPENAI_API_KEY");

        (old_config, old_teloxide, old_openrouter, old_openai)
    }

    fn restore_env(
        old_env: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    ) {
        let (old_config, old_teloxide, old_openrouter, old_openai) = old_env;
        match old_config {
            Some(v) => env::set_var("CONFIG_PATH", v),
            None => env::remove_var("CONFIG_PATH"),
        }
        match old_teloxide {
            Some(v) => env::set_var("TELOXIDE_TOKEN", v),
            None => env::remove_var("TELOXIDE_TOKEN"),
        }
        match old_openrouter {
            Some(v) => env::set_var("OPENROUTER_API_KEY", v),
            None => env::remove_var("OPENROUTER_API_KEY"),
        }
        match old_openai {
            Some(v) => env::set_var("OPENAI_API_KEY", v),
            None => env::remove_var("OPENAI_API_KEY"),
        }
    }

    #[test]
    #[serial]
    fn test_config_load_success() -> Result<(), String> {
        use tempfile::tempdir;
        let temp_dir = tempdir().map_err(|e| e.to_string())?;
        let config_file = temp_dir.path().join("config.yaml");
        let vault_dir = temp_dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir).map_err(|e| e.to_string())?;

        std::fs::write(
            &config_file,
            format!(
                r#"vault_path: {}
git:
  sync_enabled: false
"#,
                vault_dir.display()
            ),
        )
        .map_err(|e| e.to_string())?;

        let old_env = setup_env();
        env::set_var("CONFIG_PATH", config_file.to_str().unwrap());
        env::set_var("TELOXIDE_TOKEN", "test_teloxide");
        env::set_var("OPENROUTER_API_KEY", "test_openrouter");
        env::set_var("OPENAI_API_KEY", "test_openai");

        let result = Config::load();

        restore_env(old_env);

        let config = result.map_err(|e| format!("Failed to load config: {:?}", e))?;
        assert_eq!(config.teloxide_token, "test_teloxide");
        assert_eq!(config.openrouter_api_key, "test_openrouter");
        assert_eq!(config.openai_api_key, "test_openai");
        assert_eq!(config.vault_path, vault_dir);

        Ok(())
    }

    #[test]
    #[serial]
    fn test_config_load_missing_env_vars() {
        let old_env = setup_env();

        // Test missing TELOXIDE_TOKEN
        let result = Config::load();
        assert!(matches!(
            result,
            Err(ConfigError::Missing("TELOXIDE_TOKEN"))
        ));

        // Test missing OPENROUTER_API_KEY
        env::set_var("TELOXIDE_TOKEN", "test_teloxide");
        let result = Config::load();
        assert!(matches!(
            result,
            Err(ConfigError::Missing("OPENROUTER_API_KEY"))
        ));

        // Test missing OPENAI_API_KEY
        env::set_var("OPENROUTER_API_KEY", "test_openrouter");
        let result = Config::load();
        assert!(matches!(
            result,
            Err(ConfigError::Missing("OPENAI_API_KEY"))
        ));

        restore_env(old_env);
    }

    #[test]
    #[serial]
    fn test_config_load_missing_config_file() -> Result<(), String> {
        let old_env = setup_env();
        env::set_var("TELOXIDE_TOKEN", "test_teloxide");
        env::set_var("OPENROUTER_API_KEY", "test_openrouter");
        env::set_var("OPENAI_API_KEY", "test_openai");

        // Set CONFIG_PATH to a non-existent file
        env::set_var("CONFIG_PATH", "/does/not/exist/config.yaml");

        let result = Config::load();

        restore_env(old_env);

        match result {
            Err(ConfigError::FileRead(path, _)) => {
                assert_eq!(path, PathBuf::from("/does/not/exist/config.yaml"));
                Ok(())
            }
            _ => Err("Expected ConfigError::FileRead".to_string()),
        }
    }

    #[test]
    #[serial]
    fn test_config_load_invalid_config_yaml() -> Result<(), String> {
        use tempfile::tempdir;
        let temp_dir = tempdir().map_err(|e| e.to_string())?;
        let config_file = temp_dir.path().join("config.yaml");

        // Write invalid YAML
        std::fs::write(&config_file, "invalid: yaml: content:").map_err(|e| e.to_string())?;

        let old_env = setup_env();
        env::set_var("CONFIG_PATH", config_file.to_str().unwrap());
        env::set_var("TELOXIDE_TOKEN", "test_teloxide");
        env::set_var("OPENROUTER_API_KEY", "test_openrouter");
        env::set_var("OPENAI_API_KEY", "test_openai");

        let result = Config::load();

        restore_env(old_env);

        match result {
            Err(ConfigError::Parse(path, _)) => {
                assert_eq!(path, config_file);
                Ok(())
            }
            _ => Err("Expected ConfigError::Parse".to_string()),
        }
    }

    #[test]
    #[serial]
    fn test_config_load_invalid_vault_path() -> Result<(), String> {
        use tempfile::tempdir;
        let temp_dir = tempdir().map_err(|e| e.to_string())?;
        let config_file = temp_dir.path().join("config.yaml");
        let non_existent_vault = temp_dir.path().join("non_existent_vault");

        std::fs::write(
            &config_file,
            format!(
                r#"vault_path: {}
git:
  sync_enabled: false
"#,
                non_existent_vault.display()
            ),
        )
        .map_err(|e| e.to_string())?;

        let old_env = setup_env();
        env::set_var("CONFIG_PATH", config_file.to_str().unwrap());
        env::set_var("TELOXIDE_TOKEN", "test_teloxide");
        env::set_var("OPENROUTER_API_KEY", "test_openrouter");
        env::set_var("OPENAI_API_KEY", "test_openai");

        let result = Config::load();

        restore_env(old_env);

        match result {
            Err(ConfigError::InvalidPath(path)) => {
                assert_eq!(path, non_existent_vault);
                Ok(())
            }
            _ => Err("Expected ConfigError::InvalidPath".to_string()),
        }
    }

    #[test]
    #[serial]
    fn test_config_load_missing_git_path_when_sync_enabled() -> Result<(), String> {
        use tempfile::tempdir;
        let temp_dir = tempdir().map_err(|e| e.to_string())?;
        let config_file = temp_dir.path().join("config.yaml");
        let vault_dir = temp_dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir).map_err(|e| e.to_string())?;

        std::fs::write(
            &config_file,
            format!(
                r#"vault_path: {}
git:
  sync_enabled: true
"#,
                vault_dir.display()
            ),
        )
        .map_err(|e| e.to_string())?;

        let old_env = setup_env();
        env::set_var("CONFIG_PATH", config_file.to_str().unwrap());
        env::set_var("TELOXIDE_TOKEN", "test_teloxide");
        env::set_var("OPENROUTER_API_KEY", "test_openrouter");
        env::set_var("OPENAI_API_KEY", "test_openai");

        let result = Config::load();

        restore_env(old_env);

        match result {
            Err(ConfigError::MissingSetting(
                "git.path (required when git.sync_enabled is true)",
            )) => Ok(()),
            _ => Err("Expected ConfigError::MissingSetting".to_string()),
        }
    }

    #[test]
    fn test_finance_config_defaults() {
        let finance_config = FinanceConfig::default();
        assert!(!finance_config.enabled);
        assert_eq!(finance_config.folder, "Finance");
        assert_eq!(finance_config.assets_folder, "Assets");
        assert!(finance_config.guide_path.is_none());
        assert!(finance_config.allowed_user_ids.is_empty());
    }

    #[test]
    fn test_custom_finance_config() {
        let file_config: FileConfig = serde_yml::from_str(
            "vault_path: /tmp/vault\nfinance:\n  enabled: true\n  folder: custom_finance\n  assets_folder: custom_assets\n  guide_path: /path/to/finance-guide.md\n  allowed_user_ids:\n    - 98765\n"
        ).unwrap();

        assert!(file_config.finance.enabled);
        assert_eq!(file_config.finance.folder, "custom_finance");
        assert_eq!(file_config.finance.assets_folder, "custom_assets");
        assert_eq!(
            file_config.finance.guide_path,
            Some(PathBuf::from("/path/to/finance-guide.md"))
        );
        assert_eq!(file_config.finance.allowed_user_ids, vec![98765]);
    }

    #[test]
    fn test_is_finance_user_allowed() {
        let mut config = Config::default();
        config.allowed_user_ids = vec![123, 456];
        config.finance.allowed_user_ids = vec![];

        // Empty finance.allowed_user_ids falls back to global allowed_user_ids
        assert!(config.is_finance_user_allowed(123));
        assert!(config.is_finance_user_allowed(456));
        assert!(!config.is_finance_user_allowed(789));

        // When finance.allowed_user_ids is explicitly set, it overrides the global list
        config.finance.allowed_user_ids = vec![789];
        assert!(!config.is_finance_user_allowed(123));
        assert!(config.is_finance_user_allowed(789));
    }

    #[test]
    #[serial]
    fn test_finance_token_required_when_enabled() -> Result<(), String> {
        use tempfile::tempdir;
        let temp_dir = tempdir().map_err(|e| e.to_string())?;
        let config_file = temp_dir.path().join("config.yaml");
        let vault_dir = temp_dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir).map_err(|e| e.to_string())?;

        std::fs::write(
            &config_file,
            format!(
                r#"vault_path: {}
finance:
  enabled: true
git:
  sync_enabled: false
"#,
                vault_dir.display()
            ),
        )
        .map_err(|e| e.to_string())?;

        let old_env = setup_env();
        let old_finance_token = env::var("FINANCE_TELOXIDE_TOKEN").ok();
        env::remove_var("FINANCE_TELOXIDE_TOKEN");

        env::set_var("CONFIG_PATH", config_file.to_str().unwrap());
        env::set_var("TELOXIDE_TOKEN", "test_teloxide");
        env::set_var("OPENROUTER_API_KEY", "test_openrouter");
        env::set_var("OPENAI_API_KEY", "test_openai");

        // Should fail due to missing FINANCE_TELOXIDE_TOKEN
        let result = Config::load();

        restore_env(old_env);
        if let Some(t) = old_finance_token {
            env::set_var("FINANCE_TELOXIDE_TOKEN", t);
        } else {
            env::remove_var("FINANCE_TELOXIDE_TOKEN");
        }

        match result {
            Err(ConfigError::Missing("FINANCE_TELOXIDE_TOKEN")) => Ok(()),
            _ => Err("Expected ConfigError::Missing(FINANCE_TELOXIDE_TOKEN)".to_string()),
        }
    }
}
