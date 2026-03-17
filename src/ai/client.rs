use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

use crate::error::AiError;

const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const MAX_RETRIES: u32 = 3;
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// Shared OpenRouter API client with retry logic
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
    pub async fn chat_completion(&self, body: Value) -> Result<Value, AiError> {
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
                    return Err(AiError::ApiError {
                        status: status.as_u16(),
                        message: error_text,
                    });
                }
                status => {
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(AiError::ApiError {
                        status: status.as_u16(),
                        message: error_text,
                    });
                }
            }
        }

        Err(AiError::MaxRetriesExceeded(MAX_RETRIES))
    }

    /// Extract the content string from a chat completion response
    pub fn extract_content(response: &Value) -> Result<String, AiError> {
        response["choices"]
            .get(0)
            .and_then(|c| c["message"]["content"].as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| AiError::ParseError("No content in response".to_string()))
    }
}
