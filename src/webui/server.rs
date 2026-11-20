use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket},
        DefaultBodyLimit, Multipart, Query, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::ai::AiService;
use crate::config::Config;
use crate::git::debounce;
use crate::vault::daily_note::DailyNoteManager;

// Embed static files directly in the binary
const INDEX_HTML: &str = include_str!("index.html");
const APP_CSS: &str = include_str!("app.css");
const APP_JS: &str = include_str!("app.js");

/// Real-time event broadcasted to WebSocket clients
#[derive(Clone, Serialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebuiEvent {
    NoteUpdate { date: String, content: String },
}

/// Shared application state for WebUI server
#[derive(Clone)]
pub struct WebuiState {
    pub config: Arc<Config>,
    pub ai_service: Arc<AiService>,
    pub vault: Arc<DailyNoteManager>,
    pub sync_notifier: Option<debounce::SyncNotifier>,
    pub ws_broadcast: broadcast::Sender<WebuiEvent>,
    pub transcript_pending: crate::handlers::url::TranscriptPending,
}

/// Start the concurrent Axum WebUI/API server
pub async fn start_server(state: WebuiState, port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!(
        port = port,
        "Starting concurrent WebUI server on http://0.0.0.0:{}...", port
    );

    // Spawn background task to listen for Vault writes and trigger WebuiEvent::NoteUpdate broadcasts
    if let Some(mut rx) = state.vault.subscribe_updates() {
        let state_clone = state.clone();
        tokio::spawn(async move {
            while rx.recv().await.is_ok() {
                info!("Daily note write detected, broadcasting to WebSockets...");
                broadcast_note_update(&state_clone).await;
            }
        });
    }

    // Build routes
    let app = Router::new()
        // Static assets
        .route("/", get(serve_index))
        .route("/app.css", get(serve_css))
        .route("/app.js", get(serve_js))
        .route("/assets/:filename", get(serve_asset))
        // Authenticated endpoints
        .route("/api/note", get(get_daily_note))
        .route("/api/message", post(post_text_message))
        .route("/api/transcript", post(post_transcript_request))
        .route("/api/photo", post(post_photo_message))
        .route("/api/voice", post(post_voice_message))
        .route("/api/pdf", post(post_pdf_message))
        .route("/api/file", post(post_generic_file_message))
        // Real-time updates WebSocket
        .route("/ws", get(ws_handler))
        .layer(DefaultBodyLimit::max(250 * 1024 * 1024))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(error = %e, "WebUI server failed to bind to address {}", addr);
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        error!(error = %e, "WebUI server error during execution");
    }
}

// Static File Handlers
async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn serve_css() -> impl IntoResponse {
    Response::builder()
        .header("content-type", "text/css")
        .body(APP_CSS.to_string())
        .unwrap()
}

async fn serve_js() -> impl IntoResponse {
    Response::builder()
        .header("content-type", "application/javascript")
        .body(APP_JS.to_string())
        .unwrap()
}

// Serve actual saved Obsidian assets from vault to WebUI
async fn serve_asset(
    State(state): State<WebuiState>,
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> impl IntoResponse {
    let note_path = match state.vault.ensure_today().await {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Vault error: {}", e),
            )
                .into_response()
        }
    };

    let note_dir = match note_path.parent() {
        Some(p) => p,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "Invalid note path").into_response(),
    };

    let asset_path = note_dir
        .join(&state.config.image.assets_folder)
        .join(&filename);

    if !asset_path.exists() {
        return (StatusCode::NOT_FOUND, "Asset not found").into_response();
    }

    match tokio::fs::read(&asset_path).await {
        Ok(bytes) => {
            let lower = filename.to_lowercase();
            let mime = if lower.ends_with(".png") {
                "image/png"
            } else if lower.ends_with(".gif") {
                "image/gif"
            } else if lower.ends_with(".webp") {
                "image/webp"
            } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
                "image/jpeg"
            } else if lower.ends_with(".svg") {
                "image/svg+xml"
            } else if lower.ends_with(".pdf") {
                "application/pdf"
            } else if lower.ends_with(".md") {
                "text/markdown"
            } else if lower.ends_with(".txt") {
                "text/plain"
            } else if lower.ends_with(".json") {
                "application/json"
            } else if lower.ends_with(".mp3") {
                "audio/mpeg"
            } else if lower.ends_with(".mp4") {
                "video/mp4"
            } else {
                "application/octet-stream"
            };

            Response::builder()
                .header("content-type", mime)
                .body(axum::body::Body::from(bytes))
                .unwrap()
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read asset: {}", e),
        )
            .into_response(),
    }
}

