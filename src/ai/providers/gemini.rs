use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::ai::classify::ClassifiedNote;
use crate::ai::summarize::UrlSummary;
use crate::ai::{AiProvider, ChatMessage};
use crate::error::AiError;
use crate::url::PageContent;

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const MAX_RETRIES: u32 = 3;
const REQUEST_TIMEOUT_SECS: u64 = 120;

// Type alias for yup-oauth2's default Authenticator to avoid typing long generic hyper structs
type YupAuthenticator = yup_oauth2::authenticator::Authenticator<
    yup_oauth2::hyper_rustls::HttpsConnector<yup_oauth2::hyper::client::HttpConnector>,
>;

/// Google Gemini AI provider implementation
pub struct GeminiClient {
    http: Client,
    api_key: Option<String>,
    oauth_authenticator: Option<YupAuthenticator>,
}

impl std::fmt::Debug for GeminiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiClient")
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field(
                "oauth_authenticator",
                &self.oauth_authenticator.as_ref().map(|_| "Some"),
            )
            .finish()
    }
}

impl GeminiClient {
    pub async fn new(
        api_key: Option<String>,
        service_account_path: Option<std::path::PathBuf>,
    ) -> Result<Self, AiError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .pool_max_idle_per_host(5)
            .build()?;

        let mut oauth_authenticator = None;

        if let Some(path) = service_account_path {
            info!(path = %path.display(), "Initializing Gemini with Service Account OAuth2");
            let secret = yup_oauth2::read_service_account_key(&path)
                .await
                .map_err(|e| {
                    AiError::UnsupportedCapability(format!(
                        "Failed to read Service Account key: {}",
                        e
                    ))
                })?;

            let auth = yup_oauth2::ServiceAccountAuthenticator::builder(secret)
                .build()
                .await
                .map_err(|e| {
                    AiError::UnsupportedCapability(format!(
                        "Failed to build Service Account Authenticator: {}",
                        e
                    ))
                })?;

            oauth_authenticator = Some(auth);
        } else if api_key.is_none() {
            // No API key or SA path provided: try to fall back to Application Default Credentials (ADC)
            info!("No Gemini API key or Service Account path provided. Trying Application Default Credentials (ADC) OAuth2...");
            let opts = yup_oauth2::ApplicationDefaultCredentialsFlowOpts::default();
            match yup_oauth2::ApplicationDefaultCredentialsAuthenticator::builder(opts).await {
                yup_oauth2::authenticator::ApplicationDefaultCredentialsTypes::ServiceAccount(
                    builder,
                ) => match builder.build().await {
                    Ok(auth) => {
                        info!("Successfully initialized Gemini with Service Account ADC OAuth2");
                        oauth_authenticator = Some(auth);
                    }
                    Err(e) => {
                        warn!(error = %e, "Could not build Service Account ADC authenticator");
                    }
                },
                yup_oauth2::authenticator::ApplicationDefaultCredentialsTypes::InstanceMetadata(
                    builder,
                ) => match builder.build().await {
                    Ok(auth) => {
                        info!("Successfully initialized Gemini with Instance Metadata ADC OAuth2");
                        oauth_authenticator = Some(auth);
                    }
                    Err(e) => {
                        warn!(error = %e, "Could not build Instance Metadata ADC authenticator");
                    }
                },
            }
        } else {
            info!("Initializing Gemini with standard API Key authentication");
        }

        if api_key.is_none() && oauth_authenticator.is_none() {
            return Err(AiError::UnsupportedCapability(
                "Neither Gemini API key nor Google OAuth credentials could be configured. Please set GEMINI_API_KEY or authenticate via gcloud.".to_string()
            ));
        }

