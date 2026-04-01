use std::collections::HashMap;
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatAction, InlineKeyboardButton, InlineKeyboardMarkup};
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::ai::client::OpenRouterClient;
use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::git::debounce::SyncNotifier;
use crate::handlers::HandlerContext;
use crate::url::detect::{DetectedUrl, UrlType};
use crate::url::extract::fetch_page_content;
use crate::url::youtube::{fetch_youtube_metadata, fetch_youtube_description};
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

/// Handle URLs found in Telegram messages
///
/// Pipeline: fetch content -> AI summarize -> format TODO -> write to vault
pub async fn handle_url_message(
    bot: Bot,
    msg: Message,
    ctx: HandlerContext,
    transcript_pending: TranscriptPending,
    urls: Vec<DetectedUrl>,
    surrounding_text: Option<String>,
) -> AppResult<()> {
    if urls.is_empty() {
        return Ok(());
    }

    // 1) auth check
    if let Some(user) = msg.from.as_ref() {
        if !ctx.config.is_user_allowed(user.id.0) {
            info!(user_id = user.id.0, "Unauthorized user, ignoring URL message");
            return Ok(());
        }
    }

    // Track chat_id for conflict notifications (after auth check)
    ctx.chat_tracker.set(msg.chat.id);

    // 2) enforce max URL limit
    let max_urls = ctx.config.url.max_urls_per_message;
    let total_urls = urls.len();
    let (urls_to_process, truncated) = enforce_url_limit(urls, max_urls);

    // 3) immediate processing message before network work
    let status_msg = bot
        .send_message(
            msg.chat.id,
            build_processing_message(urls_to_process.len()),
        )
        .await?;

    // 4) process each URL with graceful degradation
    let mut success_count = 0usize;
    let mut results = Vec::new();
    let mut transcript_buttons: Vec<InlineKeyboardButton> = Vec::new();

    for detected_url in &urls_to_process {
        bot.send_chat_action(msg.chat.id, ChatAction::Typing).await?;

        // Check if this is a YouTube URL with transcript keyword
        let is_transcript_direct_flow = if let UrlType::YouTube { video_id: _ } = &detected_url.url_type {
            let is_transcript_req = crate::url::is_transcript_request(
                surrounding_text.as_deref().unwrap_or(msg.text().unwrap_or("")),
            );
            is_transcript_req
        } else {
            false
        };

        // Direct transcript flow for keyword-triggered requests
        if is_transcript_direct_flow {
            if let UrlType::YouTube { video_id } = &detected_url.url_type {
                bot.send_message(msg.chat.id, "Fetching transcript...")
                    .await?;

                let transcript_text = match crate::url::fetch_transcript(video_id).await {
                    Ok(text) => text,
                    Err(e) => {
                        error!(error = %e, video_id = %video_id, "Failed to fetch transcript");
                        bot.send_message(msg.chat.id, format!("Failed to fetch transcript: {}", e))
                            .await?;
                        continue;
                    }
                };

                // Fetch metadata for title
                let fetched_page = match fetch_for_url_type(detected_url, &ctx.config).await {
                    Ok(page) => page,
                    Err(e) => {
                        error!(error = %e, url = %detected_url.url, "Failed to fetch YouTube metadata");
                        bot.send_message(msg.chat.id, format!("Failed to fetch video metadata: {}", e))
                            .await?;
                        continue;
                    }
                };

                let page_content = crate::url::PageContent {
                    title: fetched_page.title.clone(),
                    description: Some("YouTube video transcript".to_string()),
                    body_text: transcript_text.clone(),
                    url: detected_url.url.clone(),
                };

                let guide = crate::ai::guide::load_guide(&ctx.config.guide_path);
                let summary = match ctx.ai_client
                    .summarize_url(
                        &page_content,
                        None,
                        &ctx.config.openrouter_model_classify,
                        guide.as_deref(),
                    )
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        error!(error = %e, url = %detected_url.url, "Failed to summarize transcript");
                        bot.send_message(msg.chat.id, format!("Failed to summarize transcript: {}", e))
                            .await?;
                        continue;
                    }
                };

                let date = chrono::Local::now().format("%Y-%m-%d").to_string();
                let video_title = fetched_page.title.clone()
                    .unwrap_or_else(|| detected_url.url.clone());

                let transcript_file = match crate::vault::save_transcript(
                    std::path::Path::new(&ctx.config.vault_path),
                    &ctx.config.url.transcript_folder,
                    video_id,
                    &video_title,
                    &summary.summary,
                    &transcript_text,
                    &date,
                )
                .await
                {
                    Ok(file) => file,
                    Err(e) => {
                        error!(error = %e, video_id = %video_id, "Failed to save transcript");
                        bot.send_message(msg.chat.id, format!("Failed to save transcript: {}", e))
                            .await?;
                        continue;
                    }
                };

                let wiki_link_entry = format!("  - Transcript: {}", transcript_file.wiki_link);
                if let Err(e) = ctx.vault.append_to_section("## Todos", &wiki_link_entry).await {
                    warn!(error = %e, "Failed to add transcript wiki-link to daily note");
                }

                ctx.notify_sync();

                bot.send_message(
                    msg.chat.id,
                    format!("Transcript saved: {}", transcript_file.wiki_link),
                )
                .await?;

                success_count += 1;
                results.push(format_result_item(
                    Some(&video_title),
                    &detected_url.url,
                    true,
                ));
            }
            continue;
        }

        // Normal fast-mode flow for non-transcript URLs
        let fetched_page = fetch_for_url_type(detected_url, &ctx.config).await;

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
                    let short_id = Uuid::new_v4().to_string()[..8].to_string();
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
                        "Full Transcript",
                        format!("yt_transcript:{}", short_id),
                    ));
                }

                let guide = crate::ai::guide::load_guide(&ctx.config.guide_path);

                match ctx.ai_client
                    .summarize_url(
                        &page_content,
                        surrounding_text.as_deref(),
                        &ctx.config.openrouter_model_classify,
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
                        (
                            Some(summary.title),
                            Some(summary.summary),
                            summary.tags,
                        )
                    }
                    Err(e) => {
                        // Graceful degradation: AI failed -> title-only TODO
                        error!(error = %e, url = %detected_url.url, "AI summarization failed");
                        (fetched_title, None, Vec::new())
                    }
                }
            }
            Err(e) => {
                // Graceful degradation: fetch failed -> plain URL TODO
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

        match ctx.vault.append_to_section(section, &content).await {
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
        ctx.notify_sync();
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

pub async fn handle_transcript_callback(
    bot: Bot,
    q: CallbackQuery,
    transcript_pending: TranscriptPending,
    ai_client: Arc<OpenRouterClient>,
    vault: Arc<DailyNoteManager>,
    config: Arc<Config>,
    sync_notifier: Option<SyncNotifier>,
) -> AppResult<()> {
    let data = q.data.as_ref().ok_or_else(|| AppError::Handler("No callback data".to_string()))?;
    let short_id = data
        .strip_prefix("yt_transcript:")
        .ok_or_else(|| AppError::Handler("Invalid callback format".to_string()))?;

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
        bot.send_message(chat_id, "⏳ Fetching transcript...").await?;

        let transcript_text = match crate::url::fetch_transcript(&request.video_id).await {
            Ok(text) => text,
            Err(e) => {
                bot.send_message(chat_id, format!("❌ Failed to fetch transcript: {}", e))
                    .await?;
                return Ok(());
            }
        };

        let page_content = crate::url::PageContent {
            title: Some(request.title.clone()),
            description: Some("YouTube video transcript".to_string()),
            body_text: transcript_text.clone(),
            url: request.url.clone(),
        };

        let guide = crate::ai::guide::load_guide(&config.guide_path);
        let summary = match ai_client
            .summarize_url(
                &page_content,
                None,
                &config.openrouter_model_classify,
                guide.as_deref(),
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                bot.send_message(chat_id, format!("❌ Failed to summarize transcript: {}", e))
                    .await?;
                return Ok(());
            }
        };

        // Format transcript with AI before saving
        let formatted_transcript = match ai_client
            .format_transcript(&transcript_text, &request.title, &config.openrouter_model_classify)
            .await
        {
            Ok(formatted) => formatted,
            Err(e) => {
                warn!(error = %e, "Failed to format transcript, using raw text");
                transcript_text.clone()
            }
        };

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let transcript_file = match crate::vault::save_transcript(
            std::path::Path::new(&config.vault_path),
            &config.url.transcript_folder,
            &request.video_id,
            &request.title,
            &summary.summary,
            &formatted_transcript,
            &date,
        )
        .await
        {
            Ok(file) => file,
            Err(e) => {
                bot.send_message(chat_id, format!("❌ Failed to save transcript: {}", e))
                    .await?;
                return Ok(());
            }
        };

        let (section, content) = crate::vault::writer::format_url_todo(
            &request.url,
            Some(&summary.title),
            Some(&summary.summary),
            &summary.tags,
            Some(&transcript_file.wiki_link),
            Some(&request.title),
        );
        if let Err(e) = vault.replace_entry_by_url(section, &request.url, &content).await {
            warn!(error = %e, section, url = %request.url, "Failed to replace entry in daily note");
        }

        if let Some(ref notifier) = sync_notifier {
            notifier.notify();
        }

        bot.send_message(
            chat_id,
            format!("✅ Transcript saved: {}", transcript_file.wiki_link),
        )
        .await?;
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
        let mut result = confirmation;
        if result.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
            truncate_to_char_boundary(&mut result, TELEGRAM_MAX_MESSAGE_LENGTH - 3);
            result.push_str("...");
        }
        return result;
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

    // Final safety: hard truncate if still too long (char-boundary safe)
    if message.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
        truncate_to_char_boundary(&mut message, TELEGRAM_MAX_MESSAGE_LENGTH - 3);
        message.push_str("...");
    }

    message
}

