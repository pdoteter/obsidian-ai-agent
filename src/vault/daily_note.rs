use chrono::Local;
use serde::Deserialize;
use std::path::PathBuf;
use tokio::fs;
use tracing::{info, warn};

use crate::error::VaultError;

const DEFAULT_DAILY_NOTE_FORMAT: &str = "YYYY-MM-DD";
const DEFAULT_DAILY_NOTE_FOLDER: &str = "";

const FALLBACK_TEMPLATE: &str = r#"---
date: {{date}}
tags: [daily]
---

# {{date}}

## 📝 Notes

## ✅ Todos

## 📋 Log

"#;

/// Settings deserialized from `.obsidian/daily-notes.json`
#[derive(Debug, Clone, Deserialize)]
pub struct DailyNoteSettings {
    /// Folder path relative to vault root (default: "" = vault root)
    #[serde(default)]
    pub folder: String,

    /// Date format in Moment.js syntax (default: "YYYY-MM-DD")
    #[serde(default)]
    pub format: String,

    /// Template file path relative to vault root, without .md extension (default: "" = no template)
    #[serde(default)]
    pub template: String,

    /// Whether to auto-open daily note on startup (not used by us, but present in the JSON)
    #[serde(default)]
    #[allow(dead_code)]
    pub autorun: bool,
}

impl Default for DailyNoteSettings {
    fn default() -> Self {
        Self {
            folder: DEFAULT_DAILY_NOTE_FOLDER.to_string(),
            format: DEFAULT_DAILY_NOTE_FORMAT.to_string(),
            template: String::new(),
            autorun: false,
        }
    }
}

impl DailyNoteSettings {
    /// Load daily note settings from `.obsidian/daily-notes.json` inside the vault.
    /// Returns default settings if the file doesn't exist or can't be parsed.
    pub async fn load_from_vault(vault_path: &PathBuf) -> Self {
        let config_path = vault_path.join(".obsidian").join("daily-notes.json");

        let raw = match fs::read_to_string(&config_path).await {
            Ok(content) => content,
            Err(_) => {
                info!(
                    path = %config_path.display(),
                    "daily-notes.json not found, using defaults"
                );
                return Self::default();
            }
        };

        match serde_json::from_str::<DailyNoteSettings>(&raw) {
            Ok(mut settings) => {
                // Apply defaults for empty fields (matching Obsidian behavior)
                if settings.format.trim().is_empty() {
                    settings.format = DEFAULT_DAILY_NOTE_FORMAT.to_string();
                }
                settings.folder = settings.folder.trim().to_string();
                settings.template = settings.template.trim().to_string();

                info!(
                    folder = %settings.folder,
                    format = %settings.format,
                    template = %settings.template,
                    "Loaded daily note settings from vault"
                );
                settings
            }
            Err(e) => {
                warn!(
                    error = %e,
                    path = %config_path.display(),
                    "Failed to parse daily-notes.json, using defaults"
                );
                Self::default()
            }
        }
    }

    /// Convert the Moment.js date format to a chrono format string.
    /// Obsidian uses Moment.js tokens; chrono uses strftime-style tokens.
    pub fn chrono_format(&self) -> String {
        momentjs_to_chrono(&self.format)
    }
}