// Token Verification Middleware helper
fn check_auth(headers: &HeaderMap, config: &Config) -> Result<(), StatusCode> {
    let expected_token = match &config.webui_auth_token {
        Some(t) => t,
        None => return Ok(()), // If no token is configured, allow all requests
    };

    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = auth_header.trim_start_matches("Bearer ");
    if token == expected_token {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

// 1. GET /api/note - Read active daily note content
#[derive(Serialize)]
struct NoteResponse {
    date: String,
    content: String,
}

async fn get_daily_note(headers: HeaderMap, State(state): State<WebuiState>) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
    }

    let note_path = match state.vault.ensure_today().await {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Vault error: {}", e),
            )
                .into_response()
        }
    };

    let date_str = note_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Today".to_string());

    let content = match tokio::fs::read_to_string(&note_path).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("IO error: {}", e),
            )
                .into_response()
        }
    };

    (
        StatusCode::OK,
        Json(NoteResponse {
            date: date_str,
            content,
        }),
    )
        .into_response()
}

// Helper: Broadcast updated daily note content via WebSockets
async fn broadcast_note_update(state: &WebuiState) {
    let note_path = match state.vault.ensure_today().await {
        Ok(p) => p,
        Err(_) => return,
    };

    let date_str = note_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Today".to_string());

    if let Ok(content) = tokio::fs::read_to_string(&note_path).await {
        let event = WebuiEvent::NoteUpdate {
            date: date_str,
            content,
        };
        let _ = state.ws_broadcast.send(event);
    }
}

// 2. POST /api/message - Submit a text entry
#[derive(Deserialize)]
struct TextMessageRequest {
    text: String,
}

#[derive(Serialize)]
struct TextMessageResponse {
    category: String,
    summary: String,
    tags: Vec<String>,
    ai_success: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    transcript_actions: Vec<crate::handlers::url::PendingTranscriptAction>,
}

impl TextMessageResponse {
    fn command(summary: impl Into<String>) -> Self {
        Self {
            category: "command".to_string(),
            summary: summary.into(),
            tags: Vec::new(),
            ai_success: true,
            transcript_actions: Vec::new(),
        }
    }
}