/// Truncate a string to at most `max_bytes` bytes, ensuring we don't split a UTF-8 char.
/// Finds the largest valid char boundary at or before `max_bytes`.
fn truncate_to_char_boundary(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    // Find the largest char boundary <= max_bytes
    let mut end = max_bytes;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    s.truncate(end);
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
) -> AppResult<PageContent> {
    match &detected_url.url_type {
        UrlType::YouTube { video_id } => {
            // Parallel fetch: metadata + description
            let (metadata_result, description_result) = tokio::join!(
                fetch_youtube_metadata(&detected_url.url, config.url.fetch_timeout_secs),
                fetch_youtube_description(video_id, config.url.fetch_timeout_secs)
            );

            // Metadata must succeed (same error handling as before)
            let metadata = metadata_result?;

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
                        format!("{}\n\nBy: {}\n\nDescription:\n{}", metadata.title, metadata.author, desc),
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
            .await?;

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
    fn test_truncate_to_char_boundary_ascii() {
        let mut s = "hello world".to_string();
        truncate_to_char_boundary(&mut s, 5);
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_truncate_to_char_boundary_unicode() {
        // "日本語" = 9 bytes (3 chars × 3 bytes each)
        let mut s = "日本語".to_string();
        assert_eq!(s.len(), 9);
        
        // Truncate to 5 bytes — can't fit second char, so stop at 3
        truncate_to_char_boundary(&mut s, 5);
        assert_eq!(s, "日"); // Only first char (3 bytes)
        
        // Truncate at exact boundary
        let mut s2 = "日本語".to_string();
        truncate_to_char_boundary(&mut s2, 6);
        assert_eq!(s2, "日本"); // Two chars (6 bytes)
    }

    #[test]
    fn test_truncate_to_char_boundary_emoji() {
        // "👋🌍" = 8 bytes (2 emoji × 4 bytes each)
        let mut s = "👋🌍".to_string();
        assert_eq!(s.len(), 8);
        
        // Truncate to 5 bytes — can't fit any of second emoji
        truncate_to_char_boundary(&mut s, 5);
        assert_eq!(s, "👋"); // Only first emoji (4 bytes)
    }

    #[test]
    fn test_truncate_to_char_boundary_no_truncation_needed() {
        let mut s = "short".to_string();
        truncate_to_char_boundary(&mut s, 100);
        assert_eq!(s, "short");
    }
}
