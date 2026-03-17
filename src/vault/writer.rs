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
            let content = format!("{}{}", note.markdown, tags_str);
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
            markdown: "- 14:00 — Team meeting".to_string(),
            tags: vec!["work".to_string()],
            summary: "Team meeting".to_string(),
        };

        let (section, content) = format_for_daily_note(&note);
        assert_eq!(section, "## 📋 Log");
        assert!(content.contains("Team meeting"));
    }
}