async fn post_text_message(
    headers: HeaderMap,
    State(state): State<WebuiState>,
    Json(payload): Json<TextMessageRequest>,
) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
    }

    let text = payload.text.trim();
    if text == "/git_refresh" {
        if let Some(ref notifier) = state.sync_notifier {
            if notifier.is_busy() {
                let response = TextMessageResponse::command(
                    "⏳ Git operation is already in progress. Please try again in a moment.",
                );
                return (StatusCode::OK, Json(response)).into_response();
            }

            let Ok(_git_guard) = notifier.git_lock.try_lock() else {
                let response = TextMessageResponse::command(
                    "⏳ Git lock is currently held by another operation. Please try again.",
                );
                return (StatusCode::OK, Json(response)).into_response();
            };

            let sync_running = notifier.sync_running.clone();
            sync_running.store(true, std::sync::atomic::Ordering::SeqCst);
            let result = notifier.git_sync.full_sync().await;
            sync_running.store(false, std::sync::atomic::Ordering::SeqCst);

            match result {
                Ok(crate::git::sync::SyncResult::NothingToSync) => {
                    let response =
                        TextMessageResponse::command("✅ Git sync completed: Nothing to sync.");
                    return (StatusCode::OK, Json(response)).into_response();
                }
                Ok(crate::git::sync::SyncResult::Pushed) => {
                    let response = TextMessageResponse::command(
                        "✅ Git sync completed: Local changes pushed to remote.",
                    );
                    return (StatusCode::OK, Json(response)).into_response();
                }
                Ok(crate::git::sync::SyncResult::PushedWithoutFetch) => {
                    let response = TextMessageResponse::command(
                        "✅ Git sync completed: Pushed without fetch (remote offline).",
                    );
                    return (StatusCode::OK, Json(response)).into_response();
                }
                Ok(crate::git::sync::SyncResult::RebasedAndPushed) => {
                    let response = TextMessageResponse::command(
                        "✅ Git sync completed: Rebased local changes on top of remote and pushed successfully.",
                    );
                    return (StatusCode::OK, Json(response)).into_response();
                }
                Ok(crate::git::sync::SyncResult::ConflictDetected(info)) => {
                    let files_list = info.files.join(", ");
                    let response = TextMessageResponse::command(format!(
                        "⚠️ Git sync failed: Conflict detected in file(s): **{}**. Please resolve using Telegram or force refresh using `/git_force_refresh`.",
                        files_list
                    ));
                    return (StatusCode::OK, Json(response)).into_response();
                }
                Err(e) => {
                    let response =
                        TextMessageResponse::command(format!("❌ Git sync failed: {}", e));
                    return (StatusCode::OK, Json(response)).into_response();
                }
            }
        } else {
            let response = TextMessageResponse::command(
                "⚠️ Git synchronization is disabled in configuration.",
            );
            return (StatusCode::OK, Json(response)).into_response();
        }
    } else if text == "/git_force_refresh" {
        if let Some(ref notifier) = state.sync_notifier {
            if notifier.is_busy() {
                let response = TextMessageResponse::command(
                    "⏳ Git operation is already in progress. Please try again in a moment.",
                );
                return (StatusCode::OK, Json(response)).into_response();
            }

            match notifier.force_refresh().await {
                Ok(()) => {
                    // Push updated daily note to all WebSockets because force refresh might have overwritten today's note!
                    broadcast_note_update(&state).await;

                    let response = TextMessageResponse::command(
                        "✅ Force refresh completed successfully. The agent vault is now aligned with the latest remote version!",
                    );
                    return (StatusCode::OK, Json(response)).into_response();
                }
                Err(e) => {
                    let response =
                        TextMessageResponse::command(format!("❌ Force refresh failed: {}", e));
                    return (StatusCode::OK, Json(response)).into_response();
                }
            }
        } else {
            let response = TextMessageResponse::command(
                "⚠️ Git synchronization is disabled in configuration.",
            );
            return (StatusCode::OK, Json(response)).into_response();
        }
    }

    // Check for URLs (including YouTube links and keyword-triggered YouTube transcripts)
    let detected_urls = crate::url::detect::detect_urls(text);
    if !detected_urls.is_empty() {
        let surrounding_text = if detected_urls.iter().all(|u| text == u.url) {
            None
        } else {
            Some(text)
        };

        let batch = crate::handlers::url::process_url_entries(
            detected_urls,
            surrounding_text,
            text,
            &state.config,
            &state.ai_service,
            &state.vault,
            state.sync_notifier.as_ref(),
            &state.transcript_pending,
        )
        .await;

        if batch.success_count > 0 {
            broadcast_note_update(&state).await;
        }

        let summary = crate::handlers::url::format_webui_url_summary(&batch);
        let mut tags = Vec::new();
        for outcome in &batch.outcomes {
            for tag in &outcome.tags {
                if !tags.contains(tag) {
                    tags.push(tag.clone());
                }
            }
        }

        let response = TextMessageResponse {
            category: "url".to_string(),
            summary,
            tags,
            ai_success: batch.success_count > 0,
            transcript_actions: batch.transcript_actions,
        };
        return (StatusCode::OK, Json(response)).into_response();
    }

    // Process text message through extracted text handler
    let result = crate::handlers::text::process_text_entry(
        &payload.text,
        &state.config,
        &state.ai_service,
        &state.vault,
        state.sync_notifier.as_ref(),
    )
    .await;

    match result {
        Ok((classified, ai_success)) => {
            // Push update to all WebSockets
            broadcast_note_update(&state).await;

            let response = TextMessageResponse {
                category: format!("{:?}", classified.category).to_lowercase(),
                summary: classified.summary,
                tags: classified.tags,
                ai_success,
                transcript_actions: Vec::new(),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to process note: {}", e),
        )
            .into_response(),
    }
}

// 2b. POST /api/transcript - Fetch & save full YouTube transcript for a pending action
#[derive(Deserialize)]
struct TranscriptActionRequest {
    id: String,
}

