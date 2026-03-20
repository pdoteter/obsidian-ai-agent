use serde::{Deserialize, Serialize};
use serde_json::json;
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
}

impl OpenRouterClient {
    /// Classify text into a structured note (todo/log/note) with formatted markdown.
    pub async fn classify_text(
        &self,
        text: &str,
        model: &str,
    ) -> Result<ClassifiedNote, AiError> {
        info!(text_length = text.len(), model = model, "Classifying text");

        let body = json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": CLASSIFICATION_SYSTEM_PROMPT
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
                            }
                        },
                        "required": ["category", "markdown", "tags", "summary"],
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
