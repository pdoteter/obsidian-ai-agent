use chrono::Local;

use crate::ai::classify::{ClassifiedNote, NoteCategory};

/// Format a classified note for insertion into the daily note.
/// Returns (section_heading, formatted_content).
pub fn format_for_daily_note(note: &ClassifiedNote) -> (&'static str, String) {
    let tags_str = if note.tags.is_empty() {
        String::new()
    } else {
        let tags: Vec<String> = note.tags.iter().map(|t| format!("#{}", t)).collect();
        format!(" {}", tags.join(" "))
    };

    match note.category {
        NoteCategory::Todo => {
            let content = format!("{}{}", note.markdown, tags_str);
            ("## ✅ Todos", content)
        }
        NoteCategory::Log => {
            let time = Local::now().format("%H:%M").to_string();
            // Strip leading "- " from AI markdown to rebuild with timestamp
            let entry_text = note.markdown.trim_start_matches("- ").trim();
            let content = format!("- {} — {}{}", time, entry_text, tags_str);
            ("## 📋 Log", content)
        }
        NoteCategory::Note => {
            let content = format!("{}{}", note.markdown, tags_str);
            ("## 📝 Notes", content)
        }
    }
}

/// Create a simple text entry without AI classification.
/// Used as a fallback when AI is unavailable.
pub fn format_raw_entry(text: &str) -> (&'static str, String) {
    let time = Local::now().format("%H:%M").to_string();
    let content = format!("- {} — {}", time, text);
    ("## 📋 Log", content)
}