/// Convert a Moment.js date format string to a chrono strftime format string.
///
/// Supports the most common tokens used in Obsidian daily note formats.
/// Tokens are replaced longest-match-first to avoid partial replacement issues
/// (e.g. "YYYY" before "YY").
fn momentjs_to_chrono(moment_fmt: &str) -> String {
    let mut result = String::with_capacity(moment_fmt.len() * 2);
    let chars: Vec<char> = moment_fmt.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Try matching from longest token to shortest
        if let Some((chrono_token, consumed)) = match_moment_token(&chars, i, len) {
            result.push_str(chrono_token);
            i += consumed;
        } else {
            // Pass through literal characters
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Try to match a Moment.js token starting at position `i`.
/// Returns (chrono_replacement, chars_consumed) or None.
fn match_moment_token(chars: &[char], i: usize, len: usize) -> Option<(&'static str, usize)> {
    let remaining = len - i;

    // 4-char tokens
    if remaining >= 4 {
        let four: String = chars[i..i + 4].iter().collect();
        match four.as_str() {
            "YYYY" => return Some(("%Y", 4)),
            "dddd" => return Some(("%A", 4)),
            "MMMM" => return Some(("%B", 4)),
            _ => {}
        }
    }

    // 3-char tokens
    if remaining >= 3 {
        let three: String = chars[i..i + 3].iter().collect();
        match three.as_str() {
            "MMM" => return Some(("%b", 3)),
            "ddd" => return Some(("%a", 3)),
            "DDD" => return Some(("%-j", 3)),
            _ => {}
        }
    }

    // 2-char tokens
    if remaining >= 2 {
        let two: String = chars[i..i + 2].iter().collect();
        match two.as_str() {
            "YY" => return Some(("%y", 2)),
            "MM" => return Some(("%m", 2)),
            "DD" => return Some(("%d", 2)),
            "dd" => return Some(("%a", 2)),  // min weekday name → abbreviated in chrono
            "HH" => return Some(("%H", 2)),
            "hh" => return Some(("%I", 2)),
            "mm" => return Some(("%M", 2)),
            "ss" => return Some(("%S", 2)),
            "Do" => return Some(("%d", 2)),  // ordinal day (1st, 2nd) → plain number in chrono
            _ => {}
        }
    }

    // 1-char tokens
    if remaining >= 1 {
        match chars[i] {
            'M' => {
                // Single M = month without leading zero
                // Only match if not followed by another M (already handled above)
                if i + 1 >= len || chars[i + 1] != 'M' {
                    return Some(("%-m", 1));
                }
            }
            'D' => {
                if i + 1 >= len || chars[i + 1] != 'D' {
                    return Some(("%-d", 1));
                }
            }
            'H' => {
                if i + 1 >= len || chars[i + 1] != 'H' {
                    return Some(("%-H", 1));
                }
            }
            'h' => {
                if i + 1 >= len || chars[i + 1] != 'h' {
                    return Some(("%-I", 1));
                }
            }
            'm' => {
                if i + 1 >= len || chars[i + 1] != 'm' {
                    return Some(("%-M", 1));
                }
            }
            's' => {
                if i + 1 >= len || chars[i + 1] != 's' {
                    return Some(("%-S", 1));
                }
            }
            'A' => return Some(("%p", 1)), // AM/PM
            'a' => return Some(("%P", 1)), // am/pm
            'X' => return Some(("%s", 1)), // unix timestamp
            _ => {}
        }
    }

    None
}

/// Manages daily note files in the Obsidian vault
pub struct DailyNoteManager {
    vault_path: PathBuf,
    settings: DailyNoteSettings,
}

impl DailyNoteManager {
    /// Create a new DailyNoteManager by loading settings from the vault's
    /// `.obsidian/daily-notes.json` configuration file.
    pub async fn new(vault_path: PathBuf) -> Self {
        let settings = DailyNoteSettings::load_from_vault(&vault_path).await;
        Self {
            vault_path,
            settings,
        }
    }

    /// Get the formatted date string for today using the configured format.
    fn format_date(&self, date: &chrono::NaiveDate) -> String {
        let chrono_fmt = self.settings.chrono_format();
        date.format(&chrono_fmt).to_string()
    }

    /// Get the folder path for daily notes (absolute).
    fn daily_notes_dir(&self) -> PathBuf {
        if self.settings.folder.is_empty() {
            self.vault_path.clone()
        } else {
            self.vault_path.join(&self.settings.folder)
        }
    }

    /// Get the path to today's daily note
    pub fn today_path(&self) -> PathBuf {
        let today = Local::now().date_naive();
        let date_str = self.format_date(&today);
        self.daily_notes_dir().join(format!("{}.md", date_str))
    }

    /// Get the path to a daily note for a specific date string (YYYY-MM-DD)
    #[allow(dead_code)]
    pub fn path_for_date(&self, date: &str) -> PathBuf {
        self.daily_notes_dir().join(format!("{}.md", date))
    }

    /// Read the template content from the configured template file.
    /// Returns None if no template is configured or the file can't be read.
    async fn read_template(&self) -> Option<String> {
        if self.settings.template.is_empty() {
            return None;
        }

        // Obsidian stores template path without .md extension
        let template_path = self
            .vault_path
            .join(&self.settings.template)
            .with_extension("md");

        match fs::read_to_string(&template_path).await {
            Ok(content) => {
                info!(
                    path = %template_path.display(),
                    "Loaded daily note template"
                );
                Some(content)
            }
            Err(e) => {
                warn!(
                    error = %e,
                    path = %template_path.display(),
                    "Failed to read template file, using fallback"
                );
                None
            }
        }
    }

    /// Ensure today's daily note exists. Creates it from template if not.
    /// Returns the path to the daily note.
    pub async fn ensure_today(&self) -> Result<PathBuf, VaultError> {
        let path = self.today_path();

        // Ensure the daily notes directory exists
        let daily_dir = self.daily_notes_dir();
        if !daily_dir.exists() {
            fs::create_dir_all(&daily_dir).await?;
            info!(dir = %daily_dir.display(), "Created daily notes directory");
        }

        // Create file from template if it doesn't exist
        if !path.exists() {
            let today = Local::now().date_naive();
            let date_str = self.format_date(&today);

            // Try configured template, fall back to built-in
            let template = self.read_template().await.unwrap_or_else(|| FALLBACK_TEMPLATE.to_string());

            // Replace Obsidian template variables
            let content = template
                .replace("{{date}}", &date_str)
                .replace("{{time}}", &Local::now().format("%H:%M").to_string())
                .replace("{{title}}", &date_str);

            fs::write(&path, &content).await?;
            info!(path = %path.display(), "Created new daily note from template");
        }

        Ok(path)
    }

    /// Append content to a specific section in the daily note.
    /// Sections are identified by their heading (e.g., "## 📝 Notes").
    pub async fn append_to_section(
        &self,
        section_heading: &str,
        content: &str,
    ) -> Result<PathBuf, VaultError> {
        let path = self.ensure_today().await?;

        let file_content = fs::read_to_string(&path).await?;

        let new_content = insert_after_heading(&file_content, section_heading, content);

        fs::write(&path, &new_content).await?;

        info!(
            path = %path.display(),
            section = section_heading,
            content_length = content.len(),
            "Appended to daily note section"
        );

        Ok(path)
    }

    /// Append content to the end of today's daily note (fallback)
    #[allow(dead_code)]
    pub async fn append(&self, content: &str) -> Result<PathBuf, VaultError> {
        let path = self.ensure_today().await?;

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await?;

        use tokio::io::AsyncWriteExt;
        file.write_all(b"\n").await?;
        file.write_all(content.as_bytes()).await?;
        file.write_all(b"\n").await?;

        info!(path = %path.display(), "Appended to daily note");

        Ok(path)
    }
}

/// Insert content after a specific heading in a markdown document.
/// If the heading is not found, append to the end.
fn insert_after_heading(document: &str, heading: &str, content: &str) -> String {
    let lines: Vec<&str> = document.lines().collect();
    let mut result = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        result.push(*line);

        if line.trim() == heading.trim() {
            // Find the next non-empty line or next heading
            // Insert content after any existing content in this section
            let mut insert_pos = i + 1;
            while insert_pos < lines.len() {
                let next_line = lines[insert_pos].trim();
                if next_line.starts_with("## ") {
                    // Hit the next section — insert before it
                    break;
                }
                result.push(lines[insert_pos]);
                insert_pos += 1;
            }

            // Add the new content
            result.push(content);
            result.push("");

            // Add remaining lines from the section we skipped
            for remaining_line in lines.iter().skip(insert_pos) {
                result.push(remaining_line);
            }

            // Skip the rest since we already added everything
            return result.join("\n");
        }
    }

    // Heading not found — append to end
    result.push("");
    result.push(content);

    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_after_heading() {
        let doc = "# Title\n\n## Notes\n\n## Todos\n\n## Log\n";
        let result = insert_after_heading(doc, "## Todos", "- [ ] Buy milk");
        assert!(result.contains("## Todos\n\n- [ ] Buy milk"));
        assert!(result.contains("## Log"));
    }

    #[test]
    fn test_insert_heading_not_found() {
        let doc = "# Title\n\nSome content\n";
        let result = insert_after_heading(doc, "## Missing", "new content");
        assert!(result.ends_with("\nnew content"));
    }

    #[test]
    fn test_momentjs_to_chrono_default_format() {
        assert_eq!(momentjs_to_chrono("YYYY-MM-DD"), "%Y-%m-%d");
    }

    #[test]
    fn test_momentjs_to_chrono_complex_format() {
        assert_eq!(momentjs_to_chrono("dddd DD-MM-YYYY"), "%A %d-%m-%Y");
    }

    #[test]
    fn test_momentjs_to_chrono_with_time() {
        assert_eq!(momentjs_to_chrono("YYYY-MM-DD HH:mm"), "%Y-%m-%d %H:%M");
    }

    #[test]
    fn test_momentjs_to_chrono_short_year() {
        assert_eq!(momentjs_to_chrono("DD/MM/YY"), "%d/%m/%y");
    }

    #[test]
    fn test_momentjs_to_chrono_month_names() {
        assert_eq!(momentjs_to_chrono("MMMM DD, YYYY"), "%B %d, %Y");
        assert_eq!(momentjs_to_chrono("MMM DD"), "%b %d");
    }

    #[test]
    fn test_momentjs_to_chrono_no_leading_zero() {
        assert_eq!(momentjs_to_chrono("M/D/YYYY"), "%-m/%-d/%Y");
    }

    #[test]
    fn test_default_settings() {
        let settings = DailyNoteSettings::default();
        assert_eq!(settings.folder, "");
        assert_eq!(settings.format, "YYYY-MM-DD");
        assert_eq!(settings.template, "");
        assert!(!settings.autorun);
    }

    #[test]
    fn test_chrono_format_default() {
        let settings = DailyNoteSettings::default();
        assert_eq!(settings.chrono_format(), "%Y-%m-%d");
    }
}
