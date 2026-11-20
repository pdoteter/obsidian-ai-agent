use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatAction, InlineKeyboardButton, InlineKeyboardMarkup};
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::ai::AiService;
use crate::config::Config;
use crate::git::chat_tracker::ChatIdTracker;
use crate::git::debounce::SyncNotifier;
use crate::url::detect::{DetectedUrl, UrlType};
use crate::url::extract::fetch_page_content;
use crate::url::youtube::{fetch_youtube_description, fetch_youtube_metadata};
use crate::url::PageContent;
use crate::vault::daily_note::DailyNoteManager;
use crate::vault::writer;

#[derive(Clone, Debug)]
pub struct TranscriptRequest {
    pub video_id: String,
    pub url: String,
    pub title: String,
}

pub type TranscriptPending = Arc<Mutex<HashMap<String, TranscriptRequest>>>;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PendingTranscriptAction {
    pub id: String,
    pub video_id: String,
    pub url: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct TranscriptResult {
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub wiki_link: String,
}

#[derive(Clone, Debug)]
pub struct UrlItemOutcome {
    pub url: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub success: bool,
    pub transcript_wiki_link: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct UrlBatchResult {
    pub total_urls: usize,
    pub processed_count: usize,
    pub success_count: usize,
    pub truncated: bool,
    pub max_urls: usize,
    pub outcomes: Vec<UrlItemOutcome>,
    pub transcript_actions: Vec<PendingTranscriptAction>,
}

/// Fetch YouTube title via oEmbed with graceful fallback to the raw URL.
async fn fetch_youtube_title(detected_url: &DetectedUrl, config: &Config) -> String {
    match fetch_youtube_metadata(&detected_url.url, config.url.fetch_timeout_secs).await {
        Ok(metadata) => metadata.title,
        Err(e) => {
            warn!(
                error = %e,
                url = %detected_url.url,
                "Failed to fetch YouTube metadata for transcript, falling back to URL as title"
            );
            detected_url.url.clone()
        }
    }
}

/// Shared YouTube transcript processing pipeline for both Telegram and WebUI:
/// fetch VTT captions via yt-dlp -> AI summarize -> AI format -> save transcript note ->
/// update/append Daily Note entry -> notify git sync.
pub async fn process_transcript_request(
    request: &TranscriptRequest,
    config: &Config,
    ai_service: &AiService,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
    replace_existing: bool,
) -> Result<TranscriptResult, String> {
    let transcript_text = crate::url::fetch_transcript(&request.video_id)
        .await
        .map_err(|e| {
            error!(error = %e, video_id = %request.video_id, "Failed to fetch transcript");
            format!("Failed to fetch transcript: {}", e)
        })?;

    let page_content = PageContent {
        title: Some(request.title.clone()),
        description: Some("YouTube video transcript".to_string()),
        body_text: transcript_text.clone(),
        url: request.url.clone(),
    };

    let guide = crate::ai::guide::load_guide(&config.guide_path).await;
    let summary = ai_service
        .summarize_url(
            &page_content,
            None,
            &config.openrouter_model_classify,
            guide.as_deref(),
        )
        .await
        .map_err(|e| {
            error!(error = %e, url = %request.url, "Failed to summarize transcript");
            format!("Failed to summarize transcript: {}", e)
        })?;

    // Format transcript with AI before saving (graceful fallback to raw text)
    let formatted_transcript = match ai_service
        .format_transcript(
            &transcript_text,
            &request.title,
            &config.openrouter_model_classify,
        )
        .await
    {
        Ok(formatted) => formatted,
        Err(e) => {
            warn!(error = %e, "Failed to format transcript, using raw text");
            transcript_text.clone()
        }
    };

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let transcript_file = crate::vault::save_transcript(
        std::path::Path::new(&config.vault_path),
        &config.url.transcript_folder,
        &request.video_id,
        &request.title,
        &summary.summary,
        &formatted_transcript,
        &date,
    )
    .await
    .map_err(|e| {
        error!(error = %e, video_id = %request.video_id, "Failed to save transcript");
        format!("Failed to save transcript: {}", e)
    })?;

    let (section, content) = writer::format_url_todo(
        &request.url,
        Some(&summary.title),
        Some(&summary.summary),
        &summary.tags,
        Some(&transcript_file.wiki_link),
        Some(&request.title),
    );

    if replace_existing {
        if let Err(e) = vault
            .replace_entry_by_url(section, &request.url, &content)
            .await
        {
            warn!(error = %e, section, url = %request.url, "Failed to replace entry in daily note");
        }
    } else if let Err(e) = vault.append_to_section(section, &content).await {
        warn!(error = %e, section, url = %request.url, "Failed to append transcript entry to daily note");
    }

    if let Some(notifier) = sync_notifier {
        notifier.notify();
    }

    Ok(TranscriptResult {
        title: summary.title,
        summary: summary.summary,
        tags: summary.tags,
        wiki_link: transcript_file.wiki_link,
    })
}

/// Shared URL processing pipeline for WebUI (handles regular web pages, YouTube fast-mode
/// with pending on-demand transcript actions, and keyword-triggered direct YouTube transcripts).
#[allow(clippy::too_many_arguments)]
pub async fn process_url_entries(
    urls: Vec<DetectedUrl>,
    surrounding_text: Option<&str>,
    raw_text: &str,
    config: &Config,
    ai_service: &AiService,
    vault: &DailyNoteManager,
    sync_notifier: Option<&SyncNotifier>,
    transcript_pending: &TranscriptPending,
) -> UrlBatchResult {
    let max_urls = config.url.max_urls_per_message;
    let total_urls = urls.len();
    let (urls_to_process, truncated) = enforce_url_limit(urls, max_urls);
    let processed_count = urls_to_process.len();

    let mut success_count = 0usize;
    let mut outcomes = Vec::new();
    let mut transcript_actions = Vec::new();

    for detected_url in &urls_to_process {
        let is_transcript_direct_flow = match &detected_url.url_type {
            UrlType::YouTube { .. } => {
                crate::url::is_transcript_request(surrounding_text.unwrap_or(raw_text))
            }
            UrlType::WebPage => false,
        };

        if is_transcript_direct_flow {
            if let UrlType::YouTube { video_id } = &detected_url.url_type {
                let video_title = fetch_youtube_title(detected_url, config).await;
                let request = TranscriptRequest {
                    video_id: video_id.clone(),
                    url: detected_url.url.clone(),
                    title: video_title.clone(),
                };

                match process_transcript_request(&request, config, ai_service, vault, None, false)
                    .await
                {
                    Ok(transcript_result) => {
                        success_count += 1;
                        outcomes.push(UrlItemOutcome {
                            url: detected_url.url.clone(),
                            title: Some(video_title),
                            summary: Some(transcript_result.summary),
                            tags: transcript_result.tags,
                            success: true,
                            transcript_wiki_link: Some(transcript_result.wiki_link),
                            error: None,
                        });
                    }
                    Err(err_msg) => {
                        outcomes.push(UrlItemOutcome {
                            url: detected_url.url.clone(),
                            title: Some(video_title),
                            summary: None,
                            tags: Vec::new(),
                            success: false,
                            transcript_wiki_link: None,
                            error: Some(err_msg),
                        });
                    }
                }
            }
            continue;
        }

        // Normal fast-mode flow for non-transcript URLs
        let fetched_page = fetch_for_url_type(detected_url, config).await;

        let raw_youtube_title = if let Ok(page_content) = &fetched_page {
            if matches!(detected_url.url_type, UrlType::YouTube { .. }) {
                page_content.title.clone()
            } else {
                None
            }
        } else {
            None
        };

        let (title_for_todo, summary_for_todo, tags_for_todo) = match fetched_page {
            Ok(page_content) => {
                let fetched_title = page_content.title.clone();

                if let UrlType::YouTube { video_id } = &detected_url.url_type {
                    let uuid_str = Uuid::new_v4().to_string();
                    let short_id = crate::utils::safe_truncate(&uuid_str, 8).to_string();
                    let title_for_request = fetched_title
                        .clone()
                        .unwrap_or_else(|| detected_url.url.clone());

                    transcript_pending.lock().await.insert(
                        short_id.clone(),
                        TranscriptRequest {
                            video_id: video_id.clone(),
                            url: detected_url.url.clone(),
                            title: title_for_request.clone(),
                        },
                    );

                    transcript_actions.push(PendingTranscriptAction {
                        id: short_id,
                        video_id: video_id.clone(),
                        url: detected_url.url.clone(),
                        title: title_for_request,
                    });
                }

                let guide = crate::ai::guide::load_guide(&config.guide_path).await;

                match ai_service
                    .summarize_url(
                        &page_content,
                        surrounding_text,
                        &config.openrouter_model_classify,
                        guide.as_deref(),
                    )
                    .await
                {
                    Ok(summary) => {
                        info!(
                            url = %detected_url.url,
                            ai_title = %summary.title,
                            tags_count = summary.tags.len(),
                            "AI summarized URL"
                        );
                        (Some(summary.title), Some(summary.summary), summary.tags)
                    }
                    Err(e) => {
                        error!(error = %e, url = %detected_url.url, "AI summarization failed");
                        (fetched_title, None, Vec::new())
                    }
                }
            }
            Err(e) => {
                error!(error = %e, url = %detected_url.url, "Failed to fetch URL content");
                (None, None, Vec::new())
            }
        };

        let video_name = raw_youtube_title.as_deref();

        let (section, content) = writer::format_url_todo(
            &detected_url.url,
            title_for_todo.as_deref(),
            summary_for_todo.as_deref(),
            &tags_for_todo,
            None,
            video_name,
        );

        match vault.append_to_section(section, &content).await {
            Ok(_) => {
                success_count += 1;
                outcomes.push(UrlItemOutcome {
                    url: detected_url.url.clone(),
                    title: title_for_todo,
                    summary: summary_for_todo,
                    tags: tags_for_todo,
                    success: true,
                    transcript_wiki_link: None,
                    error: None,
                });
            }
            Err(e) => {
                error!(error = %e, url = %detected_url.url, "Failed to write URL TODO to vault");
                outcomes.push(UrlItemOutcome {
                    url: detected_url.url.clone(),
                    title: title_for_todo,
                    summary: summary_for_todo,
                    tags: tags_for_todo,
                    success: false,
                    transcript_wiki_link: None,
                    error: Some(format!("Failed to write URL TODO to vault: {}", e)),
                });
            }
        }
    }

    if success_count > 0 {
        if let Some(notifier) = sync_notifier {
            notifier.notify();
        }
    }

    UrlBatchResult {
        total_urls,
        processed_count,
        success_count,
        truncated,
        max_urls,
        outcomes,
        transcript_actions,
    }
}

/// Format a URL processing batch result into clean Markdown for the WebUI chat bubble.
pub fn format_webui_url_summary(batch: &UrlBatchResult) -> String {
    let mut parts = Vec::new();

    let header = if batch.success_count == batch.processed_count {
        format!("🔗 **Saved {} URL(s) as TODO(s)**", batch.success_count)
    } else {
        format!(
            "🔗 **Saved {}/{} URL(s) as TODO(s)**",
            batch.success_count, batch.processed_count
        )
    };
    parts.push(header);

    if batch.truncated {
        parts.push(format!(
            "⚠️ Message contained {} URLs (max: {}). Only processed first {}.",
            batch.total_urls, batch.max_urls, batch.max_urls
        ));
    }

    for item in &batch.outcomes {
        let label = item.title.as_deref().unwrap_or(&item.url);
        if item.success {
            let mut lines = vec![format!("✅ **{}**", label)];
            if let Some(ref wiki_link) = item.transcript_wiki_link {
                lines.push(format!("*Transcript*: `{}`", wiki_link));
            }
            if let Some(ref summary) = item.summary {
                lines.push(format!("*Summary*: \"{}\"", summary));
            }
            if !item.tags.is_empty() {
                let tags_str = item
                    .tags
                    .iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ");
                lines.push(format!("*Tags*: {}", tags_str));
            }
            parts.push(lines.join("\n"));
        } else {
            let reason = item.error.as_deref().unwrap_or("Failed to save URL");
            parts.push(format!("❌ **{}** — {}", label, reason));
        }
    }

    parts.join("\n\n")
}

/// Handle URLs found in Telegram messages
///
/// Pipeline: fetch content → AI summarize → format TODO → write to vault
#[allow(clippy::too_many_arguments)]
pub async fn handle_url_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<SyncNotifier>,
    chat_tracker: ChatIdTracker,
    transcript_pending: TranscriptPending,
    urls: Vec<DetectedUrl>,
    surrounding_text: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if urls.is_empty() {
        return Ok(());
    }

    // 1) auth check
    if let Some(user) = msg.from.as_ref() {
        if !config.is_user_allowed(user.id.0) {
            info!(
                user_id = user.id.0,
                "Unauthorized user, ignoring URL message"
            );
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    chat_tracker.set(msg.chat.id).await;

    // 2) enforce max URL limit
    let max_urls = config.url.max_urls_per_message;
    let total_urls = urls.len();
    let (urls_to_process, truncated) = enforce_url_limit(urls, max_urls);

    // 3) immediate processing message before network work
    let status_msg = bot
        .send_message(msg.chat.id, build_processing_message(urls_to_process.len()))
        .await?;

    // 4) process each URL with graceful degradation
    let mut success_count = 0usize;
    let mut results = Vec::new();
    let mut transcript_buttons: Vec<InlineKeyboardButton> = Vec::new();

    for detected_url in &urls_to_process {
        bot.send_chat_action(msg.chat.id, ChatAction::Typing)
            .await?;

        // Check if this is a YouTube URL with transcript keyword
        let is_transcript_direct_flow =
            if let UrlType::YouTube { video_id: _ } = &detected_url.url_type {
                let is_transcript_req = crate::url::is_transcript_request(
                    surrounding_text
                        .as_deref()
                        .unwrap_or(msg.text().unwrap_or("")),
                );
                is_transcript_req
            } else {
                false
            };

        // Direct transcript flow for keyword-triggered requests
        if is_transcript_direct_flow {
            if let UrlType::YouTube { video_id } = &detected_url.url_type {
                bot.send_message(msg.chat.id, "⏳ Fetching transcript...")
                    .await?;

                let video_title = fetch_youtube_title(detected_url, &config).await;
                let request = TranscriptRequest {
                    video_id: video_id.clone(),
                    url: detected_url.url.clone(),
                    title: video_title.clone(),
                };

                match process_transcript_request(
                    &request,
                    &config,
                    &ai_service,
                    &vault,
                    None, // sync_notifier is triggered once after processing all URLs
                    false,
                )
                .await
                {
                    Ok(transcript_result) => {
                        bot.send_message(
                            msg.chat.id,
                            format!("✅ Transcript saved: {}", transcript_result.wiki_link),
                        )
                        .await?;

                        success_count += 1;
                        results.push(format_result_item(
                            Some(&video_title),
                            &detected_url.url,
                            true,
                        ));
                    }
                    Err(e) => {
                        bot.send_message(msg.chat.id, format!("❌ {}", e)).await?;
                        results.push(format_result_item(
                            Some(&video_title),
                            &detected_url.url,
                            false,
                        ));
                    }
                }
            }
            continue;
        }

        // Normal fast-mode flow for non-transcript URLs
        let fetched_page = fetch_for_url_type(detected_url, &config).await;

        // Extract raw oEmbed title before AI processing for YouTube heading
        let raw_youtube_title = if let Ok(page_content) = &fetched_page {
            if matches!(detected_url.url_type, UrlType::YouTube { .. }) {
                page_content.title.clone()
            } else {
                None
            }
        } else {
            None
        };

        let (title_for_todo, summary_for_todo, tags_for_todo) = match fetched_page {
            Ok(page_content) => {
                let fetched_title = page_content.title.clone();

                if let UrlType::YouTube { video_id } = &detected_url.url_type {
                    // Show button for manual transcript request (not keyword-triggered)
                    let uuid_str = Uuid::new_v4().to_string();
                    let short_id = crate::utils::safe_truncate(&uuid_str, 8).to_string();
                    let title_for_request = fetched_title
                        .clone()
                        .unwrap_or_else(|| detected_url.url.clone());

                    transcript_pending.lock().await.insert(
                        short_id.clone(),
                        TranscriptRequest {
                            video_id: video_id.clone(),
                            url: detected_url.url.clone(),
                            title: title_for_request,
                        },
                    );

                    transcript_buttons.push(InlineKeyboardButton::callback(
                        "📝 Full Transcript",
                        format!("yt_transcript:{}", short_id),
                    ));
                }

                let guide = crate::ai::guide::load_guide(&config.guide_path).await;

                match ai_service
                    .summarize_url(
                        &page_content,
                        surrounding_text.as_deref(),
                        &config.openrouter_model_classify,
                        guide.as_deref(),
                    )
                    .await
                {
                    Ok(summary) => {
                        info!(
                            url = %detected_url.url,
                            ai_title = %summary.title,
                            tags_count = summary.tags.len(),
                            "AI summarized URL"
                        );
                        (Some(summary.title), Some(summary.summary), summary.tags)
                    }
                    Err(e) => {
                        // Graceful degradation: AI failed -> title-only fallback
                        error!(error = %e, url = %detected_url.url, "AI summarization failed");
                        (fetched_title, None, Vec::new())
                    }
                }
            }
            Err(e) => {
                // Graceful degradation: fetch failed -> plain URL fallback
                error!(error = %e, url = %detected_url.url, "Failed to fetch URL content");
                (None, None, Vec::new())
            }
        };

        // Wire video_name for YouTube URLs using raw oEmbed title
        let video_name = raw_youtube_title.as_deref();

        let (section, content) = writer::format_url_todo(
            &detected_url.url,
            title_for_todo.as_deref(),
            summary_for_todo.as_deref(),
            &tags_for_todo,
            None, // transcript integration is a later task
            video_name,
        );

        match vault.append_to_section(section, &content).await {
            Ok(_) => {
                success_count += 1;
                results.push(format_result_item(
                    title_for_todo.as_deref(),
                    &detected_url.url,
                    true,
                ));
            }
            Err(e) => {
                error!(error = %e, url = %detected_url.url, "Failed to write URL TODO to vault");
                results.push(format_result_item(
                    title_for_todo.as_deref(),
                    &detected_url.url,
                    false,
                ));
            }
        }
    }

    // 5) notify git sync once
    if success_count > 0 {
        if let Some(ref notifier) = sync_notifier {
            notifier.notify();
        }
    }

    // 6) edit processing message to final confirmation
    let confirmation = build_confirmation_message(
        success_count,
        urls_to_process.len(),
        total_urls,
        max_urls,
        truncated,
        &results,
    );

    let confirmation = truncate_confirmation_if_needed(confirmation);

    if transcript_buttons.is_empty() {
        bot.edit_message_text(msg.chat.id, status_msg.id, confirmation)
            .await?;
    } else {
        let rows: Vec<Vec<InlineKeyboardButton>> = transcript_buttons
            .into_iter()
            .map(|button| vec![button])
            .collect();
        let keyboard = InlineKeyboardMarkup::new(rows);

        bot.edit_message_text(msg.chat.id, status_msg.id, confirmation)
            .reply_markup(keyboard)
            .await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_transcript_callback(
    bot: Bot,
    q: CallbackQuery,
    transcript_pending: TranscriptPending,
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    config: Arc<Config>,
    sync_notifier: Option<SyncNotifier>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = q.data.as_ref().ok_or("No callback data")?;
    let short_id = data
        .strip_prefix("yt_transcript:")
        .ok_or("Invalid callback format")?;

    // Acknowledge callback immediately to stop Telegram loading spinner.
    bot.answer_callback_query(&q.id).await?;

    let chat_id = q.message.as_ref().map(|m| m.chat().id);

    let request = {
        let mut pending = transcript_pending.lock().await;
        pending.remove(short_id)
    };

    let request = match request {
        Some(req) => req,
        None => {
            if let Some(chat_id) = chat_id {
                bot.send_message(
                    chat_id,
                    "❌ Transcript request expired or not found. Please resend the YouTube link.",
                )
                .await?;
            }
            return Ok(());
        }
    };

    if let Some(chat_id) = chat_id {
        bot.send_message(chat_id, "⏳ Fetching transcript...")
            .await?;

        match process_transcript_request(
            &request,
            &config,
            &ai_service,
            &vault,
            sync_notifier.as_ref(),
            true,
        )
        .await
        {
            Ok(transcript_result) => {
                bot.send_message(
                    chat_id,
                    format!("✅ Transcript saved: {}", transcript_result.wiki_link),
                )
                .await?;
            }
            Err(e) => {
                bot.send_message(chat_id, format!("❌ {}", e)).await?;
            }
        }
    }

    Ok(())
}

fn enforce_url_limit(urls: Vec<DetectedUrl>, max_urls: usize) -> (Vec<DetectedUrl>, bool) {
    if urls.len() > max_urls {
        warn!(
            total_urls = urls.len(),
            max_urls = max_urls,
            "Message contains more URLs than configured limit, truncating"
        );
        (urls.into_iter().take(max_urls).collect(), true)
    } else {
        (urls, false)
    }
}

fn build_processing_message(count: usize) -> String {
    format!("🔗 Processing {} link(s)...", count)
}

const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

fn build_confirmation_message(
    success_count: usize,
    processed_count: usize,
    total_urls: usize,
    max_urls: usize,
    truncated: bool,
    results: &[String],
) -> String {
    let mut confirmation = if success_count == processed_count {
        format!("📝 Saved {} URL(s) as TODO(s)", success_count)
    } else {
        format!(
            "📝 Saved {}/{} URL(s) as TODO(s)",
            success_count, processed_count
        )
    };

    if truncated {
        confirmation.push_str(&format!(
            "\n\n⚠️ Message contained {} URLs (max: {}). Only processed first {}.",
            total_urls, max_urls, max_urls
        ));
    }

    if !results.is_empty() {
        confirmation.push_str("\n\n");
        confirmation.push_str(&results.join("\n"));
    }

    confirmation
}

fn truncate_confirmation_if_needed(confirmation: String) -> String {
    if confirmation.len() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return confirmation;
    }

    // Split into lines to identify header vs results
    let lines: Vec<&str> = confirmation.split('\n').collect();

    // Find where results section starts (first line with ✅ or ❌)
    let mut header_end_idx = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.contains("✅") || line.contains("❌") {
            header_end_idx = i;
            break;
        }
    }

    // If no result lines found, just hard truncate the entire message
    if header_end_idx == 0 {
        if confirmation.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
            let mut result =
                crate::utils::safe_truncate(&confirmation, TELEGRAM_MAX_MESSAGE_LENGTH - 3)
                    .to_string();
            result.push_str("...");
            return result;
        }
        return confirmation;
    }

    // Keep header, progressively remove result lines from the end
    let header_lines = &lines[..header_end_idx];
    let result_lines = &lines[header_end_idx..];

    let mut kept_results = result_lines.len();
    let mut message = header_lines.join("\n") + "\n\n" + &result_lines.join("\n");

    while message.len() > TELEGRAM_MAX_MESSAGE_LENGTH && kept_results > 0 {
        kept_results -= 1;
        message = header_lines.join("\n") + "\n\n" + &result_lines[..kept_results].join("\n");
    }

    let removed_count = result_lines.len() - kept_results;

    // Add truncation notice if results were removed
    if removed_count > 0 {
        message.push_str(&format!("\n\n... ({} more URLs not shown)", removed_count));
    }

    // Final safety: hard truncate if still too long
    if message.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
        let mut result =
            crate::utils::safe_truncate(&message, TELEGRAM_MAX_MESSAGE_LENGTH - 3).to_string();
        result.push_str("...");
        return result;
    }

    message
}

fn format_result_item(title: Option<&str>, url: &str, success: bool) -> String {
    let label = title.unwrap_or(url).chars().take(50).collect::<String>();
    if success {
        format!("✅ {}", label)
    } else {
        format!("❌ {} (write failed)", label)
    }
}

async fn fetch_for_url_type(
    detected_url: &DetectedUrl,
    config: &Config,
) -> Result<PageContent, Box<dyn std::error::Error + Send + Sync>> {
    match &detected_url.url_type {
        UrlType::YouTube { video_id } => {
            // Parallel fetch: metadata + description
            let (metadata_result, description_result) = tokio::join!(
                fetch_youtube_metadata(&detected_url.url, config.url.fetch_timeout_secs),
                fetch_youtube_description(video_id, config.url.fetch_timeout_secs)
            );

            // Metadata must succeed (same error handling as before)
            let metadata = metadata_result
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            info!(
                video_id = %video_id,
                fetched_video_id = %metadata.video_id,
                title = %metadata.title,
                has_thumbnail = metadata.thumbnail_url.is_some(),
                "Fetched YouTube metadata"
            );

            // Description is best-effort — use if available, fallback if not
            let (body_text, description) = match description_result {
                Ok(desc) => {
                    info!(video_id = %video_id, description_length = desc.len(), "Fetched YouTube description");
                    (
                        format!(
                            "{}\n\nBy: {}\n\nDescription:\n{}",
                            metadata.title, metadata.author, desc
                        ),
                        Some(desc),
                    )
                }
                Err(e) => {
                    warn!(video_id = %video_id, error = %e, "Failed to fetch YouTube description, falling back");
                    (
                        format!("{}\n\nBy: {}", metadata.title, metadata.author),
                        Some(format!("YouTube video by {}", metadata.author)),
                    )
                }
            };

            Ok(PageContent {
                title: Some(metadata.title),
                description,
                body_text,
                url: detected_url.url.clone(),
            })
        }
        UrlType::WebPage => {
            let page = fetch_page_content(
                &detected_url.url,
                config.url.fetch_timeout_secs,
                config.url.max_content_bytes,
            )
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            info!(
                url = %detected_url.url,
                title = ?page.title,
                body_length = page.body_text.len(),
                "Fetched page content"
            );

            Ok(page)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_handler_basic_structure() {
        let _fn_ref = handle_url_message;
    }

    #[test]
    fn test_format_multiple_url_confirmations() {
        let results = vec![
            "✅ Example Page".to_string(),
            "✅ Another Page".to_string(),
            "❌ https://broken.example.com (write failed)".to_string(),
        ];

        let confirmation = build_confirmation_message(2, 3, 3, 5, false, &results);

        assert!(confirmation.contains("Saved 2/3 URL(s) as TODO(s)"));
        assert!(confirmation.contains("✅ Example Page"));
        assert!(confirmation.contains("✅ Another Page"));
        assert!(confirmation.contains("❌ https://broken.example.com"));
    }

    #[test]
    fn test_url_limit_enforcement() {
        let urls = (1..=10)
            .map(|i| DetectedUrl {
                url: format!("https://example.com/{}", i),
                url_type: UrlType::WebPage,
                start: 0,
                end: 0,
            })
            .collect::<Vec<_>>();

        let (urls_to_process, truncated) = enforce_url_limit(urls, 5);

        assert_eq!(urls_to_process.len(), 5);
        assert!(truncated);
    }

    #[test]
    fn test_processing_message_format() {
        assert_eq!(build_processing_message(1), "🔗 Processing 1 link(s)...");
        assert_eq!(build_processing_message(3), "🔗 Processing 3 link(s)...");
    }

    #[test]
    fn test_confirmation_includes_truncation_warning() {
        let confirmation = build_confirmation_message(
            5,
            5,
            10,
            5,
            true,
            &["✅ One".to_string(), "✅ Two".to_string()],
        );

        assert!(confirmation.contains("Saved 5 URL(s) as TODO(s)"));
        assert!(confirmation.contains("Message contained 10 URLs (max: 5)"));
        assert!(confirmation.contains("✅ One"));
    }

    #[test]
    fn test_format_result_item_uses_title_or_url() {
        let with_title = format_result_item(Some("Readable title"), "https://example.com", true);
        let with_url = format_result_item(None, "https://example.com/path", false);

        assert_eq!(with_title, "✅ Readable title");
        assert!(with_url.starts_with("❌ https://example.com/path"));
        assert!(with_url.ends_with("(write failed)"));
    }

    #[test]
    fn test_format_webui_url_summary_youtube_with_transcript() {
        let batch = UrlBatchResult {
            total_urls: 1,
            processed_count: 1,
            success_count: 1,
            truncated: false,
            max_urls: 5,
            outcomes: vec![UrlItemOutcome {
                url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
                title: Some("Never Gonna Give You Up".to_string()),
                summary: Some("Classic pop music video.".to_string()),
                tags: vec!["music".to_string(), "youtube".to_string()],
                success: true,
                transcript_wiki_link: Some(
                    "[[transcripts/2026-11-20-never-gonna-give-you-up]]".to_string(),
                ),
                error: None,
            }],
            transcript_actions: vec![],
        };

        let formatted = format_webui_url_summary(&batch);
        assert!(formatted.contains("Saved 1 URL(s) as TODO(s)"));
        assert!(formatted.contains("✅ **Never Gonna Give You Up**"));
        assert!(formatted.contains("[[transcripts/2026-11-20-never-gonna-give-you-up]]"));
        assert!(formatted.contains("Classic pop music video."));
        assert!(formatted.contains("#music #youtube"));
    }

    #[test]
    fn test_format_webui_url_summary_partial_failure_and_truncation() {
        let batch = UrlBatchResult {
            total_urls: 6,
            processed_count: 2,
            success_count: 1,
            truncated: true,
            max_urls: 2,
            outcomes: vec![
                UrlItemOutcome {
                    url: "https://example.com".to_string(),
                    title: Some("Example".to_string()),
                    summary: Some("Example summary".to_string()),
                    tags: vec![],
                    success: true,
                    transcript_wiki_link: None,
                    error: None,
                },
                UrlItemOutcome {
                    url: "https://youtu.be/dQw4w9WgXcQ".to_string(),
                    title: Some("Test Video".to_string()),
                    summary: None,
                    tags: vec![],
                    success: false,
                    transcript_wiki_link: None,
                    error: Some("Failed to fetch transcript: yt-dlp not found".to_string()),
                },
            ],
            transcript_actions: vec![],
        };

        let formatted = format_webui_url_summary(&batch);
        assert!(formatted.contains("Saved 1/2 URL(s) as TODO(s)"));
        assert!(formatted.contains("Message contained 6 URLs (max: 2)"));
        assert!(formatted.contains("✅ **Example**"));
        assert!(
            formatted.contains("❌ **Test Video** — Failed to fetch transcript: yt-dlp not found")
        );
    }
}