/// Format URL metadata into a TODO markdown entry for the daily note's `## ✅ Todos` section.
/// Returns (section_heading, formatted_content) following the pattern of other format_* functions.
///
/// # Arguments
/// * `url` - The URL to add as a TODO
/// * `title` - Optional page title for the markdown link text
/// * `summary` - Optional page summary to display in a blockquote
/// * `tags` - Slice of tags to append as hashtags
/// * `transcript_link` - Optional transcript wiki-link to append after the title
///
/// # Returns
/// Tuple of ("## ✅ Todos", formatted_markdown_string)
///
/// # Format Examples
/// Full: `- [ ] [Title](url) — [[transcript/path]]\n  > summary\n  #tag1 #tag2`
/// Without summary: `- [ ] [Title](url)\n  > ⚠️ Could not fetch page content`
/// Without title: `- [ ] url\n  > summary`
pub fn format_url_todo(
    url: &str,
    title: Option<&str>,
    summary: Option<&str>,
    tags: &[String],
    transcript_link: Option<&str>,
) -> (&'static str, String) {
    // Build the title line
    let title_line = if let Some(title) = title {
        format!("- [ ] [{}]({})", title, url)
    } else {
        format!("- [ ] {}", url)
    };

    // Append transcript wiki-link if present
    let title_line = if let Some(transcript_link) = transcript_link {
        format!("{} — [[{}]]", title_line, transcript_link)
    } else {
        title_line
    };

    // Build the summary line
    let summary_line = if let Some(summary) = summary {
        format!("  > {}", summary)
    } else {
        "  > ⚠️ Could not fetch page content".to_string()
    };

    // Build the tags line (only if tags are present)
    let tags_line = if !tags.is_empty() {
        let tag_str = tags
            .iter()
            .map(|t| format!("#{}", t))
            .collect::<Vec<_>>()
            .join(" ");
        format!("  {}", tag_str)
    } else {
        String::new()
    };

    // Combine all parts
    let content = if tags.is_empty() {
        format!("{}\n{}", title_line, summary_line)
    } else {
        format!("{}\n{}\n{}", title_line, summary_line, tags_line)
    };

    ("## ✅ Todos", content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_todo() {
        let note = ClassifiedNote {
            category: NoteCategory::Todo,
            markdown: "- [ ] Buy groceries".to_string(),
            tags: vec!["shopping".to_string()],
            summary: "Buy groceries".to_string(),
            frontmatter: None,
        };

        let (section, content) = format_for_daily_note(&note);
        assert_eq!(section, "## ✅ Todos");
        assert!(content.contains("- [ ] Buy groceries"));
        assert!(content.contains("#shopping"));
    }

    #[test]
    fn test_format_log() {
        let note = ClassifiedNote {
            category: NoteCategory::Log,
            markdown: "- Team meeting".to_string(),
            tags: vec!["work".to_string()],
            summary: "Team meeting".to_string(),
            frontmatter: None,
        };

        let (section, content) = format_for_daily_note(&note);
        assert_eq!(section, "## 📋 Log");
        // Should have format: "- HH:MM — Team meeting #work"
        assert!(content.starts_with("- "));
        assert!(content.contains(" — Team meeting"));
        assert!(content.contains("#work"));
    }

    #[test]
    fn test_format_log_with_timestamp() {
        let note = ClassifiedNote {
            category: NoteCategory::Log,
            markdown: "- Went for a run".to_string(),
            tags: vec![],
            summary: "Running".to_string(),
            frontmatter: None,
        };

        let (section, content) = format_for_daily_note(&note);
        assert_eq!(section, "## 📋 Log");
        // Verify format: "- HH:MM — Went for a run"
        assert!(
            content.starts_with("- "),
            "Should start with '- ', got: {}",
            content
        );
        assert!(
            content.contains(" — Went for a run"),
            "Should contain ' — Went for a run', got: {}",
            content
        );
        // Verify the time part is 5 chars (HH:MM) after "- "
        let after_dash = &content[2..7];
        assert!(
            after_dash.contains(':'),
            "Should have HH:MM format, got: {}",
            after_dash
        );
    }

    #[test]
    fn test_format_url_todo_full() {
        let (section, content) = format_url_todo(
            "https://example.com",
            Some("Example"),
            Some("This is a summary"),
            &["web".to_string(), "example".to_string()],
            Some("transcripts/2026-03-25-video-title"),
        );
        assert_eq!(section, "## ✅ Todos");
        assert!(content.contains(
            "- [ ] [Example](https://example.com) — [[transcripts/2026-03-25-video-title]]"
        ));
        assert!(content.contains("> This is a summary"));
        assert!(content.contains("#web"));
        assert!(content.contains("#example"));
    }

    #[test]
    fn test_format_url_todo_no_transcript() {
        let (section, content) = format_url_todo(
            "https://example.com",
            Some("Example"),
            Some("Summary"),
            &["web".to_string()],
            None,
        );
        assert_eq!(section, "## ✅ Todos");
        assert!(content.contains("- [ ] [Example](https://example.com)"));
        assert!(!content.contains("—"));
        assert!(content.contains("> Summary"));
        assert!(content.contains("#web"));
    }

    #[test]
    fn test_format_url_todo_no_tags() {
        let (section, content) = format_url_todo(
            "https://example.com",
            Some("Example"),
            Some("Summary"),
            &[],
            None,
        );
        assert_eq!(section, "## ✅ Todos");
        assert!(content.contains("- [ ] [Example](https://example.com)"));
        assert!(content.contains("> Summary"));
        // Should not have a tag line
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_format_url_todo_no_summary() {
        let (section, content) = format_url_todo(
            "https://example.com",
            Some("Example"),
            None,
            &["web".to_string()],
            None,
        );
        assert_eq!(section, "## ✅ Todos");
        assert!(content.contains("- [ ] [Example](https://example.com)"));
        assert!(content.contains("> ⚠️ Could not fetch page content"));
        assert!(content.contains("#web"));
    }

    #[test]
    fn test_format_url_todo_no_title() {
        let (section, content) = format_url_todo(
            "https://example.com",
            None,
            Some("Summary"),
            &["web".to_string()],
            None,
        );
        assert_eq!(section, "## ✅ Todos");
        assert!(content.contains("- [ ] https://example.com"));
        assert!(content.contains("> Summary"));
        assert!(content.contains("#web"));
    }

    #[test]
    fn test_format_url_todo_url_only() {
        let (section, content) = format_url_todo("https://example.com", None, None, &[], None);
        assert_eq!(section, "## ✅ Todos");
        assert!(content.contains("- [ ] https://example.com"));
        assert!(content.contains("> ⚠️ Could not fetch page content"));
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_format_url_todo_special_chars() {
        let (section, content) = format_url_todo(
            "https://example.com",
            Some("Example [with] brackets"),
            Some("Summary"),
            &[],
            None,
        );
        assert_eq!(section, "## ✅ Todos");
        // Should contain markdown link even with special chars
        assert!(content.contains("- [ ] [Example [with] brackets](https://example.com)"));
        assert!(content.contains("> Summary"));
    }
}