        Ok(Self {
            http,
            api_key,
            oauth_authenticator,
        })
    }

    /// Obtain active authorization token if authenticating via OAuth 2.0
    async fn get_oauth_token(&self) -> Result<String, AiError> {
        if let Some(ref auth) = self.oauth_authenticator {
            let scopes = &[
                "https://www.googleapis.com/auth/generative-language",
                "https://www.googleapis.com/auth/cloud-platform",
            ];
            let token_res = auth
                .token(scopes)
                .await
                .map_err(|e| AiError::ProviderError {
                    status: 401,
                    message: format!("Failed to acquire Google OAuth 2.0 token: {}", e),
                })?;

            if let Some(tok) = token_res.token() {
                return Ok(tok.to_string());
            }
        }
        Err(AiError::UnsupportedCapability(
            "OAuth authenticator is not configured".to_string(),
        ))
    }

    /// Make a chat generation request to Gemini with automatic retry
    async fn generate_content(&self, model: &str, body: &Value) -> Result<Value, AiError> {
        let clean_model = model.strip_prefix("google/").unwrap_or(model);
        let mut backoff = Duration::from_secs(1);

        for attempt in 0..=MAX_RETRIES {
            let url = format!("{}/models/{}:generateContent", GEMINI_BASE_URL, clean_model);
            let mut req = self
                .http
                .post(&url)
                .header("Content-Type", "application/json");

            // Attach OAuth bearer token or API key
            if self.oauth_authenticator.is_some() {
                match self.get_oauth_token().await {
                    Ok(token) => {
                        req = req.header("Authorization", format!("Bearer {}", token));
                    }
                    Err(e) => {
                        if attempt < MAX_RETRIES {
                            warn!(attempt = attempt + 1, error = %e, "Failed to get OAuth token, retrying");
                            sleep(backoff).await;
                            backoff *= 2;
                            continue;
                        }
                        return Err(e);
                    }
                }
            } else if let Some(ref key) = self.api_key {
                req = req.header("x-goog-api-key", key);
            }

            let response = req.json(body).send().await?;

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
                            "Rate limited by Gemini, retrying"
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
                            "Gemini server error, retrying"
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

    /// Extract generated text response from Gemini generateContent JSON response
    fn extract_content(response: &Value) -> Result<String, AiError> {
        // Gemini response structure path: candidates[0].content.parts[0].text
        response["candidates"]
            .get(0)
            .and_then(|c| c["content"]["parts"].get(0))
            .and_then(|p| p["text"].as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                AiError::ParseError(format!(
                    "No content found in Gemini response candidate parts. Raw response: {:?}",
                    response
                ))
            })
    }
}

/// Helper to parse data URI into MIME type and base64 data
fn parse_data_uri(data_uri: &str) -> (String, String) {
    if let Some(stripped) = data_uri.strip_prefix("data:") {
        if let Some(comma_idx) = stripped.find(',') {
            let header = &stripped[..comma_idx];
            let data = &stripped[comma_idx + 1..];
            let mime_type = header.split(';').next().unwrap_or("image/jpeg").to_string();
            return (mime_type, data.to_string());
        }
    }
    ("image/jpeg".to_string(), data_uri.to_string())
}

/// Convert a standard OpenAPI/JSON Schema to one compatible with Gemini REST API.
/// In particular, Gemini response schema does not support union types like `["object", "null"]`.
/// We map any type array to the first non-null type in the list.
fn convert_to_gemini_schema(mut schema: Value) -> Value {
    if let Some(obj) = schema.as_object_mut() {
        // Gemini REST API does not support additionalProperties in responseSchema
        obj.remove("additionalProperties");

        if let Some(t) = obj.get_mut("type") {
            if let Some(arr) = t.as_array() {
                let non_null_type = arr
                    .iter()
                    .find(|v| v.as_str() != Some("null"))
                    .cloned()
                    .unwrap_or_else(|| json!("string"));
                *t = non_null_type;
            }
        }

        if let Some(properties) = obj.get_mut("properties") {
            if let Some(prop_obj) = properties.as_object_mut() {
                for (_, prop_val) in prop_obj.iter_mut() {
                    *prop_val = convert_to_gemini_schema(prop_val.clone());
                }
            }
        }

        if let Some(items) = obj.get_mut("items") {
            *items = convert_to_gemini_schema(items.clone());
        }
    }
    schema
}

/// Extract Gemini schema from OpenAI format response format JSON
fn extract_gemini_schema(response_format: &Value) -> Option<Value> {
    let raw_schema = response_format
        .get("json_schema")
        .and_then(|js| js.get("schema"))?;
    Some(convert_to_gemini_schema(raw_schema.clone()))
}

