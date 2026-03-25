use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

use crate::ai::client::OpenRouterClient;
use crate::error::AiError;
use crate::url::PageContent;

/// The structured output from AI URL summarization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UrlSummary {
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
}

impl OpenRouterClient {
    pub async fn summarize_url(
        &self,
        page_content: &PageContent,
        user_text: Option<&str>,
        model: &str,
        guide: Option<&str>,
    ) -> Result<UrlSummary, AiError> {
        info!(
            url = %page_content.url,
            body_length = page_content.body_text.len(),
            model = model,
            "Summarizing URL content"
        );

        let system_prompt =
            crate::ai::guide::compose_system_prompt(url_summary_system_prompt(), guide);
        let body = build_url_request_body(page_content, user_text, model, &system_prompt);
        let summarized = self.chat_completion_and_parse_url_summary(body).await?;

        info!(
            title = %summarized.title,
            tags = ?summarized.tags,
            "URL content summarized"
        );

        Ok(summarized)
    }

    async fn chat_completion_and_parse_url_summary(
        &self,
        body: serde_json::Value,
    ) -> Result<UrlSummary, AiError> {
        let response = self.chat_completion(body).await?;
        let content = Self::extract_content(&response)?;

        serde_json::from_str(&content).map_err(|e| {
            AiError::ClassificationFailed(format!(
                "Failed to parse URL summary JSON: {}. Raw: {}",
                e, content
            ))
        })
    }
}

fn url_summary_response_format() -> serde_json::Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "url_summary",
            "strict": true,
            "schema": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Concise page title in 5-10 words"
                    },
                    "summary": {
                        "type": "string",
                        "description": "Concise summary in 2-3 sentences"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Relevant tags (3-5) without # prefix"
                    }
                },
                "required": ["title", "summary", "tags"],
                "additionalProperties": false
            }
        }
    })
}

fn build_url_request_body(
    page_content: &PageContent,
    user_text: Option<&str>,
    model: &str,
    system_prompt: &str,
) -> serde_json::Value {
    let user_prompt = build_url_user_prompt(page_content, user_text);

    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": system_prompt
            },
            {
                "role": "user",
                "content": user_prompt
            }
        ],
        "response_format": url_summary_response_format(),
        "max_tokens": 4096
    })
}

fn build_url_user_prompt(page_content: &PageContent, user_text: Option<&str>) -> String {
    let title = page_content.title.as_deref().unwrap_or("none provided");
    let description = page_content
        .description
        .as_deref()
        .unwrap_or("none provided");
    let user_context = user_text.unwrap_or("none provided");

    format!(
        "URL: {}\n\nPage title: {}\n\nPage description: {}\n\nPage body:\n{}\n\nUser context: {}",
        page_content.url, title, description, page_content.body_text, user_context
    )
}

fn url_summary_system_prompt() -> &'static str {
    "Analyze this web page content and optional user context. Produce: concise title (5-10 words), summary (2-3 sentences), relevant tags (3-5)."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::guide::compose_system_prompt;
    use serde_json::json;

    fn sample_page_content() -> PageContent {
        PageContent {
            title: Some("Example Article".to_string()),
            description: Some("An example description".to_string()),
            body_text: "This is the full article body text.".to_string(),
            url: "https://example.com/article".to_string(),
        }
    }

    #[test]
    fn test_url_summary_response_format_is_strict_schema() {
        let format = url_summary_response_format();

        assert_eq!(format["type"], json!("json_schema"));
        assert_eq!(format["json_schema"]["name"], json!("url_summary"));
        assert_eq!(format["json_schema"]["strict"], json!(true));
        assert_eq!(
            format["json_schema"]["schema"]["required"],
            json!(["title", "summary", "tags"])
        );
        assert_eq!(
            format["json_schema"]["schema"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn test_build_url_request_body_includes_response_format_and_max_tokens() {
        let body = build_url_request_body(
            &sample_page_content(),
            Some("Please focus on implementation details"),
            "google/gemini-2.5-flash",
            "system prompt",
        );

        assert_eq!(body["model"], json!("google/gemini-2.5-flash"));
        assert_eq!(body["messages"][0]["role"], json!("system"));
        assert_eq!(body["messages"][0]["content"], json!("system prompt"));
        assert_eq!(body["messages"][1]["role"], json!("user"));
        assert!(body["messages"][1]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("https://example.com/article"));
        assert_eq!(body["max_tokens"], json!(4096));
        assert_eq!(body["response_format"]["type"], json!("json_schema"));
    }

    #[test]
    fn test_build_url_user_prompt_includes_page_content_and_user_context() {
        let prompt = build_url_user_prompt(
            &sample_page_content(),
            Some("I care most about the practical takeaways."),
        );

        assert!(prompt.contains("https://example.com/article"));
        assert!(prompt.contains("Example Article"));
        assert!(prompt.contains("An example description"));
        assert!(prompt.contains("This is the full article body text."));
        assert!(prompt.contains("I care most about the practical takeaways."));
    }

    #[test]
    fn test_build_url_user_prompt_without_user_context() {
        let prompt = build_url_user_prompt(&sample_page_content(), None);
        assert!(prompt.contains("User context: none provided"));
    }

    #[test]
    fn test_url_summary_deserializes_successfully() {
        let value = json!({
            "title": "Practical Rust testing patterns",
            "summary": "This article explains how to structure unit tests with TDD. It focuses on fast feedback loops.",
            "tags": ["rust", "testing", "tdd"]
        });

        let parsed: UrlSummary = serde_json::from_value(value).expect("should deserialize");
        assert_eq!(parsed.title, "Practical Rust testing patterns");
        assert_eq!(parsed.tags.len(), 3);
    }

    #[test]
    fn test_url_summary_deserialize_fails_when_tags_not_array() {
        let value = json!({
            "title": "Title",
            "summary": "Summary",
            "tags": "rust"
        });

        let parsed = serde_json::from_value::<UrlSummary>(value);
        assert!(parsed.is_err(), "invalid tags type should fail");
    }

    #[test]
    fn test_compose_system_prompt_with_guide_for_url_summary() {
        let guide = "Use short, actionable tags";
        let composed = compose_system_prompt(url_summary_system_prompt(), Some(guide));

        assert!(composed.contains(url_summary_system_prompt()));
        assert!(composed.contains("<user_guide>"));
        assert!(composed.contains("Use short, actionable tags"));
        assert!(composed.contains("</user_guide>"));
    }

    #[test]
    fn test_compose_system_prompt_without_guide_for_url_summary() {
        let composed = compose_system_prompt(url_summary_system_prompt(), None);
        assert_eq!(composed, url_summary_system_prompt());
    }
}
