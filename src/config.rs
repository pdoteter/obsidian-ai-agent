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
    pub whisper_model: String,
    pub whisper_language: Option<String>,
    pub openrouter_model_classify: String,
    pub allowed_user_ids: Vec<u64>,
    pub timezone: String,
    /// Chrono strftime format for {{date}} in daily note templates
    pub date_display_format: String,
    pub guide_path: Option<PathBuf>,
    pub image: ImageConfig,
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
            guide_path,
            image: file.image,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