#[async_trait]
impl AiProvider for GeminiClient {
    async fn classify_text(
        &self,
        text: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError> {
        info!(
            text_length = text.len(),
            model = model,
            "Classifying text via Gemini"
        );

        let system_prompt = crate::ai::guide::compose_system_prompt(
            crate::ai::classify::CLASSIFICATION_SYSTEM_PROMPT,
            guide,
        );

        let response_format = crate::ai::classify::classified_note_response_format();
        let gemini_schema = extract_gemini_schema(&response_format);

        let mut generation_config = json!({
            "response_mime_type": "application/json"
        });
        if let Some(schema) = gemini_schema {
            generation_config["responseSchema"] = schema;
        }

        let body = json!({
            "system_instruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": [{
                "role": "user",
                "parts": [{ "text": text }]
            }],
            "generationConfig": generation_config
        });

        let response = self.generate_content(model, &body).await?;
        let content = Self::extract_content(&response)?;

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
        info!(model = model, "Classifying image via Gemini");

        let base_prompt = format!(
            "{}\n\nYou are also receiving an image. Describe what you see and classify it. If a caption is provided, use it as primary context. Include the image description in the markdown output as a short paragraph.",
            crate::ai::classify::CLASSIFICATION_SYSTEM_PROMPT
        );
        let system_prompt = crate::ai::guide::compose_system_prompt(&base_prompt, guide);

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

        let (mime_type, raw_data) = parse_data_uri(image_base64);

        let response_format = crate::ai::classify::classified_note_response_format();
        let gemini_schema = extract_gemini_schema(&response_format);

        let mut generation_config = json!({
            "response_mime_type": "application/json"
        });
        if let Some(schema) = gemini_schema {
            generation_config["responseSchema"] = schema;
        }

        let body = json!({
            "system_instruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": [{
                "role": "user",
                "parts": [
                    {
                        "inline_data": {
                            "mime_type": mime_type,
                            "data": raw_data
                        }
                    },
                    {
                        "text": text_content
                    }
                ]
            }],
            "generationConfig": generation_config
        });

        let response = self.generate_content(model, &body).await?;
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
        info!(url = %page_content.url, model = model, "Summarizing URL via Gemini");

        let system_prompt = crate::ai::guide::compose_system_prompt(
            crate::ai::summarize::url_summary_system_prompt(),
            guide,
        );

        let mut text_content = format!(
            "URL: {}\nTitle: {}\nDescription: {}\nContent:\n{}",
            page_content.url,
            page_content.title.as_deref().unwrap_or("[No Title]"),
            page_content
                .description
                .as_deref()
                .unwrap_or("[No Description]"),
            page_content.body_text
        );

        if let Some(user_prompt) = user_text {
            text_content.push_str(&format!("\n\nUser instructions:\n{}", user_prompt));
        }

        let response_format = crate::ai::summarize::url_summary_response_format();
        let gemini_schema = extract_gemini_schema(&response_format);

        let mut generation_config = json!({
            "response_mime_type": "application/json"
        });
        if let Some(schema) = gemini_schema {
            generation_config["responseSchema"] = schema;
        }

        let body = json!({
            "system_instruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": [{
                "role": "user",
                "parts": [{ "text": text_content }]
            }],
            "generationConfig": generation_config
        });

        let response = self.generate_content(model, &body).await?;
        let content = Self::extract_content(&response)?;

        serde_json::from_str(&content).map_err(|e| {
            AiError::SummarizationFailed(format!(
                "Failed to parse URL summary JSON: {}. Raw: {}",
                e, content
            ))
        })
    }

    async fn transcribe(&self, _audio_bytes: &[u8]) -> Result<String, AiError> {
        Err(AiError::UnsupportedCapability(
            "Transcription not supported by Gemini provider natively".to_string(),
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
            "Formatting transcript via Gemini"
        );

        let guide_path = crate::ai::transcript_format::default_guide_path();
        let guide_content = crate::ai::guide::load_guide(&Some(guide_path))
            .await
            .unwrap_or_default();

        let system_prompt = format!(
            "You are a transcription formatting assistant. Your task is to transform raw transcripts into a structured, readable markdown summary. Follow this style guide: {}",
            guide_content
        );

        let text_content = format!(
            "Video Title: {}\nRaw Transcript:\n{}",
            video_title, raw_transcript
        );

        let body = json!({
            "system_instruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": [{
                "role": "user",
                "parts": [{ "text": text_content }]
            }]
        });

        let response = self.generate_content(model, &body).await?;
        Self::extract_content(&response)
    }

    async fn chat_completion(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        max_tokens: Option<u32>,
    ) -> Result<String, AiError> {
        let mut contents = Vec::new();
        let mut system_instruction = None;

        for m in messages {
            if m.role == "system" {
                system_instruction = Some(json!({
                    "parts": [{ "text": m.content }]
                }));
            } else {
                let role = if m.role == "assistant" {
                    "model"
                } else {
                    "user"
                };
                contents.push(json!({
                    "role": role,
                    "parts": [{ "text": m.content }]
                }));
            }
        }

        let mut body = json!({
            "contents": contents
        });

        if let Some(sys) = system_instruction {
            body["system_instruction"] = sys;
        }

        if let Some(tokens) = max_tokens {
            body["generationConfig"] = json!({
                "maxOutputTokens": tokens
            });
        }

        let response = self.generate_content(model, &body).await?;
        Self::extract_content(&response)
    }

    async fn transcribe_pdf(
        &self,
        pdf_bytes: &[u8],
        user_prompt: Option<&str>,
        model: &str,
    ) -> Result<ClassifiedNote, AiError> {
        info!(model = model, bytes_len = pdf_bytes.len(), "Transcribing PDF via Gemini Multimodal");

        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let base64_data = STANDARD.encode(pdf_bytes);

        let system_prompt = "You are a highly precise document transcription and classification assistant. \
You are receiving a PDF document. Your tasks are:
1. **Transcribe** the entire document's text as accurately as possible. If there are tables, preserve their structure in markdown tables. If there are images, describe them inline.
2. **Summarize** the document in a concise, human-readable summary of 1-3 sentences.
3. **Classify** and tag the content.
4. **Output your result in JSON format ONLY**, matching this schema exactly:
{
  \"category\": \"note\",
  \"summary\": \"Concise 1-3 sentence summary of the document\",
  \"markdown\": \"The full and detailed transcription of the document in markdown format\",
  \"tags\": [\"tag1\", \"tag2\"]
}";

        let text_content = if let Some(p) = user_prompt {
            format!("Transcribe the attached PDF document. User instruction/context: {}", p)
        } else {
            "Transcribe the attached PDF document.".to_string()
        };

        let response_format = crate::ai::classify::classified_note_response_format();
        let gemini_schema = extract_gemini_schema(&response_format);

        let mut generation_config = json!({
            "response_mime_type": "application/json"
        });
        if let Some(schema) = gemini_schema {
            generation_config["responseSchema"] = schema;
        }

        let body = json!({
            "system_instruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": [{
                "role": "user",
                "parts": [
                    {
                        "inline_data": {
                            "mime_type": "application/pdf",
                            "data": base64_data
                        }
                    },
                    {
                        "text": text_content
                    }
                ]
            }],
            "generationConfig": generation_config
        });

        let response = self.generate_content(model, &body).await?;
        let content = Self::extract_content(&response)?;

        crate::ai::classify::parse_classification_with_fallback(&content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_data_uri_jpeg() {
        let uri = "data:image/jpeg;base64,/9j/4AAQSkZJRg==";
        let (mime, data) = parse_data_uri(uri);
        assert_eq!(mime, "image/jpeg");
        assert_eq!(data, "/9j/4AAQSkZJRg==");
    }

    #[test]
    fn test_parse_data_uri_png() {
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
        let (mime, data) = parse_data_uri(uri);
        assert_eq!(mime, "image/png");
        assert_eq!(data, "iVBORw0KGgoAAAANSUhEUg==");
    }

    #[test]
    fn test_parse_data_uri_plain() {
        let uri = "/9j/4AAQSkZJRg==";
        let (mime, data) = parse_data_uri(uri);
        assert_eq!(mime, "image/jpeg");
        assert_eq!(data, "/9j/4AAQSkZJRg==");
    }

    #[test]
    fn test_convert_to_gemini_schema_removes_null_union_and_additional_properties() {
        let input_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "category": {
                    "type": "string"
                },
                "frontmatter": {
                    "type": ["object", "null"],
                    "additionalProperties": true
                }
            }
        });

        let output = convert_to_gemini_schema(input_schema);
        
        assert_eq!(output["properties"]["frontmatter"]["type"], json!("object"));
        assert!(output.get("additionalProperties").is_none());
        assert!(output["properties"]["frontmatter"].get("additionalProperties").is_none());
    }

    #[test]
    fn test_extract_gemini_schema_success() {
        let response_format = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "test_schema",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": {
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                }
            }
        });

        let schema = extract_gemini_schema(&response_format).expect("should extract");
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["properties"]["tags"]["type"], json!("array"));
        assert_eq!(schema["properties"]["tags"]["items"]["type"], json!("string"));
    }
}
