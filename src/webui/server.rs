use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket},
        Multipart, Query, State, WebSocketUpgrade,
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
        .route("/api/photo", post(post_photo_message))
        .route("/api/voice", post(post_voice_message))
        // Real-time updates WebSocket
        .route("/ws", get(ws_handler))
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
            let mime = if filename.ends_with(".png") {
                "image/png"
            } else if filename.ends_with(".gif") {
                "image/gif"
            } else if filename.ends_with(".webp") {
                "image/webp"
            } else {
                "image/jpeg"
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
}

async fn post_text_message(
    headers: HeaderMap,
    State(state): State<WebuiState>,
    Json(payload): Json<TextMessageRequest>,
) -> impl IntoResponse {
    if let Err(status) = check_auth(&headers, &state.config) {
        return (status, "Unauthorized").into_response();
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
        Ok((filename, summary)) => {
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
