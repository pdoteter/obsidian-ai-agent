mod ai;
mod audio;
mod config;
mod error;
mod git;
mod handlers;
mod vault;

use std::sync::Arc;

use teloxide::dispatching::UpdateHandler;
use teloxide::prelude::*;
use tracing::{error, info, warn};

use ai::client::OpenRouterClient;
use ai::transcribe::WhisperClient;
use config::Config;
use git::conflict;
use git::debounce;
use git::sync::GitSync;
use vault::daily_note::DailyNoteManager;

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

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

    info!("Starting Obsidian AI Agent...");

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
    let vault = Arc::new(DailyNoteManager::new(config.vault_path.clone(), config.date_display_format.clone()).await);

    // Initialize git sync with debouncing (if enabled)
    let sync_notifier: Option<debounce::SyncNotifier> = if config.git_sync_enabled {
        let git_path = config.git_path.clone().expect("GIT_PATH required when git sync enabled");
        let git_sync = Arc::new(GitSync::new(
            git_path,
            config.git_remote_name.clone(),
            config.git_branch.clone(),
            config.git_ssh_key_path.clone(),
        ));
        Some(debounce::spawn_debounced_sync(git_sync.clone(), config.git_sync_debounce_secs))
    } else {
        info!("Git sync disabled (GIT_SYNC_ENABLED=false)");
        None
    };

    // Initialize Telegram bot
    let bot = Bot::new(&config.teloxide_token);

    // Initialize conflict resolver
    let conflict_resolver = conflict::ConflictResolver::new(bot.clone());
    let conflict_pending = conflict_resolver.pending_map();

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
            conflict_pending.clone()
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
    config: Arc<Config>,
    ai_client: Arc<OpenRouterClient>,
    whisper_client: Arc<WhisperClient>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<debounce::SyncNotifier>,
) -> HandlerResult {
    // Route based on message content type
    if msg.voice().is_some() {
        handlers::voice::handle_voice_message(bot, msg, config, ai_client, whisper_client, vault, sync_notifier)
            .await
    } else if msg.text().is_some() {
        handlers::text::handle_text_message(bot, msg, config, ai_client, vault, sync_notifier).await
    } else {
        bot.send_message(
            msg.chat.id,
            "I can process text and voice messages. Please send one of those!",
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
) -> HandlerResult {
    conflict::handle_conflict_callback(bot, q, conflict_pending).await
}
