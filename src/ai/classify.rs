use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tracing::info;

use super::client::OpenRouterClient;
use crate::error::AiError;

/// The category assigned to a piece of text by the AI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NoteCategory {
    Todo,
    Log,
    Note,
}

impl std::fmt::Display for NoteCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoteCategory::Todo => write!(f, "todo"),
            NoteCategory::Log => write!(f, "log"),
            NoteCategory::Note => write!(f, "note"),
        }
    }
}

/// The structured output from the AI classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedNote {
    /// The category: todo, log, or note
    pub category: NoteCategory,
    /// The formatted markdown content ready for the daily note
    pub markdown: String,
    /// Optional tags extracted from the content
    pub tags: Vec<String>,
    /// Brief summary (1 line)
    pub summary: String,
    /// Optional key-value pairs for daily note frontmatter updates
    #[serde(default)]
    pub frontmatter: Option<HashMap<String, serde_json::Value>>,
}

impl OpenRouterClient {
    /// Classify text into a structured note (todo/log/note) with formatted markdown.
    pub async fn classify_text(
        &self,
        text: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError> {
        info!(text_length = text.len(), model = model, "Classifying text");
        let system_prompt = crate::ai::guide::compose_system_prompt(CLASSIFICATION_SYSTEM_PROMPT, guide);

        let body = json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt
                },
                {
                    "role": "user",
                    "content": text
                }
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "classified_note",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "category": {
                                "type": "string",
                                "enum": ["todo", "log", "note"],
                                "description": "The type of entry: todo for action items, log for activity/event logs, note for knowledge/thoughts"
                            },
                            "markdown": {
                                "type": "string",
                                "description": "The formatted markdown content. For todos: use '- [ ] ' prefix. For logs: use '- <activity description>' (do NOT include time — it will be added automatically). For notes: use a clean paragraph or bullet points."
                            },
                            "tags": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Relevant Obsidian tags without # prefix (e.g. 'work', 'health', 'project-x')"
                            },
                            "summary": {
                                "type": "string",
                                "description": "A one-line summary of the content"
                            },
                            "frontmatter": {
                                "type": ["object", "null"],
                                "description": "Optional frontmatter key-value pairs to add/update in the daily note YAML frontmatter. Use for structured data like weight, measurements, etc. Return null if no frontmatter updates needed.",
                                "additionalProperties": true
                            }
                        },
                        "required": ["category", "markdown", "tags", "summary", "frontmatter"],
                        "additionalProperties": false
                    }
                }
            }
        });

        let response = self.chat_completion(body).await?;
        let content = Self::extract_content(&response)?;

        let classified: ClassifiedNote = serde_json::from_str(&content).map_err(|e| {
            AiError::ClassificationFailed(format!(
                "Failed to parse classification JSON: {}. Raw: {}",
                e, content
            ))
        })?;

        info!(
            category = %classified.category,
            tags = ?classified.tags,
            summary = %classified.summary,
            "Text classified"
        );

        Ok(classified)
    }
}

const CLASSIFICATION_SYSTEM_PROMPT: &str = r#"You are a personal knowledge management assistant. Your job is to classify incoming text messages and format them as markdown for an Obsidian daily note.

## Classification Rules:

**todo** — Action items, tasks, reminders, things to do
- Format: `- [ ] <task description>`
- Example input: "Vergeet niet melk te kopen"
- Example output: `- [ ] Melk kopen`

**log** — Activities, events, things that happened or are happening
- Format: `- <activity description>` (do NOT include time — it is added automatically)
- Example input: "Net 30 minuten hardgelopen in het park"
- Example output: `- 30 min hardgelopen in het park 🏃`

**note** — Thoughts, ideas, knowledge, observations, reflections
- Format: Clean paragraph or bullet points as appropriate
- Example input: "Ik denk dat we het project beter kunnen opsplitsen in microservices"
- Example output: A well-formatted note about the thought

## Important:
- Preserve the original language (Dutch, English, etc.)
- Keep it concise but complete
- Extract relevant tags (without # prefix)
- The markdown should be ready to append directly to a daily note
- For todos, always use `- [ ] ` checkbox format
- For logs, do NOT include time — timestamps are added automatically by the system
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::guide::compose_system_prompt;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn test_classified_note_deserialize_with_frontmatter() {
        let value = json!({
            "category": "note",
            "markdown": "test",
            "tags": [],
            "summary": "test",
            "frontmatter": {"gewicht": 80.2}
        });

        let note: ClassifiedNote = serde_json::from_value(value).expect("should deserialize");
        let frontmatter = note.frontmatter.expect("frontmatter should be Some");

        assert_eq!(
            frontmatter.get("gewicht"),
            Some(&json!(80.2)),
            "frontmatter should contain gewicht value"
        );
    }

    #[test]
    fn test_classified_note_deserialize_without_frontmatter() {
        let value = json!({
            "category": "note",
            "markdown": "test",
            "tags": [],
            "summary": "test",
            "frontmatter": null
        });

        let note: ClassifiedNote = serde_json::from_value(value).expect("should deserialize");
        assert!(note.frontmatter.is_none(), "frontmatter null should map to None");
    }

    #[test]
    fn test_classified_note_deserialize_empty_frontmatter() {
        let value = json!({
            "category": "note",
            "markdown": "test",
            "tags": [],
            "summary": "test",
            "frontmatter": {}
        });

        let note: ClassifiedNote = serde_json::from_value(value).expect("should deserialize");
        let frontmatter = note.frontmatter.expect("frontmatter should be Some");
        let expected: HashMap<String, serde_json::Value> = HashMap::new();
        assert_eq!(frontmatter, expected, "frontmatter should be empty map");
    }

    #[test]
    fn test_classified_note_backward_compat() {
        let value = json!({
            "category": "note",
            "markdown": "test",
            "tags": [],
            "summary": "test"
        });

        let note: ClassifiedNote = serde_json::from_value(value).expect("should deserialize");
        assert!(
            note.frontmatter.is_none(),
            "missing frontmatter should map to None"
        );
    }

    #[test]
    fn test_compose_system_prompt_with_guide() {
        let guide = "## Custom Rules\ngewicht triggers frontmatter";
        let composed = compose_system_prompt(CLASSIFICATION_SYSTEM_PROMPT, Some(guide));

        assert!(composed.contains(CLASSIFICATION_SYSTEM_PROMPT));
        assert!(composed.contains("<user_guide>"));
        assert!(composed.contains("## Custom Rules"));
        assert!(composed.contains("</user_guide>"));
    }

    #[test]
    fn test_compose_system_prompt_without_guide() {
        let composed = compose_system_prompt(CLASSIFICATION_SYSTEM_PROMPT, None);

        assert_eq!(composed, CLASSIFICATION_SYSTEM_PROMPT);
    }
}
