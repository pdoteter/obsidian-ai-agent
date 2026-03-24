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

        let body = build_text_request_body(text, model, &system_prompt);
        let classified = self.chat_completion_and_parse_classification(body).await?;

        info!(
            category = %classified.category,
            tags = ?classified.tags,
            summary = %classified.summary,
            "Text classified"
        );

        Ok(classified)
    }

    /// Classify an image using vision API (multimodal)
    pub async fn classify_image(
        &self,
        image_base64: &str,
        caption: Option<&str>,
        exif_context: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError> {
        info!(
            caption_length = caption.map(|s| s.len()).unwrap_or(0),
            has_exif = !exif_context.is_empty(),
            model = model,
            "Classifying image"
        );

        let base_prompt = format!(
            "{}\n\nYou are also receiving an image. Describe what you see and classify it. If a caption is provided, use it as primary context. Include the image description in the markdown output as a short paragraph.",
            CLASSIFICATION_SYSTEM_PROMPT
        );
        let system_prompt = crate::ai::guide::compose_system_prompt(&base_prompt, guide);

        let body = build_image_request_body(image_base64, caption, exif_context, model, &system_prompt);
        let classified = self.chat_completion_and_parse_classification(body).await?;

        info!(
            category = %classified.category,
            tags = ?classified.tags,
            summary = %classified.summary,
            "Image classified"
        );

        Ok(classified)
    }

    async fn chat_completion_and_parse_classification(
        &self,
        body: serde_json::Value,
    ) -> Result<ClassifiedNote, AiError> {
        let response = self.chat_completion(body).await?;
        let content = Self::extract_content(&response)?;

        serde_json::from_str(&content).map_err(|e| {
            AiError::ClassificationFailed(format!(
                "Failed to parse classification JSON: {}. Raw: {}",
                e, content
            ))
        })
    }
}

fn classified_note_response_format() -> serde_json::Value {
    json!({
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
    })
}

fn build_text_request_body(text: &str, model: &str, system_prompt: &str) -> serde_json::Value {
    json!({
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
        "response_format": classified_note_response_format()
    })
}

fn build_image_request_body(
    image_base64: &str,
    caption: Option<&str>,
    exif_context: &str,
    model: &str,
    system_prompt: &str,
) -> serde_json::Value {
    let text_content = if let Some(cap) = caption {
        if exif_context.is_empty() {
            cap.to_string()
        } else {
            format!("{}\n\n{}", cap, exif_context)
        }
    } else if exif_context.is_empty() {
        "Describe this image.".to_string()
    } else {
        format!("Describe this image.\n\n{}", exif_context)
    };

    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": system_prompt
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": text_content
                    },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": image_base64
                        }
                    }
                ]
            }
        ],
        "response_format": classified_note_response_format()
    })
}

/// Generate a filename-safe slug from an AI summary
pub fn slug_from_summary(summary: &str) -> String {
    let slug = crate::image::process::sanitize_slug(summary);
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
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

    #[test]
    fn test_build_image_request_body_multimodal_with_caption() {
        let body = build_image_request_body(
            "data:image/jpeg;base64,abc123",
            Some("A calm harbor at sunset"),
            "Photo taken: 2026:03:24 18:30:00.",
            "google/gemini-2.5-flash",
            "system prompt",
        );

        assert_eq!(body["model"], json!("google/gemini-2.5-flash"));
        assert_eq!(body["messages"][0]["role"], json!("system"));

        let user_content = &body["messages"][1]["content"];
        assert!(
            user_content.is_array(),
            "user content should be an array for multimodal input"
        );
        assert_eq!(user_content[0]["type"], json!("text"));
        assert_eq!(
            user_content[1]["type"],
            json!("image_url"),
            "second content block should carry image_url"
        );
        assert_eq!(
            user_content[1]["image_url"]["url"],
            json!("data:image/jpeg;base64,abc123")
        );

        let text_block = user_content[0]["text"].as_str().unwrap_or("");
        assert!(text_block.contains("A calm harbor at sunset"));
        assert!(text_block.contains("Photo taken: 2026:03:24 18:30:00."));
    }

    #[test]
    fn test_slug_from_summary() {
        let slug = slug_from_summary("Beautiful sunset over the harbor");
        assert_eq!(slug, "beautiful-sunset-over-the-harbor");
    }

    #[test]
    fn test_slug_from_summary_long() {
        let input = "This is an intentionally long summary sentence that should be trimmed cleanly";
        let slug = slug_from_summary(input);

        assert!(slug.len() <= 50, "slug should be max 50 chars");
        assert!(
            !slug.ends_with('-'),
            "slug should not end with trailing hyphen"
        );
    }

    #[test]
    fn test_slug_from_summary_special_chars() {
        let slug = slug_from_summary("Test! @#$ Photo 123");
        assert_eq!(slug, "test-photo-123");
    }

    #[test]
    fn test_slug_from_summary_empty() {
        let slug = slug_from_summary("");
        assert!(
            !slug.is_empty(),
            "empty summary should produce non-empty fallback slug"
        );
    }
}
