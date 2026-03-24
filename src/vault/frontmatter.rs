use std::collections::HashMap;

use tracing::warn;

pub fn parse_frontmatter(content: &str) -> (Option<serde_yml::Value>, &str) {
    let (after_opening, opening_found) = if let Some(rest) = content.strip_prefix("---\n") {
        (content.len() - rest.len(), true)
    } else if let Some(rest) = content.strip_prefix("---\r\n") {
        (content.len() - rest.len(), true)
    } else {
        (0, false)
    };

    if !opening_found {
        return (None, content);
    }

    let mut line_start = after_opening;
    while line_start <= content.len() {
        let remaining = &content[line_start..];
        let newline_rel = remaining.find('\n');
        let line_end = newline_rel
            .map(|idx| line_start + idx)
            .unwrap_or(content.len());
        let line = &content[line_start..line_end];

        if line == "---" || line == "---\r" {
            let yaml_text = &content[after_opening..line_start];
            let body_start = newline_rel
                .map(|idx| line_start + idx + 1)
                .unwrap_or(content.len());

            match serde_yml::from_str::<serde_yml::Value>(yaml_text) {
                Ok(parsed) => return (Some(parsed), &content[body_start..]),
                Err(error) => {
                    warn!(error = %error, "Failed to parse YAML frontmatter");
                    return (None, content);
                }
            }
        }

        if let Some(idx) = newline_rel {
            line_start += idx + 1;
        } else {
            break;
        }
    }

    (None, content)
}

pub fn merge_frontmatter(
    existing: &mut serde_yml::Value,
    new: &HashMap<String, serde_json::Value>,
    protected_keys: &[&str],
) {
    if !existing.is_mapping() {
        *existing = serde_yml::Value::Mapping(serde_yml::Mapping::new());
    }

    let map = existing
        .as_mapping_mut()
        .expect("mapping should exist after normalization");

    for (key, value) in new {
        if protected_keys.contains(&key.as_str()) {
            continue;
        }

        match serde_yml::to_value(value) {
            Ok(yaml_value) => {
                map.insert(serde_yml::Value::String(key.clone()), yaml_value);
            }
            Err(error) => {
                warn!(key = %key, error = %error, "Failed to convert JSON value to YAML");
            }
        }
    }
}

pub fn serialize_frontmatter(yaml: &serde_yml::Value) -> String {
    let mut serialized = serde_yml::to_string(yaml).unwrap_or_default();

    if let Some(stripped) = serialized.strip_prefix("---\n") {
        serialized = stripped.to_string();
    } else if let Some(stripped) = serialized.strip_prefix("---\r\n") {
        serialized = stripped.to_string();
    }

    if let Some(stripped) = serialized.strip_suffix("\n...") {
        serialized = stripped.to_string();
    } else if let Some(stripped) = serialized.strip_suffix("\n...\n") {
        serialized = stripped.to_string();
    }

    if !serialized.ends_with('\n') {
        serialized.push('\n');
    }

    format!("---\n{}---\n", serialized)
}

