pub mod classify;
pub mod client;
pub mod conflict;
pub mod guide;
pub mod providers;
pub mod summarize;
pub mod transcribe;
pub mod transcript_format;

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use crate::error::AiError;
use crate::url::PageContent;
use crate::ai::classify::ClassifiedNote;
use crate::ai::summarize::UrlSummary;

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn classify_text(
        &self,
        text: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError>;

    async fn classify_image(
        &self,
        image_base64: &str,
        caption: Option<&str>,
        exif_context: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError>;

    async fn summarize_url(
        &self,
        page_content: &PageContent,
        user_text: Option<&str>,
        model: &str,
        guide: Option<&str>,
    ) -> Result<UrlSummary, AiError>;

    async fn transcribe(&self, audio_bytes: &[u8]) -> Result<String, AiError>;

    async fn format_transcript(
        &self,
        raw_transcript: &str,
        video_title: &str,
        model: &str,
    ) -> Result<String, AiError>;
}

/// Orchestrator that routes AI tasks to the configured providers
pub struct AiService {
    providers: HashMap<String, Arc<dyn AiProvider>>,
    transcription_provider: String,
    classification_provider: String,
    summarization_provider: String,
}

impl AiService {
    pub fn new(
        providers: HashMap<String, Arc<dyn AiProvider>>,
        config: &crate::config::Config,
    ) -> Self {
        Self {
            providers,
            transcription_provider: config.transcription.provider.clone().unwrap_or_else(|| config.ai_provider.clone()),
            classification_provider: config.classification.provider.clone().unwrap_or_else(|| config.ai_provider.clone()),
            summarization_provider: config.summarization.provider.clone().unwrap_or_else(|| config.ai_provider.clone()),
        }
    }

    fn get_provider(&self, name: &str) -> Result<Arc<dyn AiProvider>, AiError> {
        self.providers.get(name).cloned().ok_or_else(|| {
            AiError::UnsupportedCapability(format!("AI Provider '{}' not found", name))
        })
    }

    pub async fn classify_text(
        &self,
        text: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError> {
        let provider = self.get_provider(&self.classification_provider)?;
        provider.classify_text(text, model, guide).await
    }

    pub async fn classify_image(
        &self,
        image_base64: &str,
        caption: Option<&str>,
        exif_context: &str,
        model: &str,
        guide: Option<&str>,
    ) -> Result<ClassifiedNote, AiError> {
        let provider = self.get_provider(&self.classification_provider)?;
        provider.classify_image(image_base64, caption, exif_context, model, guide).await
    }

    pub async fn summarize_url(
        &self,
        page_content: &PageContent,
        user_text: Option<&str>,
        model: &str,
        guide: Option<&str>,
    ) -> Result<UrlSummary, AiError> {
        let provider = self.get_provider(&self.summarization_provider)?;
        provider.summarize_url(page_content, user_text, model, guide).await
    }

    pub async fn transcribe(&self, audio_bytes: &[u8]) -> Result<String, AiError> {
        let provider = self.get_provider(&self.transcription_provider)?;
        provider.transcribe(audio_bytes).await
    }

    pub async fn format_transcript(
        &self,
        raw_transcript: &str,
        video_title: &str,
        model: &str,
    ) -> Result<String, AiError> {
        let provider = self.get_provider(&self.classification_provider)?;
        provider.format_transcript(raw_transcript, video_title, model).await
    }
}
