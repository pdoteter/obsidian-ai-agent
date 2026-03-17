use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::json;
use std::path::Path;
use tracing::info;

use super::client::OpenRouterClient;
use crate::error::AiError;

impl OpenRouterClient {
    /// Transcribe an audio file (WAV) using OpenRouter with a multimodal model.
    /// The audio is sent as base64-encoded data in the input_audio content part.
    pub async fn transcribe_audio(
        &self,
        audio_path: &Path,
        model: &str,
    ) -> Result<String, AiError> {
        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| AiError::TranscriptionFailed(format!("Failed to read audio file: {}", e)))?;

        let base64_audio = BASE64.encode(&audio_bytes);

        let format = match audio_path.extension().and_then(|e| e.to_str()) {
            Some("wav") => "wav",
            Some("mp3") => "mp3",
            Some("ogg") | Some("oga") => "ogg",
            Some("flac") => "flac",
            Some("m4a") => "m4a",
            _ => "wav",
        };

        info!(
            model = model,
            audio_format = format,
            audio_size_bytes = audio_bytes.len(),
            "Sending audio for transcription"
        );

        let body = json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a transcription assistant. Transcribe the audio exactly as spoken. Output only the transcription text, nothing else. If the audio is in Dutch, transcribe in Dutch. If in English, transcribe in English. Preserve the original language."
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Transcribe this audio message."
                        },
                        {
                            "type": "input_audio",
                            "input_audio": {
                                "data": base64_audio,
                                "format": format
                            }
                        }
                    ]
                }
            ]
        });

        let response = self.chat_completion(body).await?;
        let transcript = Self::extract_content(&response)?;

        if transcript.trim().is_empty() {
            return Err(AiError::TranscriptionFailed(
                "Empty transcription returned".to_string(),
            ));
        }

        info!(
            transcript_length = transcript.len(),
            "Audio transcription complete"
        );

        Ok(transcript.trim().to_string())
    }
}