pub fn update_note_frontmatter(
    content: &str,
    new_fields: &HashMap<String, serde_json::Value>,
) -> String {
    const PROTECTED_KEYS: &[&str] = &["date", "tags"];

    let (existing_yaml, body) = parse_frontmatter(content);
    let mut yaml = existing_yaml
        .filter(|value| value.is_mapping())
        .unwrap_or_else(|| serde_yml::Value::Mapping(serde_yml::Mapping::new()));

    merge_frontmatter(&mut yaml, new_fields, PROTECTED_KEYS);
    let serialized = serialize_frontmatter(&yaml);

    format!("{}{}", serialized, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_mapping(value: &serde_yml::Value) -> &serde_yml::Mapping {
        value.as_mapping().expect("expected YAML mapping")
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\ndate: 2026-03-16\ntags: [daily]\n---\n# Title";
        let (yaml, body) = parse_frontmatter(content);

        assert_eq!(body, "# Title");
        let yaml = yaml.expect("expected frontmatter to parse");
        let map = as_mapping(&yaml);
        assert!(map.contains_key("date"));
        assert!(map.contains_key("tags"));
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "# Title\nContent";
        let (yaml, body) = parse_frontmatter(content);

        assert!(yaml.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_malformed_frontmatter() {
        let content = "---\ninvalid: [unclosed\n---";
        let (yaml, body) = parse_frontmatter(content);

        assert!(yaml.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_merge_upsert() {
        let mut existing =
            serde_yml::from_str::<serde_yml::Value>("date: 2026-03-16\ntags: [daily]")
                .expect("valid YAML");
        let new = HashMap::from([("gewicht".to_string(), serde_json::json!(80.2))]);

        merge_frontmatter(&mut existing, &new, &["date", "tags"]);

        let map = as_mapping(&existing);
        assert!(map.contains_key("date"));
        assert!(map.contains_key("tags"));
        assert_eq!(
            map.get("gewicht"),
            Some(&serde_yml::to_value(80.2).expect("yaml value"))
        );
    }

    #[test]
    fn test_merge_overwrite_allowed() {
        let mut existing =
            serde_yml::from_str::<serde_yml::Value>("gewicht: 79.0").expect("valid YAML");
        let new = HashMap::from([("gewicht".to_string(), serde_json::json!(80.2))]);

        merge_frontmatter(&mut existing, &new, &["date", "tags"]);

        let map = as_mapping(&existing);
        assert_eq!(
            map.get("gewicht"),
            Some(&serde_yml::to_value(80.2).expect("yaml value"))
        );
    }

    #[test]
    fn test_merge_protected_keys() {
        let mut existing =
            serde_yml::from_str::<serde_yml::Value>("date: 2026-03-16\ntags: [daily]")
                .expect("valid YAML");
        let new = HashMap::from([
            ("date".to_string(), serde_json::json!("2099-01-01")),
            ("tags".to_string(), serde_json::json!(["modified"])),
        ]);

        merge_frontmatter(&mut existing, &new, &["date", "tags"]);

        let map = as_mapping(&existing);
        assert_eq!(
            map.get("date"),
            Some(&serde_yml::to_value("2026-03-16").expect("yaml value"))
        );
        assert_eq!(
            map.get("tags"),
            Some(&serde_yml::to_value(vec!["daily"]).expect("yaml value"))
        );
    }

    #[test]
    fn test_serialize_frontmatter() {
        let yaml = serde_yml::from_str::<serde_yml::Value>("key: value").expect("valid YAML");
        let serialized = serialize_frontmatter(&yaml);

        assert!(serialized.starts_with("---\n"));
        assert!(serialized.contains("key: value"));
        assert!(serialized.ends_with("---\n"));
    }

    #[test]
    fn test_roundtrip() {
        let original = "---\ngewicht: 79.0\n---\n# Daily\nBody";
        let new = HashMap::from([
            ("vetpercentage".to_string(), serde_json::json!(22.1)),
            ("tags".to_string(), serde_json::json!(["daily"])),
        ]);

        let updated = update_note_frontmatter(original, &new);
        let (yaml, body) = parse_frontmatter(&updated);

        let yaml = yaml.expect("expected frontmatter");
        let map = as_mapping(&yaml);
        assert!(map.contains_key("gewicht"));
        assert!(map.contains_key("vetpercentage"));
        assert_eq!(body, "# Daily\nBody");
    }

    #[test]
    fn test_insert_frontmatter_into_no_frontmatter_doc() {
        let content = "# Title\nBody";
        let new = HashMap::from([("gewicht".to_string(), serde_json::json!(80.2))]);

        let updated = update_note_frontmatter(content, &new);

        assert!(updated.starts_with("---\n"));
        assert!(updated.contains("gewicht: 80.2"));
        assert!(updated.contains("# Title\nBody"));
    }
}
