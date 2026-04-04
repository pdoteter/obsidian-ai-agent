use chrono::Local;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info, warn};

use crate::git::debounce::SyncNotifier;
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
    pub async fn load_from_vault(vault_path: &Path) -> Self {
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
pub(crate) fn momentjs_to_chrono(moment_fmt: &str) -> String {
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
    /// Chrono strftime format for {{date}}/{{title}} in templates
    date_display_format: String,
    sync_notifier: Option<SyncNotifier>,
}

impl DailyNoteManager {
    /// Create a new DailyNoteManager by loading settings from the vault's
    /// `.obsidian/daily-notes.json` configuration file.
    ///
    /// `date_display_format` is a chrono strftime string used for `{{date}}`/`{{title}}`
    /// in daily note templates.
    pub async fn new(
        vault_path: PathBuf,
        date_display_format: String,
        sync_notifier: Option<SyncNotifier>,
    ) -> Self {
        let settings = DailyNoteSettings::load_from_vault(&vault_path).await;
        Self {
            vault_path,
            settings,
            date_display_format,
            sync_notifier,
        }
    }

    async fn sync_before_write_if_idle(&self) {
        let Some(sync_notifier) = self.sync_notifier.as_ref() else {
            return;
        };

        match sync_notifier.pull_if_idle().await {
            Ok(Some(result)) => {
                info!(result = %result, "Completed pre-write git pull");
            }
            Ok(None) => {
                debug!("Skipping pre-write git pull because debounce worker is busy");
            }
            Err(error) => {
                warn!(error = %error, "Pre-write git pull failed, continuing with local write");
            }
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
        self.sync_before_write_if_idle().await;

        let path = self.today_path();

        // Ensure the full parent directory for today's note exists.
        // The configured date format can include path separators like YYYY/MM/YYYY-MM-DD,
        // which means the resolved note path may be nested deeper than the configured folder.
        if let Some(parent_dir) = path.parent() {
            if !parent_dir.exists() {
                fs::create_dir_all(parent_dir).await?;
                info!(dir = %parent_dir.display(), "Created daily note parent directory");
            }
        }

        // Create file from template if it doesn't exist
        if !path.exists() {
            let today = Local::now().date_naive();

            // Try configured template, fall back to built-in
            let template = self.read_template().await.unwrap_or_else(|| FALLBACK_TEMPLATE.to_string());

            // Replace Obsidian template variables
            // {{date}} uses the configured date_display_format, not the file-path format
            let display_date = today.format(&self.date_display_format).to_string();
            let content = template
                .replace("{{date}}", &display_date)
                .replace("{{time}}", &Local::now().format("%H:%M").to_string())
                .replace("{{title}}", &display_date);

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

    /// Replace an existing entry in a specific section by matching URL.
    pub async fn replace_entry_by_url(
        &self,
        section_heading: &str,
        url: &str,
        new_content: &str,
    ) -> Result<PathBuf, VaultError> {
        let path = self.ensure_today().await?;

        let file_content = fs::read_to_string(&path).await?;

        let updated_file_content =
            replace_in_section_by_url(&file_content, section_heading, url, new_content);

        fs::write(&path, &updated_file_content).await?;

        debug!(
            path = %path.display(),
            section = section_heading,
            url,
            content_length = new_content.len(),
            "Replaced daily note entry by URL"
        );

        Ok(path)
    }

    /// Update frontmatter fields in today's daily note
    pub async fn update_frontmatter(
        &self,
        fields: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<PathBuf, VaultError> {
        let path = self.ensure_today().await?;

        let file_content = fs::read_to_string(&path).await?;

        let new_content = crate::vault::frontmatter::update_note_frontmatter(&file_content, fields);

        fs::write(&path, &new_content).await?;

        info!(
            path = %path.display(),
            field_count = fields.len(),
            "Updated daily note frontmatter"
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

/// Replace an entry inside a section by matching URL in markdown link format.
/// Returns the original document unchanged when no matching URL is found in the section.
fn replace_in_section_by_url(
    document: &str,
    section_heading: &str,
    url: &str,
    new_content: &str,
) -> String {
    let lines: Vec<&str> = document.lines().collect();
    let mut section_start = None;

    for (i, line) in lines.iter().enumerate() {
        if line.trim() == section_heading.trim() {
            section_start = Some(i + 1);
            break;
        }
    }

    let Some(section_start) = section_start else {
        return document.to_string();
    };

    let mut section_end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(section_start) {
        if line.trim().starts_with("## ") {
            section_end = i;
            break;
        }
    }

    let url_pattern = format!("]({})", url);
    let mut url_line_idx = None;
    for (i, line) in lines
        .iter()
        .enumerate()
        .skip(section_start)
        .take(section_end.saturating_sub(section_start))
    {
        if line.contains(&url_pattern) {
            url_line_idx = Some(i);
            break;
        }
    }

    let Some(url_line_idx) = url_line_idx else {
        return document.to_string();
    };

    let mut entry_start = url_line_idx;
    if entry_start > section_start {
        let prev_trimmed = lines[entry_start - 1].trim_start();
        if prev_trimmed.starts_with("### ") {
            entry_start -= 1;
        } else if !prev_trimmed.starts_with("- [ ]") {
            let mut j = entry_start;
            while j > section_start {
                let candidate = lines[j - 1].trim_start();
                if candidate.is_empty() {
                    break;
                }
                if candidate.starts_with("- [ ]") || candidate.starts_with("### ") {
                    entry_start = j - 1;
                    break;
                }
                j -= 1;
            }
        }
    }

    let mut entry_end = url_line_idx + 1;
    while entry_end < section_end {
        let next_line = lines[entry_end];
        if next_line.starts_with("  >") || next_line.starts_with("  #") {
            entry_end += 1;
        } else {
            break;
        }
    }

    let mut result_lines: Vec<&str> = Vec::new();
    result_lines.extend_from_slice(&lines[..entry_start]);
    result_lines.extend(new_content.lines());
    result_lines.extend_from_slice(&lines[entry_end..]);

    result_lines.join("\n")
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
    fn test_replace_in_section_basic_replace() {
        let doc = "# Daily\n\n## ✅ Todos\n- [ ] [Old](https://youtube.com/watch?v=abc)\n  > old summary\n  #old\n\n## 📋 Log\n";
        let replacement = "- [ ] [New](https://youtube.com/watch?v=abc) — [[transcripts/new]]\n  > new summary\n  #new";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=abc",
            replacement,
        );

        assert!(result.contains(replacement));
        assert!(!result.contains("old summary"));
        assert!(result.contains("## 📋 Log"));
    }

    #[test]
    fn test_replace_in_section_heading_variant_replaces_four_lines() {
        let doc = "# Daily\n\n## ✅ Todos\n### Video Name\n- [ ] [Old](https://youtube.com/watch?v=abc)\n  > old summary\n  #tag1 #tag2\n\n## 📋 Log\n";
        let replacement = "### Video Name\n- [ ] [New](https://youtube.com/watch?v=abc) — [[transcripts/new]]\n  > updated summary\n  #new";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=abc",
            replacement,
        );

        assert!(result.contains(replacement));
        assert!(!result.contains("#tag1 #tag2"));
        assert!(!result.contains("old summary"));
    }

    #[test]
    fn test_replace_in_section_no_tags_variant_replaces_two_lines() {
        let doc = "# Daily\n\n## ✅ Todos\n- [ ] [Old](https://youtube.com/watch?v=abc)\n  > old summary\n\n## 📋 Log\n";
        let replacement = "- [ ] [New](https://youtube.com/watch?v=abc)\n  > new summary";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=abc",
            replacement,
        );

        assert!(result.contains(replacement));
        assert!(!result.contains("old summary"));
    }

    #[test]
    fn test_replace_in_section_transcript_link_variant() {
        let doc = "# Daily\n\n## ✅ Todos\n- [ ] [Old](https://youtube.com/watch?v=abc) — [[transcripts/old]]\n  > old summary\n  #video\n\n## 📋 Log\n";
        let replacement = "- [ ] [Old](https://youtube.com/watch?v=abc) — [[transcripts/new]]\n  > new transcript summary\n  #video";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=abc",
            replacement,
        );

        assert!(result.contains("[[transcripts/new]]"));
        assert!(!result.contains("[[transcripts/old]]"));
        assert!(!result.contains("old summary"));
    }

    #[test]
    fn test_replace_in_section_url_not_found_no_op() {
        let doc = "# Daily\n\n## ✅ Todos\n- [ ] [Old](https://youtube.com/watch?v=abc)\n  > old summary\n\n## 📋 Log\n";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=missing",
            "- [ ] [New](https://youtube.com/watch?v=missing)\n  > replacement",
        );

        assert_eq!(result, doc);
    }

    #[test]
    fn test_replace_url_not_found() {
        let doc = "# Daily\n\n## ✅ Todos\n- [ ] [Old](https://youtube.com/watch?v=abc)\n  > old summary\n\n## 📋 Log\n";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=missing",
            "- [ ] [New](https://youtube.com/watch?v=missing)\n  > replacement",
        );

        assert_eq!(result, doc);
    }

    #[test]
    fn test_replace_in_section_multiple_entries_only_matching_url_replaced() {
        let doc = "# Daily\n\n## ✅ Todos\n- [ ] [First](https://youtube.com/watch?v=aaa)\n  > first summary\n\n- [ ] [Second](https://youtube.com/watch?v=bbb)\n  > second summary\n  #tag\n\n## 📋 Log\n";
        let replacement = "- [ ] [Second Updated](https://youtube.com/watch?v=bbb)\n  > second updated";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=bbb",
            replacement,
        );

        assert!(result.contains("first summary"));
        assert!(result.contains(replacement));
        assert!(!result.contains("second summary"));
    }

    #[test]
    fn test_replace_in_section_entry_at_end_of_section() {
        let doc = "# Daily\n\n## ✅ Todos\n- [ ] [End](https://youtube.com/watch?v=end)\n  > end summary\n  #tag\n";
        let replacement = "- [ ] [End Updated](https://youtube.com/watch?v=end)\n  > updated end summary\n  #done";

        let result = replace_in_section_by_url(
            doc,
            "## ✅ Todos",
            "https://youtube.com/watch?v=end",
            replacement,
        );

        assert!(result.contains(replacement));
        assert!(!result.contains("  > end summary"));
        assert!(result.contains("## ✅ Todos"));
    }

    #[test]
    fn test_chrono_format_default() {
        let settings = DailyNoteSettings::default();
        assert_eq!(settings.chrono_format(), "%Y-%m-%d");
    }

    #[tokio::test]
    async fn test_update_frontmatter_adds_new_fields() {
        use std::collections::HashMap;
        use serde_json::json;

        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path().to_path_buf();

        // Create daily note with existing frontmatter
        let note_content = "---\ndate: 2026-03-24\ntags: [daily]\n---\n# Daily Note\n\nContent";
        let today = Local::now().format("%Y-%m-%d").to_string();
        let note_path = vault_path.join(format!("{}.md", today));
        fs::write(&note_path, note_content).await.unwrap();

        let manager = DailyNoteManager {
            vault_path: vault_path.clone(),
            settings: DailyNoteSettings::default(),
            date_display_format: "%Y-%m-%d".to_string(),
            sync_notifier: None,
        };

        let mut fields = HashMap::new();
        fields.insert("gewicht".to_string(), json!(80.2));

        let result = manager.update_frontmatter(&fields).await;
        assert!(result.is_ok());

        let updated = fs::read_to_string(&note_path).await.unwrap();
        assert!(updated.contains("gewicht: 80.2"));
        assert!(updated.contains("2026-03-24")); // Date value preserved
        assert!(updated.contains("tags:")); // Tags field preserved
    }

    #[tokio::test]
    async fn test_update_frontmatter_empty_hashmap_no_op() {
        use std::collections::HashMap;

        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path().to_path_buf();

        let note_content = "---\ndate: 2026-03-24\n---\n# Daily Note\n\nContent";
        let today = Local::now().format("%Y-%m-%d").to_string();
        let note_path = vault_path.join(format!("{}.md", today));
        fs::write(&note_path, note_content).await.unwrap();

        let manager = DailyNoteManager {
            vault_path: vault_path.clone(),
            settings: DailyNoteSettings::default(),
            date_display_format: "%Y-%m-%d".to_string(),
            sync_notifier: None,
        };

        let fields = HashMap::new();
        let result = manager.update_frontmatter(&fields).await;
        assert!(result.is_ok());

        let updated = fs::read_to_string(&note_path).await.unwrap();
        // Content should be unchanged for empty HashMap
        assert!(updated.starts_with("---\n"));
        assert!(updated.contains("2026-03-24"));
        assert!(updated.contains("# Daily Note"));
    }

    #[tokio::test]
    async fn test_update_frontmatter_protected_keys_ignored() {
        use std::collections::HashMap;
        use serde_json::json;

        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path().to_path_buf();

        let note_content = "---\ndate: 2026-03-24\n---\n# Daily Note\n\nContent";
        let today = Local::now().format("%Y-%m-%d").to_string();
        let note_path = vault_path.join(format!("{}.md", today));
        fs::write(&note_path, note_content).await.unwrap();

        let manager = DailyNoteManager {
            vault_path: vault_path.clone(),
            settings: DailyNoteSettings::default(),
            date_display_format: "%Y-%m-%d".to_string(),
            sync_notifier: None,
        };

        let mut fields = HashMap::new();
        fields.insert("date".to_string(), json!("9999-01-01"));

        let result = manager.update_frontmatter(&fields).await;
        assert!(result.is_ok());

        let updated = fs::read_to_string(&note_path).await.unwrap();
        assert!(updated.contains("2026-03-24")); // Original date preserved
        assert!(!updated.contains("9999-01-01")); // Protected key not updated
    }

    #[tokio::test]
    async fn test_ensure_today_creates_nested_parent_directories_from_format() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path().to_path_buf();

        let manager = DailyNoteManager {
            vault_path: vault_path.clone(),
            settings: DailyNoteSettings {
                folder: "Daily Notes".to_string(),
                format: "YYYY/MM/YYYY-MM-DD".to_string(),
                template: String::new(),
                autorun: false,
            },
            date_display_format: "%Y-%m-%d".to_string(),
            sync_notifier: None,
        };

        let note_path = manager.ensure_today().await.unwrap();

        assert!(note_path.exists());
        assert!(note_path.parent().unwrap().exists());
        assert!(note_path.starts_with(vault_path.join("Daily Notes")));

        let expected_path = vault_path.join("Daily Notes").join(
            Local::now()
                .format("%Y/%m/%Y-%m-%d.md")
                .to_string(),
        );
        assert_eq!(note_path, expected_path);
    }
}
