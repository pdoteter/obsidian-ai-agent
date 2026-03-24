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
}
