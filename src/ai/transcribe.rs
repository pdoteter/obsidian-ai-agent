use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

use crate::error::AiError;

const WHISPER_API_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// OpenAI Whisper API client for audio transcription.
#[derive(Debug, Clone)]
pub struct WhisperClient {
    http: Client,
    api_key: String,
    model: String,
}

#[derive(Deserialize)]
struct WhisperResponse {
    text: String,
}

impl WhisperClient {
    pub fn new(api_key: String, model: String) -> Result<Self, AiError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()?;

        Ok(Self {
            http,
            api_key,
            model,
        })
    }

    /// Transcribe raw audio bytes (Ogg Opus / .oga) using OpenAI Whisper API.
    pub async fn transcribe(&self, audio_bytes: &[u8]) -> Result<String, AiError> {
        info!(
            audio_size_bytes = audio_bytes.len(),
            model = %self.model,
            "Sending audio to Whisper API"
        );

        let file_part = reqwest::multipart::Part::bytes(audio_bytes.to_vec())
            .file_name("audio.oga")
            .mime_str("audio/ogg")
            .map_err(|e| AiError::TranscriptionFailed(format!("Failed to build multipart: {}", e)))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .part("file", file_part);

        let response = self
            .http
            .post(WHISPER_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AiError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let whisper_response: WhisperResponse = response.json().await.map_err(|e| {
            AiError::TranscriptionFailed(format!("Failed to parse Whisper response: {}", e))
        })?;

        let transcript = whisper_response.text.trim().to_string();

        if transcript.is_empty() {
            return Err(AiError::TranscriptionFailed(
                "Empty transcription returned".to_string(),
            ));
        }

        info!(
            transcript_length = transcript.len(),
            "Whisper transcription complete"
        );

        Ok(transcript)
    }
}
