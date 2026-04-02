mod ai;
mod audio;
mod config;
mod error;
mod git;
mod handlers;
mod image;
mod url;
mod vault;

use std::collections::HashMap;
use std::sync::Arc;

use teloxide::dispatching::UpdateHandler;
use teloxide::prelude::*;
use teloxide::types::ChatId;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use ai::client::OpenRouterClient;
use ai::transcribe::WhisperClient;
use config::Config;
use error::AppError;
use git::chat_tracker;
use git::conflict;
use git::debounce;
use git::sync::GitSync;
use handlers::url::TranscriptPending;
use handlers::HandlerContext;
use vault::daily_note::DailyNoteManager;

/// Build version embedded at compile time.
/// CI builds pass BUILD_VERSION from Cargo.toml; local builds default to "0.1".
const VERSION: &str = env!("BUILD_VERSION");

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Convert AppError to HandlerResult for teloxide dispatcher compatibility
fn into_handler_error(e: AppError) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(e)
}

#[tokio::main]
async fn main() {
    // Load .env file for API keys
    let _ = dotenvy::dotenv();

    // Load configuration from YAML file + env secrets
    // Done before tracing init so log_level from config is available
    let config = match Config::load() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("Failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    // Initialize tracing (RUST_LOG set by Config::load from config.yaml)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(false)
        .init();

    info!("Starting Obsidian AI Agent V{}...", VERSION);

    // Initialize OpenRouter client (for classification)
    let ai_client = match OpenRouterClient::new(config.openrouter_api_key.clone()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            error!(error = %e, "Failed to create OpenRouter client");
            std::process::exit(1);
        }
    };

    // Initialize Whisper client (for voice transcription)
    let whisper_client = match WhisperClient::new(
        config.openai_api_key.clone(),
        config.whisper_model.clone(),
        config.whisper_language.clone(),
    ) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            error!(error = %e, "Failed to create Whisper client");
            std::process::exit(1);
        }
    };

    // Initialize vault manager (loads daily note settings from .obsidian/daily-notes.json)
    let vault = Arc::new(
        DailyNoteManager::new(
            config.vault_path.clone(),
            config.date_display_format.clone(),
        )
        .await,
    );

    // Initialize Telegram bot
    let bot = Bot::new(&config.teloxide_token);

    // Send startup notification to admin (if configured)
    if let Some(admin_chat_id) = config.admin_chat_id {
        let startup_msg = format!("Started agent V{}", VERSION);
        match bot.send_message(ChatId(admin_chat_id), &startup_msg).await {
            Ok(_) => info!(chat_id = admin_chat_id, "Startup notification sent"),
            Err(e) => {
                warn!(error = %e, chat_id = admin_chat_id, "Failed to send startup notification")
            }
        }
    }

    // Initialize conflict resolver
    let conflict_resolver = conflict::ConflictResolver::new(bot.clone());
    let conflict_pending = conflict_resolver.pending_map();

    // Initialize chat_id tracker
    let chat_tracker = chat_tracker::ChatIdTracker::new();

    // Pending transcript requests for inline callback workflow
    let transcript_pending: TranscriptPending = Arc::new(Mutex::new(HashMap::new()));

    // Initialize git sync with debouncing (if enabled)
    let sync_notifier: Option<debounce::SyncNotifier> = if config.git_sync_enabled {
        let git_path = config
            .git_path
            .clone()
            .expect("GIT_PATH required when git sync enabled");
        let git_sync = Arc::new(GitSync::new(
            git_path,
            config.git_remote_name.clone(),
            config.git_branch.clone(),
            config.git_ssh_key_path.clone(),
        ));
        Some(debounce::spawn_debounced_sync(
            git_sync.clone(),
            config.git_sync_debounce_secs,
            conflict_resolver,
            ai_client.clone(),
            config.clone(),
            chat_tracker.clone(),
        ))
    } else {
        info!("Git sync disabled (GIT_SYNC_ENABLED=false)");
        None
    };

    // Create shared handler context
    let handler_ctx = HandlerContext::new(
        config.clone(),
        ai_client.clone(),
        vault.clone(),
        sync_notifier.clone(),
        chat_tracker.clone(),
    );

    info!(
        vault_path = %config.vault_path.display(),
        git_sync_enabled = config.git_sync_enabled,
        git_remote = %config.git_remote_name,
        git_branch = %config.git_branch,
        debounce_secs = config.git_sync_debounce_secs,
        timezone = %config.timezone,
        allowed_users = ?config.allowed_user_ids,
        "Bot configured"
    );

    // Build dispatcher
    let handler = schema();

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            config.clone(),
            ai_client.clone(),
            whisper_client.clone(),
            vault.clone(),
            sync_notifier.clone(),
            conflict_pending.clone(),
            handler_ctx.clone(),
            transcript_pending.clone()
        ])
        .default_handler(|upd| async move {
            warn!(update_id = upd.id.0, "Unhandled update");
        })
        .error_handler(LoggingErrorHandler::with_custom_text("Error in handler"))
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    let message_handler = Update::filter_message().endpoint(handle_message);

    let callback_handler = Update::filter_callback_query().endpoint(handle_callback);

    dptree::entry()
        .branch(message_handler)
        .branch(callback_handler)
}

async fn handle_message(
    bot: Bot,
    msg: Message,
    ctx: HandlerContext,
    whisper_client: Arc<WhisperClient>,
    transcript_pending: TranscriptPending,
) -> HandlerResult {
    // Route based on message content type
    if msg.photo().is_some() {
        handlers::photo::handle_photo_message(bot, msg, ctx)
            .await
            .map_err(into_handler_error)
    } else if msg.voice().is_some() {
        handlers::voice::handle_voice_message(bot, msg, ctx, whisper_client)
            .await
            .map_err(into_handler_error)
    } else if msg.text().is_some() {
        handlers::text::handle_text_message(bot, msg, ctx, transcript_pending)
            .await
            .map_err(into_handler_error)
    } else {
        bot.send_message(
            msg.chat.id,
            "I can process text, voice, and photo messages. Please send one of those!",
        )
        .await?;
        Ok(())
    }
}

async fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    conflict_pending: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<
                String,
                tokio::sync::oneshot::Sender<conflict::ConflictResolution>,
            >,
        >,
    >,
    transcript_pending: TranscriptPending,
    config: Arc<Config>,
    ai_client: Arc<OpenRouterClient>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<debounce::SyncNotifier>,
    ctx: HandlerContext,
) -> HandlerResult {
    // Enforce same authorization policy as message handlers before processing callbacks.
    if !config.allowed_user_ids.is_empty() && !config.is_user_allowed(q.from.id.0) {
        info!(
            user_id = q.from.id.0,
            "Unauthorized user, ignoring callback"
        );
        return Ok(());
    }

    // Track latest active chat on callbacks when available.
    if let Some(ref msg) = q.message {
        ctx.chat_tracker.set(msg.chat().id);
    }

    if let Some(ref data) = q.data {
        if data.starts_with("yt_transcript:") {
            return handlers::url::handle_transcript_callback(
                bot,
                q,
                transcript_pending,
                ai_client,
                vault,
                config,
                sync_notifier,
            )
            .await
            .map_err(into_handler_error);
        } else if data.starts_with("conflict:") {
            return conflict::handle_conflict_callback(bot, q, conflict_pending).await;
        }
    }

    if let Some(ref data) = q.data {
        warn!(callback_data = %data, "Unknown callback type");
    }
    bot.answer_callback_query(&q.id).await?;
    Ok(())
}
