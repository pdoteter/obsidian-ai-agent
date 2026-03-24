use std::path::PathBuf;

/// Load a user guide from a file path, returning None if missing or empty.
///
/// # Arguments
/// * `path` - Optional path to guide file. Returns None if path is None.
/// * On file read error, logs warning and returns None.
/// * Empty files return None.
pub fn load_guide(path: &Option<PathBuf>) -> Option<String> {
    let path = path.as_ref()?;

    match std::fs::read_to_string(path) {
        Ok(content) => {
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(e) => {
            eprintln!("Failed to read guide file: {}", e);
            None
        }
    }
}

/// Compose a system prompt with optional guide content.
///
/// If guide is provided, appends it wrapped in `<user_guide>` delimiters.
/// If guide is None, returns base_prompt unchanged.
pub fn compose_system_prompt(base_prompt: &str, guide: Option<&str>) -> String {
    match guide {
        Some(guide_content) => {
            format!(
                "{}\n\n<user_guide>\n{}\n</user_guide>",
                base_prompt, guide_content
            )
        }
        None => base_prompt.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_guide_from_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let guide_content = "This is a user guide\nwith multiple lines";
        temp_file.write_all(guide_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let path = Some(temp_file.path().to_path_buf());
        let result = load_guide(&path);

        assert!(result.is_some());
        assert_eq!(result.unwrap(), guide_content);
    }

    #[test]
    fn test_load_guide_missing_file() {
        let path = Some(PathBuf::from("/nonexistent/path/to/guide.md"));
        let result = load_guide(&path);

        assert!(result.is_none());
    }

    #[test]
    fn test_load_guide_empty_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = Some(temp_file.path().to_path_buf());
        let result = load_guide(&path);

        assert!(result.is_none());
    }

    #[test]
    fn test_load_guide_none_path() {
        let path: Option<PathBuf> = None;
        let result = load_guide(&path);

        assert!(result.is_none());
    }

    #[test]
    fn test_compose_prompt_with_guide() {
        let base = "You are an assistant";
        let guide = "Use concise language";
        let result = compose_system_prompt(base, Some(guide));

        assert!(result.contains(base));
        assert!(result.contains("<user_guide>"));
        assert!(result.contains(guide));
        assert!(result.contains("</user_guide>"));
        assert_eq!(
            result,
            "You are an assistant\n\n<user_guide>\nUse concise language\n</user_guide>"
        );
    }

    #[test]
    fn test_compose_prompt_without_guide() {
        let base = "You are an assistant";
        let result = compose_system_prompt(base, None);

        assert_eq!(result, base);
        assert!(!result.contains("<user_guide>"));
    }
}
