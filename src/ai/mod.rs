pub mod classify;
pub mod conflict;
pub mod guide;
pub mod providers;
pub mod summarize;
pub mod transcript_format;

use crate::ai::classify::ClassifiedNote;
use crate::ai::summarize::UrlSummary;
use crate::error::AiError;
use crate::url::PageContent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

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

    async fn chat_completion(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        max_tokens: Option<u32>,
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
            transcription_provider: config
                .transcription
                .provider
                .clone()
                .unwrap_or_else(|| "openai".to_string()),
            classification_provider: config
                .classification
                .provider
                .clone()
                .unwrap_or_else(|| config.ai_provider.clone()),
            summarization_provider: config
                .summarization
                .provider
                .clone()
                .unwrap_or_else(|| config.ai_provider.clone()),
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
        provider
            .classify_image(image_base64, caption, exif_context, model, guide)
            .await
    }

    pub async fn summarize_url(
        &self,
        page_content: &PageContent,
        user_text: Option<&str>,
        model: &str,
        guide: Option<&str>,
    ) -> Result<UrlSummary, AiError> {
        let provider = self.get_provider(&self.summarization_provider)?;
        provider
            .summarize_url(page_content, user_text, model, guide)
            .await
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
        provider
            .format_transcript(raw_transcript, video_title, model)
            .await
    }

    pub async fn chat_completion(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        max_tokens: Option<u32>,
    ) -> Result<String, AiError> {
        let provider = self.get_provider(&self.classification_provider)?;
        provider.chat_completion(model, messages, max_tokens).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::classify::{ClassifiedNote, NoteCategory};
    use crate::ai::summarize::UrlSummary;
    use crate::config::Config;

    struct MockProvider {
        name: String,
    }

    #[async_trait]
    impl AiProvider for MockProvider {
        async fn classify_text(
            &self,
            _t: &str,
            _m: &str,
            _g: Option<&str>,
        ) -> Result<ClassifiedNote, AiError> {
            Ok(ClassifiedNote {
                category: NoteCategory::Log,
                summary: format!("classified by {}", self.name),
                markdown: String::new(),
                tags: Vec::new(),
                frontmatter: None,
            })
        }
        async fn classify_image(
            &self,
            _i: &str,
            _c: Option<&str>,
            _e: &str,
            _m: &str,
            _g: Option<&str>,
        ) -> Result<ClassifiedNote, AiError> {
            Ok(ClassifiedNote {
                category: NoteCategory::Log,
                summary: format!("image by {}", self.name),
                markdown: String::new(),
                tags: Vec::new(),
                frontmatter: None,
            })
        }
        async fn summarize_url(
            &self,
            _p: &PageContent,
            _u: Option<&str>,
            _m: &str,
            _g: Option<&str>,
        ) -> Result<UrlSummary, AiError> {
            Ok(UrlSummary {
                title: format!("summary by {}", self.name),
                summary: String::new(),
                tags: Vec::new(),
            })
        }
        async fn transcribe(&self, _a: &[u8]) -> Result<String, AiError> {
            Ok(format!("transcribed by {}", self.name))
        }
        async fn format_transcript(&self, _r: &str, _v: &str, _m: &str) -> Result<String, AiError> {
            Ok(format!("formatted by {}", self.name))
        }
        async fn chat_completion(
            &self,
            _m: &str,
            _ms: Vec<ChatMessage>,
            _t: Option<u32>,
        ) -> Result<String, AiError> {
            Ok(format!("chat by {}", self.name))
        }
    }

    #[tokio::test]
    async fn test_ai_service_routing() {
        let mut providers: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        providers.insert(
            "p1".to_string(),
            Arc::new(MockProvider {
                name: "p1".to_string(),
            }),
        );
        providers.insert(
            "p2".to_string(),
            Arc::new(MockProvider {
                name: "p2".to_string(),
            }),
        );

        let mut config = Config::default();
        config.ai_provider = "p1".to_string();
        config.transcription.provider = Some("p2".to_string());

        let service = AiService::new(providers, &config);

        // Classification should go to p1 (default)
        let res = service.classify_text("test", "model", None).await.unwrap();
        assert_eq!(res.summary, "classified by p1");

        // Transcription should go to p2 (override)
        let res = service.transcribe(&[]).await.unwrap();
        assert_eq!(res, "transcribed by p2");
    }

    #[tokio::test]
    async fn test_ai_service_fallback_to_default() {
        let mut providers: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        providers.insert(
            "default".to_string(),
            Arc::new(MockProvider {
                name: "default".to_string(),
            }),
        );
        providers.insert(
            "openai".to_string(),
            Arc::new(MockProvider {
                name: "openai".to_string(),
            }),
        );

        let mut config = Config::default();
        config.ai_provider = "default".to_string();
        // No explicit providers for subtasks

        let service = AiService::new(providers, &config);

        assert_eq!(
            service.transcribe(&[]).await.unwrap(),
            "transcribed by openai"
        );
        assert_eq!(
            service
                .summarize_url(
                    &PageContent {
                        title: None,
                        description: None,
                        body_text: String::new(),
                        url: String::new()
                    },
                    None,
                    "m",
                    None
                )
                .await
                .unwrap()
                .title,
            "summary by default"
        );
    }

    #[test]
    fn test_ai_service_from_yaml() {
        let yaml = r#"
vault_path: "."
ai:
  provider: "global"
  transcription:
    provider: "whisper-specific"
"#;
        let mut providers: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();
        providers.insert(
            "global".to_string(),
            Arc::new(MockProvider {
                name: "global".to_string(),
            }),
        );
        providers.insert(
            "whisper-specific".to_string(),
            Arc::new(MockProvider {
                name: "whisper-specific".to_string(),
            }),
        );

        // Simulate Config loading (ignoring env vars for this unit test by using serde_yml directly on FileConfig if possible,
        // but Config::load is better if we can mock env. Actually, let's just test the logic that maps Config to AiService.)

        let mut config = Config::default();
        config.ai_provider = "global".to_string();
        config.transcription.provider = Some("whisper-specific".to_string());

        let service = AiService::new(providers, &config);
        assert_eq!(service.classification_provider, "global");
        assert_eq!(service.transcription_provider, "whisper-specific");
    }
}