#[derive(Serialize)]
struct TranscriptActionResponse {
    title: String,
    summary: String,
    tags: Vec<String>,
    wiki_link: String,
}

async fn post_transcript_request(
    headers: HeaderMap,
    State(state): State<WebuiState>,
    Json(payload): Json<TranscriptActionRequest>,
) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
    }

    let request = {
        let mut pending = state.transcript_pending.lock().await;
        pending.remove(&payload.id)
    };

    let request = match request {
        Some(req) => req,
        None => {
            return (
                StatusCode::NOT_FOUND,
                "Transcript request expired or not found. Please resend the YouTube link.",
            )
                .into_response();
        }
    };

    match crate::handlers::url::process_transcript_request(
        &request,
        &state.config,
        &state.ai_service,
        &state.vault,
        state.sync_notifier.as_ref(),
        true,
    )
    .await
    {
        Ok(result) => {
            broadcast_note_update(&state).await;
            (
                StatusCode::OK,
                Json(TranscriptActionResponse {
                    title: result.title,
                    summary: result.summary,
                    tags: result.tags,
                    wiki_link: result.wiki_link,
                }),
            )
                .into_response()
        }
        Err(e) => {
            // Re-insert request so the user can retry on transient errors
            state
                .transcript_pending
                .lock()
                .await
                .insert(payload.id, request);
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}

// 3. POST /api/photo - Submit a photo multipart file
#[derive(Serialize)]
struct PhotoMessageResponse {
    filename: String,
    summary: String,
}

async fn post_photo_message(
    headers: HeaderMap,
    State(state): State<WebuiState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
    }

    let mut photo_bytes = Vec::new();
    let mut caption = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            if let Ok(bytes) = field.bytes().await {
                photo_bytes = bytes.to_vec();
            }
        } else if name == "caption" {
            if let Ok(text) = field.text().await {
                if !text.trim().is_empty() {
                    caption = Some(text);
                }
            }
        }
    }

    if photo_bytes.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing photo file payload").into_response();
    }

    let result = crate::handlers::photo::process_photo_entry(
        &photo_bytes,
        caption.as_deref(),
        &state.config,
        &state.ai_service,
        &state.vault,
        state.sync_notifier.as_ref(),
    )
    .await;

    match result {
        Ok((filename, summary, _exif_missing)) => {
            broadcast_note_update(&state).await;
            (
                StatusCode::OK,
                Json(PhotoMessageResponse { filename, summary }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to process photo note: {}", e),
        )
            .into_response(),
    }
}

// 4. POST /api/voice - Submit a voice multipart file
#[derive(Serialize)]
struct VoiceMessageResponse {
    transcript: String,
    category: String,
    summary: String,
}

async fn post_voice_message(
    headers: HeaderMap,
    State(state): State<WebuiState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
    }

    let mut voice_bytes = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            if let Ok(bytes) = field.bytes().await {
                voice_bytes = bytes.to_vec();
            }
        }
    }

    if voice_bytes.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing audio file payload").into_response();
    }

    let result = crate::handlers::voice::process_voice_entry(
        &voice_bytes,
        &state.config,
        &state.ai_service,
        &state.vault,
        state.sync_notifier.as_ref(),
    )
    .await;

    match result {
        Ok((transcript, classified)) => {
            broadcast_note_update(&state).await;
            (
                StatusCode::OK,
                Json(VoiceMessageResponse {
                    transcript,
                    category: format!("{:?}", classified.category).to_lowercase(),
                    summary: classified.summary,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to process voice note: {}", e),
        )
            .into_response(),
    }
}

// POST /api/pdf - Submit a PDF multipart file
#[derive(Serialize)]
struct PdfMessageResponse {
    pdf_filename: String,
    transcript_filename: String,
    title: String,
    gemini_success: bool,
}

async fn post_pdf_message(
    headers: HeaderMap,
    State(state): State<WebuiState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
    }

    let mut pdf_bytes = Vec::new();
    let mut caption = None;
    let mut original_filename = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            original_filename = field.file_name().map(|f| f.to_string());
            if let Ok(bytes) = field.bytes().await {
                pdf_bytes = bytes.to_vec();
            }
        } else if name == "caption" {
            if let Ok(text) = field.text().await {
                if !text.trim().is_empty() {
                    caption = Some(text);
                }
            }
        }
    }

    if pdf_bytes.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing PDF file payload").into_response();
    }

    let result = crate::handlers::pdf::process_pdf_entry(
        &pdf_bytes,
        original_filename.as_deref(),
        caption.as_deref(),
        &state.config,
        &state.ai_service,
        &state.vault,
        state.sync_notifier.as_ref(),
    )
    .await;

    match result {
        Ok((pdf_filename, transcript_filename, title, _summary, gemini_success)) => {
            broadcast_note_update(&state).await;
            let response = PdfMessageResponse {
                pdf_filename,
                transcript_filename,
                title,
                gemini_success,
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to process PDF note: {}", e),
        )
            .into_response(),
    }
}

