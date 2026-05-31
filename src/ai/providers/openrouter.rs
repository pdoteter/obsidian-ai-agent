use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::ai::classify::ClassifiedNote;
use crate::ai::summarize::UrlSummary;
use crate::ai::AiProvider;
use crate::error::AiError;
use crate::url::PageContent;

const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const MAX_RETRIES: u32 = 3;
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// OpenRouter AI provider implementation
#[derive(Debug, Clone)]
pub struct OpenRouterClient {
    http: Client,
    api_key: String,
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Result<Self, AiError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .pool_max_idle_per_host(5)
            .build()?;

        Ok(Self { http, api_key })
    }

    /// Make a chat completion request with automatic retry on rate limits and server errors
    async fn chat_completion(&self, body: &Value) -> Result<Value, AiError> {
        let mut backoff = Duration::from_secs(1);

        for attempt in 0..=MAX_RETRIES {
            let response = self
                .http
                .post(format!("{}/chat/completions", OPENROUTER_BASE_URL))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("HTTP-Referer", "https://github.com/obsidian-ai-agent")
                .header("X-Title", "Obsidian AI Agent")
                .json(&body)
                .send()
                .await?;

            match response.status() {
                StatusCode::OK => {
                    let json: Value = response.json().await?;
                    return Ok(json);
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    if attempt < MAX_RETRIES {
                        let retry_after = response
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(backoff.as_secs());

                        warn!(
                            attempt = attempt + 1,
                            retry_after_secs = retry_after,
                            "Rate limited, retrying"
                        );
                        sleep(Duration::from_secs(retry_after)).await;
                        backoff *= 2;
                        continue;
                    }
                    return Err(AiError::RateLimited {
                        retry_after_secs: backoff.as_secs(),
                    });
                }
                status if status.is_server_error() => {
                    if attempt < MAX_RETRIES {
                        warn!(
                            attempt = attempt + 1,
                            status = %status,
                            backoff_secs = backoff.as_secs(),
                            "Server error, retrying"
                        );
                        sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(AiError::ProviderError {
                        status: status.as_u16(),
                        message: error_text,
                    });
                }
                status => {
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(AiError::ProviderError {
                        status: status.as_u16(),
                        message: error_text,
                    });
                }
            }
        }

        Err(AiError::MaxRetriesExceeded(MAX_RETRIES))
    }

    /// Extract the content string from a chat completion response
    fn extract_content(response: &Value) -> Result<String, AiError> {
        response["choices"]
            .get(0)
            .and_then(|c| c["message"]["content"].as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| AiError::ParseError("No content in response".to_string()))
    }
}

#[async_trait]
impl AiProvider for OpenRouterClient {
    async fn classify_text(
        &self,
        text: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError> {
        info!(
            text_length = text.len(),
            model = model,
            "Classifying text via OpenRouter"
        );
        // Note: CLASSIFICATION_SYSTEM_PROMPT and helper functions remain in ai/classify.rs for now
        // or we move them here if they are provider-specific.
        // Actually, they use response_format which is very OpenAI-like.

        // I'll re-implement the logic here, pulling from classify.rs
        let system_prompt = crate::ai::guide::compose_system_prompt(
            crate::ai::classify::CLASSIFICATION_SYSTEM_PROMPT,
            guide,
        );
        let body = crate::ai::classify::build_text_request_body(text, model, &system_prompt);

        let response = self.chat_completion(&body).await?;
        let content = Self::extract_content(&response)?;

        // Use the same robust parsing logic from classify.rs
        crate::ai::classify::parse_classification_with_fallback(&content)
    }

    async fn classify_image(
        &self,
        image_base64: &str,
        caption: Option<&str>,
        exif_context: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError> {
        info!(model = model, "Classifying image via OpenRouter");

        let base_prompt = format!(
            "{}\n\nYou are also receiving an image. Describe what you see and classify it. If a caption is provided, use it as primary context. Include the image description in the markdown output as a short paragraph.",
            crate::ai::classify::CLASSIFICATION_SYSTEM_PROMPT
        );
        let system_prompt = crate::ai::guide::compose_system_prompt(&base_prompt, guide);

        let body = crate::ai::classify::build_image_request_body(
            image_base64,
            caption,
            exif_context,
            model,
            &system_prompt,
        );
        let response = self.chat_completion(&body).await?;
        let content = Self::extract_content(&response)?;

        crate::ai::classify::parse_classification_with_fallback(&content)
    }

    async fn summarize_url(
        &self,
        page_content: &PageContent,
        user_text: Option<&str>,
        model: &str,
        guide: Option<&str>,
    ) -> Result<UrlSummary, AiError> {
        info!(url = %page_content.url, model = model, "Summarizing URL via OpenRouter");

        let system_prompt = crate::ai::guide::compose_system_prompt(
            crate::ai::summarize::url_summary_system_prompt(),
            guide,
        );
        let body = crate::ai::summarize::build_url_request_body(
            page_content,
            user_text,
            model,
            &system_prompt,
        );

        let response = self.chat_completion(&body).await?;
        let content = Self::extract_content(&response)?;

        serde_json::from_str(&content).map_err(|e| {
            AiError::SummarizationFailed(format!(
                "Failed to parse URL summary JSON: {}. Raw: {}",
                e, content
            ))
        })
    }

    async fn transcribe(&self, _audio_bytes: &[u8]) -> Result<String, AiError> {
        // OpenRouter doesn't support Whisper directly via chat completions yet in a standard way
        // that matches our current WhisperClient.
        Err(AiError::UnsupportedCapability(
            "Transcription not supported by OpenRouter provider".to_string(),
        ))
    }

    async fn format_transcript(
        &self,
        raw_transcript: &str,
        video_title: &str,
        model: &str,
    ) -> Result<String, AiError> {
        debug!(
            video_title = video_title,
            model = model,
            "Formatting transcript via OpenRouter"
        );

        let guide_path = crate::ai::transcript_format::default_guide_path();
        let guide_content = crate::ai::guide::load_guide(&Some(guide_path))
            .await
            .unwrap_or_default();
        let body = crate::ai::transcript_format::build_transcript_format_body(
            model,
            video_title,
            raw_transcript,
            &guide_content,
        );

        let response = self.chat_completion(&body).await?;
        let formatted_text = Self::extract_content(&response)?;

        Ok(formatted_text)
    }

    async fn chat_completion(
        &self,
        model: &str,
        messages: Vec<crate::ai::ChatMessage>,
        max_tokens: Option<u32>,
    ) -> Result<String, AiError> {
        let mut body = json!({
            "model": model,
            "messages": messages.into_iter().map(|m| json!({
                "role": m.role,
                "content": m.content
            })).collect::<Vec<_>>()
        });

        if let Some(tokens) = max_tokens {
            body["max_tokens"] = json!(tokens);
        }

        let response = self.chat_completion(&body).await?;
        Self::extract_content(&response)
    }

    async fn transcribe_pdf(
        &self,
        _pdf_bytes: &[u8],
        _user_prompt: Option<&str>,
        _model: &str,
    ) -> Result<ClassifiedNote, AiError> {
        Err(AiError::UnsupportedCapability(
            "PDF transcription not supported by OpenRouter provider".to_string(),
        ))
    }
}
