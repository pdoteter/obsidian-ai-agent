use chrono::Local;
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

use crate::error::VaultError;

const DAILY_NOTE_TEMPLATE: &str = r#"---
date: {{date}}
tags: [daily]
---

# {{date}}

## 📝 Notes

## ✅ Todos

## 📋 Log

"#;

/// Manages daily note files in the Obsidian vault
pub struct DailyNoteManager {
    vault_path: PathBuf,
}

impl DailyNoteManager {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }

    /// Get the path to today's daily note
    pub fn today_path(&self) -> PathBuf {
        let date = Local::now().format("%Y-%m-%d").to_string();
        self.vault_path.join("Daily").join(format!("{}.md", date))
    }

    /// Get the path to a daily note for a specific date string (YYYY-MM-DD)
    #[allow(dead_code)]
    pub fn path_for_date(&self, date: &str) -> PathBuf {
        self.vault_path.join("Daily").join(format!("{}.md", date))
    }

    /// Ensure today's daily note exists. Creates it from template if not.
    /// Returns the path to the daily note.
    pub async fn ensure_today(&self) -> Result<PathBuf, VaultError> {
        let path = self.today_path();

        // Ensure the Daily directory exists
        let daily_dir = self.vault_path.join("Daily");
        if !daily_dir.exists() {
            fs::create_dir_all(&daily_dir).await?;
            info!(dir = %daily_dir.display(), "Created Daily notes directory");
        }

        // Create file from template if it doesn't exist
        if !path.exists() {
            let date = Local::now().format("%Y-%m-%d").to_string();
            let content = DAILY_NOTE_TEMPLATE
                .replace("{{date}}", &date);

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
}