// POST /api/file - Submit an unrecognized or generic multipart file
#[derive(Serialize)]
struct FileMessageResponse {
    filename: String,
    original_name: String,
    warning: String,
}

async fn post_generic_file_message(
    headers: HeaderMap,
    State(state): State<WebuiState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
    }

    let mut file_bytes = Vec::new();
    let mut caption = None;
    let mut original_filename = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            original_filename = field.file_name().map(|f| f.to_string());
            if let Ok(bytes) = field.bytes().await {
                file_bytes = bytes.to_vec();
            }
        } else if name == "caption" {
            if let Ok(text) = field.text().await {
                if !text.trim().is_empty() {
                    caption = Some(text);
                }
            }
        }
    }

    if file_bytes.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing file payload").into_response();
    }

    let result = crate::handlers::file::process_generic_file_entry(
        &file_bytes,
        original_filename.as_deref(),
        caption.as_deref(),
        &state.config,
        &state.vault,
        state.sync_notifier.as_ref(),
    )
    .await;

    match result {
        Ok((filename, original_name)) => {
            broadcast_note_update(&state).await;
            let response = FileMessageResponse {
                filename,
                original_name,
                warning: "Unrecognized file stored as-is in assets folder without AI processing."
                    .to_string(),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to process file: {}", e),
        )
            .into_response(),
    }
}

