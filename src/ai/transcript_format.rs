use crate::ai::guide::{compose_system_prompt};
use crate::error::AiError;
use serde_json::json;
use std::path::PathBuf;

pub const DEFAULT_GUIDE_PATH: &str = "./system-guide.md";

pub fn default_guide_path() -> PathBuf {
    PathBuf::from(DEFAULT_GUIDE_PATH)
}

pub fn transcript_format_system_prompt() -> &'static str {
    "You are a transcript formatter. Your task is to reformat raw video transcript text into human-readable structured content.

Rules:
- Add paragraph breaks where natural topic shifts occur
- Add topic headings using ### Markdown syntax for major topics
- Preserve ALL content — do NOT summarize or omit anything
- Do NOT add commentary, opinions, or meta-discussion
- Output plain Markdown text only

The goal is readability, not summarization."
}

pub fn build_transcript_format_body(
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

    let system_prompt = compose_system_prompt(transcript_format_system_prompt(), guide);

    let user_prompt = format!(
        "Video Title: {}\n\nRaw Transcript:\n{}",
        video_title, raw_transcript
    );

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
        "max_tokens": 4096
    })
}
