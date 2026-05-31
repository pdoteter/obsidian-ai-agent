mod ai;
mod audio;
mod config;
mod error;
mod git;
mod handlers;
mod image;
mod url;
mod utils;
mod vault;
mod webui;

use std::collections::HashMap;
use std::sync::Arc;

use teloxide::dispatching::UpdateHandler;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use ai::providers::gemini::GeminiClient;
use ai::providers::openai_whisper::WhisperClient;
use ai::providers::openrouter::OpenRouterClient;
use ai::{AiProvider, AiService};
use config::Config;
use git::chat_tracker;
use git::conflict;
use git::debounce;
use git::sync::GitSync;
use handlers::url::TranscriptPending;
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

    // Initialize AI Providers
    let mut providers: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();

    // OpenRouter Provider
    let openrouter = match OpenRouterClient::new(config.openrouter_api_key.clone()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            error!(error = %e, "Failed to create OpenRouter client");
            std::process::exit(1);
        }
    };
    providers.insert("openrouter".to_string(), openrouter.clone());

    // OpenAI Whisper Provider
    let whisper = match WhisperClient::new(
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
    providers.insert("openai".to_string(), whisper);

    // Google Gemini Provider (Optional on startup, warning only if credentials missing)
    match GeminiClient::new(
        config.gemini_api_key.clone(),
        config.gemini_service_account_key_path.clone(),
    )
    .await
    {
        Ok(c) => {
            providers.insert("gemini".to_string(), Arc::new(c));
        }
        Err(e) => {
            warn!(error = %e, "Gemini client could not be initialized on startup");
        }
    }

    // Initialize AI Service (Orchestrator)
    let ai_service = Arc::new(AiService::new(providers, &config));

    // Initialize Telegram bot
    let bot = Bot::new(&config.teloxide_token);

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
            ai_service.clone(),
            config.clone(),
            chat_tracker.clone(),
        ))
    } else {
        info!("Git sync disabled (GIT_SYNC_ENABLED=false)");
        None
    };

    // Create broadcast channel for real-time vault update signals
    let (update_tx, _) = tokio::sync::broadcast::channel::<()>(100);

    // Initialize vault manager (loads daily note settings from .obsidian/daily-notes.json)
    let vault = Arc::new(
        DailyNoteManager::new(
            config.vault_path.clone(),
            config.date_display_format.clone(),
            sync_notifier.clone(),
            Some(update_tx),
        )
        .await,
    );

    // Start concurrent WebUI Server if enabled
    let _webui_task = if config.webui_enabled {
        // Resolve authentication token: if None, generate a random secure passcode and print it
        let auth_token = match &config.webui_auth_token {
            Some(t) => {
                info!("WebUI: Using configured authentication token");
                Some(t.clone())
            }
            None => {
                let generated: String = uuid::Uuid::new_v4().to_string().chars().take(8).collect();
                info!("**********************************************************");
                info!("⚠️  WebUI: No WEBUI_AUTH_TOKEN configured!");
                info!("🔑 Generated temporary Access Token: {}", generated);
                info!(
                    "👉 Access the portal at: http://127.0.0.1:{}?token={}",
                    config.webui_port, generated
                );
                info!("**********************************************************");
                Some(generated)
            }
        };

        // Create updated config with the resolved token so server has it
        let mut server_config = (*config).clone();
        server_config.webui_auth_token = auth_token;
        let server_config = Arc::new(server_config);

        let (ws_broadcast, _) = tokio::sync::broadcast::channel::<webui::server::WebuiEvent>(100);

        let webui_state = webui::server::WebuiState {
            config: server_config,
            ai_service: ai_service.clone(),
            vault: vault.clone(),
            sync_notifier: sync_notifier.clone(),
            ws_broadcast,
        };

        let port = config.webui_port;
        Some(tokio::spawn(async move {
            webui::server::start_server(webui_state, port).await;
        }))
    } else {
        info!("WebUI: Disabled (webui.enabled=false)");
        None
    };

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

    // Start concurrent Finance Bot if enabled
    let _finance_task = if config.finance.enabled {
        let finance_token = config
            .finance_teloxide_token
            .clone()
            .expect("Missing FINANCE_TELOXIDE_TOKEN when finance bot is enabled");

        info!("Starting Finance Bot...");
        let finance_bot = Bot::new(finance_token);
        let finance_handler = handlers::finance::schema();

        let config_clone = config.clone();
        let ai_service_clone = ai_service.clone();
        let sync_notifier_clone = sync_notifier.clone();

        Some(tokio::spawn(async move {
            Dispatcher::builder(finance_bot, finance_handler)
                .dependencies(dptree::deps![
                    config_clone,
                    ai_service_clone,
                    sync_notifier_clone
                ])
                .default_handler(|upd| async move {
                    warn!(update_id = upd.id.0, "Finance Bot: Unhandled update");
                })
                .error_handler(LoggingErrorHandler::with_custom_text(
                    "Error in Finance Bot handler",
                ))
                .build()
                .dispatch()
                .await;
        }))
    } else {
        None
    };

    // Build dispatcher for primary bot
    let handler = schema();

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            config.clone(),
            ai_service.clone(),
            vault.clone(),
            sync_notifier.clone(),
            conflict_pending.clone(),
            chat_tracker.clone(),
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

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<debounce::SyncNotifier>,
    chat_tracker: chat_tracker::ChatIdTracker,
    transcript_pending: TranscriptPending,
) -> HandlerResult {
    // Route based on message content type
    if msg.photo().is_some() {
        handlers::photo::handle_photo_message(
            bot,
            msg,
            config,
            ai_service,
            vault,
            sync_notifier,
            chat_tracker,
        )
        .await
    } else if msg.voice().is_some() {
        handlers::voice::handle_voice_message(
            bot,
            msg,
            config,
            ai_service,
            vault,
            sync_notifier,
            chat_tracker,
        )
        .await
    } else if msg.text().is_some() {
        handlers::text::handle_text_message(
            bot,
            msg,
            config,
            ai_service,
            vault,
            sync_notifier,
            chat_tracker,
            transcript_pending,
        )
        .await
    } else if let Some(doc) = msg.document() {
        if doc.mime_type.as_ref().map(|m| m.as_ref()) == Some("application/pdf")
            || doc
                .file_name
                .as_ref()
                .map(|f| f.ends_with(".pdf"))
                .unwrap_or(false)
        {
            handlers::pdf::handle_pdf_message(
                bot,
                msg,
                config,
                ai_service,
                vault,
                sync_notifier,
                chat_tracker,
            )
            .await
        } else {
            bot.send_message(
                msg.chat.id,
                "I currently only support PDF documents. Please send a valid PDF!",
            )
            .await?;
            Ok(())
        }
    } else {
        bot.send_message(
            msg.chat.id,
            "I can process text, voice, photo, and PDF messages. Please send one of those!",
        )
        .await?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
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
    ai_service: Arc<AiService>,
    vault: Arc<DailyNoteManager>,
    sync_notifier: Option<debounce::SyncNotifier>,
    chat_tracker: chat_tracker::ChatIdTracker,
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
        chat_tracker.set(msg.chat().id).await;
    }

    if let Some(ref data) = q.data {
        if data.starts_with("yt_transcript:") {
            return handlers::url::handle_transcript_callback(
                bot,
                q,
                transcript_pending,
                ai_service,
                vault,
                config,
                sync_notifier,
            )
            .await;
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