// 5. GET /ws - Upgrade to WebSockets
#[derive(Deserialize)]
struct WsQuery {
    token: String,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<WebuiState>,
) -> impl IntoResponse {
    let expected_token = state.config.webui_auth_token.clone();

    // Check WebSocket Auth Token
    if let Some(token) = expected_token {
        if query.token != token {
            warn!(token = %query.token, "Unauthorized WebSocket connection rejected");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    ws.on_upgrade(move |socket| handle_ws_session(socket, state))
}

async fn handle_ws_session(socket: WebSocket, state: WebuiState) {
    let (mut sender, mut receiver) = socket.split();

    // Immediately send the active daily note contents to populate the frontend
    if let Ok(note_path) = state.vault.ensure_today().await {
        let date_str = note_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Today".to_string());

        if let Ok(content) = tokio::fs::read_to_string(&note_path).await {
            let init_event = WebuiEvent::NoteUpdate {
                date: date_str,
                content,
            };
            if let Ok(json) = serde_json::to_string(&init_event) {
                let _ = sender.send(WsMessage::Text(json.into())).await;
            }
        }
    }

    let mut ws_rx = state.ws_broadcast.subscribe();

    // Keep checking for updates and broadcast events
    tokio::select! {
        // Broadcast listener
        _ = async {
            while let Ok(event) = ws_rx.recv().await {
                if let Ok(json) = serde_json::to_string(&event) {
                    if sender.send(WsMessage::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
        } => {}

        // Connection listener (disconnect checking)
        _ = async {
            while let Some(msg) = receiver.next().await {
                if let Ok(WsMessage::Close(_)) = msg {
                    break;
                }
            }
        } => {}
    }

    info!("WebSocket session closed cleanly");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    async fn build_test_state(auth_token: Option<&str>) -> WebuiState {
        let config = Config {
            vault_path: std::env::temp_dir()
                .join(format!("obsidian_webui_test_{}", uuid::Uuid::new_v4())),
            webui_auth_token: auth_token.map(|t| t.to_string()),
            ..Default::default()
        };

        let ai_service = Arc::new(AiService::new(HashMap::new(), &config));
        let vault = Arc::new(
            DailyNoteManager::new(
                config.vault_path.clone(),
                config.date_display_format.clone(),
                None,
                None,
            )
            .await,
        );
        let (ws_broadcast, _) = broadcast::channel(16);
        let transcript_pending = Arc::new(Mutex::new(HashMap::new()));

        WebuiState {
            config: Arc::new(config),
            ai_service,
            vault,
            sync_notifier: None,
            ws_broadcast,
            transcript_pending,
        }
    }

    #[test]
    fn test_text_message_response_omits_empty_transcript_actions() {
        let resp = TextMessageResponse::command("ok");
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("transcript_actions").is_none());
    }

    #[test]
    fn test_text_message_response_serializes_transcript_actions() {
        let resp = TextMessageResponse {
            category: "url".to_string(),
            summary: "Saved 1 URL(s)".to_string(),
            tags: vec!["youtube".to_string()],
            ai_success: true,
            transcript_actions: vec![crate::handlers::url::PendingTranscriptAction {
                id: "abcd1234".to_string(),
                video_id: "dQw4w9WgXcQ".to_string(),
                url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
                title: "Never Gonna Give You Up".to_string(),
            }],
        };

        let json = serde_json::to_value(&resp).unwrap();
        let actions = json["transcript_actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["id"], "abcd1234");
        assert_eq!(actions[0]["video_id"], "dQw4w9WgXcQ");
    }

    #[tokio::test]
    async fn test_post_transcript_request_requires_auth() {
        let state = build_test_state(Some("secret-token")).await;
        let headers = HeaderMap::new();

        let resp = post_transcript_request(
            headers,
            State(state),
            Json(TranscriptActionRequest {
                id: "missing".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_post_transcript_request_not_found_for_expired_id() {
        let state = build_test_state(Some("secret-token")).await;
        let vault_path = state.config.vault_path.clone();
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            axum::http::HeaderValue::from_static("Bearer secret-token"),
        );

        let resp = post_transcript_request(
            headers,
            State(state),
            Json(TranscriptActionRequest {
                id: "nonexistent".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let _ = tokio::fs::remove_dir_all(&vault_path).await;
    }

    struct MockWebuiAiProvider;

    #[async_trait::async_trait]
    impl crate::ai::AiProvider for MockWebuiAiProvider {
        async fn classify_text(
            &self,
            _text: &str,
            _model: &str,
            _guide: Option<&str>,
        ) -> Result<crate::ai::classify::ClassifiedNote, crate::error::AiError> {
            Ok(crate::ai::classify::ClassifiedNote {
                category: crate::ai::classify::NoteCategory::Log,
                markdown: "- mock".to_string(),
                tags: vec![],
                summary: "mock".to_string(),
                frontmatter: None,
                date: None,
            })
        }

        async fn classify_image(
            &self,
            _image_base64: &str,
            _caption: Option<&str>,
            _exif_context: &str,
            _model: &str,
            _guide: Option<&str>,
        ) -> Result<crate::ai::classify::ClassifiedNote, crate::error::AiError> {
            unreachable!()
        }

        async fn summarize_url(
            &self,
            page_content: &crate::url::PageContent,
            _user_text: Option<&str>,
            _model: &str,
            _guide: Option<&str>,
        ) -> Result<crate::ai::summarize::UrlSummary, crate::error::AiError> {
            Ok(crate::ai::summarize::UrlSummary {
                title: page_content
                    .title
                    .clone()
                    .unwrap_or_else(|| "Mock Title".to_string()),
                summary: "Concise AI summary of page.".to_string(),
                tags: vec!["reading".to_string(), "webui".to_string()],
            })
        }

        async fn transcribe(&self, _audio_bytes: &[u8]) -> Result<String, crate::error::AiError> {
            unreachable!()
        }

        async fn format_transcript(
            &self,
            raw_transcript: &str,
            _video_title: &str,
            _model: &str,
        ) -> Result<String, crate::error::AiError> {
            Ok(format!("### Formatted\n\n{}", raw_transcript))
        }

        async fn chat_completion(
            &self,
            _model: &str,
            _messages: Vec<crate::ai::ChatMessage>,
            _max_tokens: Option<u32>,
        ) -> Result<String, crate::error::AiError> {
            unreachable!()
        }

        async fn transcribe_pdf(
            &self,
            _pdf_bytes: &[u8],
            _user_prompt: Option<&str>,
            _model: &str,
        ) -> Result<crate::ai::classify::ClassifiedNote, crate::error::AiError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn test_post_text_message_url_pipeline_writes_todo_to_daily_note() {
        // Spin up a tiny local HTTP server serving a test HTML page
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_handle = tokio::spawn(async move {
            let app = Router::new().route(
                "/article",
                get(|| async {
                    Html("<html><head><title>Rust WebUI Guide</title></head><body><p>Wiring URL processing into WebUI.</p></body></html>")
                }),
            );
            let _ = axum::serve(listener, app).await;
        });

        let vault_path =
            std::env::temp_dir().join(format!("obsidian_webui_url_test_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&vault_path).await.unwrap();

        let config = Config {
            vault_path: vault_path.clone(),
            ai_provider: "mock".to_string(),
            webui_auth_token: Some("test-secret".to_string()),
            ..Default::default()
        };

        let mut providers: HashMap<String, Arc<dyn crate::ai::AiProvider>> = HashMap::new();
        providers.insert("mock".to_string(), Arc::new(MockWebuiAiProvider));
        let ai_service = Arc::new(AiService::new(providers, &config));
        let vault = Arc::new(
            DailyNoteManager::new(
                config.vault_path.clone(),
                config.date_display_format.clone(),
                None,
                None,
            )
            .await,
        );
        let (ws_broadcast, mut ws_rx) = broadcast::channel(16);
        let state = WebuiState {
            config: Arc::new(config),
            ai_service,
            vault: vault.clone(),
            sync_notifier: None,
            ws_broadcast,
            transcript_pending: Arc::new(Mutex::new(HashMap::new())),
        };

        let url = format!("http://{}/article", addr);
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            axum::http::HeaderValue::from_static("Bearer test-secret"),
        );

        let resp = post_text_message(
            headers,
            State(state),
            Json(TextMessageRequest {
                text: format!("Read this later {}", url),
            }),
        )
        .await
        .into_response();

        assert_eq!(resp.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["category"], "url");
        assert_eq!(json["ai_success"], true);
        assert!(json["summary"]
            .as_str()
            .unwrap()
            .contains("Rust WebUI Guide"));

        // Verify Daily Note was written and WebSocket update was broadcast
        let note_path = vault.ensure_today().await.unwrap();
        let note_content = tokio::fs::read_to_string(&note_path).await.unwrap();
        assert!(note_content.contains("## ✅ Todos"));
        assert!(note_content.contains(&format!("[Rust WebUI Guide]({})", url)));
        assert!(note_content.contains("Concise AI summary of page."));
        assert!(note_content.contains("#reading #webui"));

        let ws_event = ws_rx
            .try_recv()
            .expect("Expected WebSocket NoteUpdate broadcast");
        match ws_event {
            WebuiEvent::NoteUpdate { content, .. } => {
                assert!(content.contains(&url));
            }
        }

        server_handle.abort();
        let _ = tokio::fs::remove_dir_all(&vault_path).await;
    }

    #[tokio::test]
    async fn test_post_transcript_request_reinserts_pending_entry_on_failure() {
        let state = build_test_state(Some("secret-token")).await;
        let vault_path = state.config.vault_path.clone();

        // Insert a pending transcript request with an invalid video ID so fetch_transcript fails fast
        state.transcript_pending.lock().await.insert(
            "retry123".to_string(),
            crate::handlers::url::TranscriptRequest {
                video_id: "invalid".to_string(),
                url: "https://youtube.com/watch?v=invalid".to_string(),
                title: "Invalid Test Video".to_string(),
            },
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            axum::http::HeaderValue::from_static("Bearer secret-token"),
        );

        let resp = post_transcript_request(
            headers,
            State(state.clone()),
            Json(TranscriptActionRequest {
                id: "retry123".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        // Pending request should be re-inserted so user can retry
        assert!(state
            .transcript_pending
            .lock()
            .await
            .contains_key("retry123"));

        let _ = tokio::fs::remove_dir_all(&vault_path).await;
    }
}
