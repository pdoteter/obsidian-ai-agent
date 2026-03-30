use crate::ai::client::OpenRouterClient;
use crate::ai::guide::{compose_system_prompt, load_guide};
use crate::error::AiError;
use serde_json::json;
use std::path::PathBuf;
use tracing::debug;

const DEFAULT_GUIDE_PATH: &str = "./system-guide.md";

impl OpenRouterClient {
    pub async fn format_transcript(
        &self,
        raw_transcript: &str,
        video_title: &str,
        model: &str,
    ) -> Result<String, AiError> {
        debug!(
            transcript_length = raw_transcript.len(),
            video_title = video_title,
            model = model,
            "Formatting transcript"
        );

        let guide_path = Some(PathBuf::from(DEFAULT_GUIDE_PATH));
        let guide_content = load_guide(&guide_path).unwrap_or_default();
        let body = build_transcript_format_body(model, video_title, raw_transcript, &guide_content);

        let response = self.chat_completion(body).await?;
        let formatted_text = Self::extract_content(&response)?;

        debug!(
            formatted_length = formatted_text.len(),
            video_title = video_title,
            "Transcript formatted"
        );

        Ok(formatted_text)
    }
}

fn transcript_format_system_prompt() -> &'static str {
    "You are a transcript formatter. Your task is to reformat raw video transcript text into human-readable structured content.

Rules:
- Add paragraph breaks where natural topic shifts occur
- Add topic headings using ### Markdown syntax for major topics
- Preserve ALL content — do NOT summarize or omit anything
- Do NOT add commentary, opinions, or meta-discussion
- Output plain Markdown text only

The goal is readability, not summarization."
}

fn build_transcript_format_body(
    model: &str,
    video_title: &str,
    raw_transcript: &str,
    guide_content: &str,
) -> serde_json::Value {
    let guide = if guide_content.trim().is_empty() {
        None
    } else {
        Some(guide_content)
    };

    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": compose_system_prompt(transcript_format_system_prompt(), guide)
            },
            {
                "role": "user",
                "content": format!(
                    "Video Title: {}\n\nRaw Transcript:\n{}",
                    video_title, raw_transcript
                )
            }
        ],
        "max_tokens": 8192
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_build_transcript_format_body_has_plain_text_structure_without_response_format() {
        let body = build_transcript_format_body(
            "google/gemini-2.5-flash",
            "Rust async deep dive",
            "line one line two",
            "Keep headings concise",
        );

        assert_eq!(body["model"], json!("google/gemini-2.5-flash"));
        assert_eq!(body["messages"][0]["role"], json!("system"));
        assert_eq!(body["messages"][1]["role"], json!("user"));
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn test_transcript_format_system_prompt_contains_paragraph_and_heading_rules() {
        let prompt = transcript_format_system_prompt();

        assert!(prompt.contains("Add paragraph breaks where natural topic shifts occur"));
        assert!(prompt.contains("Add topic headings using ### Markdown syntax for major topics"));
        assert!(prompt.contains("do NOT summarize or omit anything"));
        assert!(prompt.contains("The goal is readability, not summarization."));
    }

    #[test]
    fn test_build_transcript_format_body_uses_requested_model() {
        let body = build_transcript_format_body(
            "anthropic/claude-haiku-3.5",
            "Video",
            "raw transcript",
            "",
        );

        assert_eq!(body["model"], json!("anthropic/claude-haiku-3.5"));
    }

    #[test]
    fn test_build_transcript_format_body_uses_large_max_tokens() {
        let body = build_transcript_format_body("google/gemini-2.5-flash", "Video", "raw", "");

        let max_tokens = body["max_tokens"].as_i64().unwrap_or_default();
        assert!(max_tokens >= 8192);
    }

    #[test]
    fn test_build_transcript_format_body_user_prompt_contains_title_and_transcript() {
        let body = build_transcript_format_body(
            "google/gemini-2.5-flash",
            "My Video",
            "hello world transcript",
            "",
        );

        let user_content = body["messages"][1]["content"].as_str().unwrap_or_default();
        assert!(user_content.contains("Video Title: My Video"));
        assert!(user_content.contains("Raw Transcript:"));
        assert!(user_content.contains("hello world transcript"));
    }
}
