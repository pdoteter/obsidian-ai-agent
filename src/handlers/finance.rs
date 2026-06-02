use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use teloxide::dispatching::UpdateHandler;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tracing::{debug, error, info, warn};

use crate::ai::{AiService, ChatMessage};
use crate::config::Config;
use crate::error::{AiError, ImageError};
use crate::git::debounce::SyncNotifier;

const DEFAULT_FINANCE_TRANSACTION_GUIDE: &str = r#"You are an expert financial ledger assistant. Your task is to update an Obsidian markdown note representing a stock/crypto position ledger with a new transaction.
The note represents a single equity/stock/crypto/asset (e.g., AAPL).
You must parse the new transaction details (Buy/Sell, Open/Close, price, size/quantity, profit/loss, time) and update the note.

Maintain a frontmatter section with exactly:
---
symbol: <uppercase asset symbol, e.g. AAPL>
status: <"open" or "closed">
position_size: <current total shares/contracts/units held, 0 if closed>
average_entry: <average entry price of the currently open position, 0 if closed. Calculate using weighted average when buying more, resetting appropriately when completely closed>
realized_profit: <total accumulated realized profit/loss from closed positions/partial sales for this asset in USD/etc.>
last_updated: <current ISO-8601 date-time>
---

Below the frontmatter, write a clear `# <symbol> Ledger` header.
Then, maintain a beautiful Markdown table of transactions under `## Transactions`.
Columns: Date | Action (BUY/SELL or OPEN/CLOSE) | Price | Quantity | Profit/Loss | Notes
Add the new transaction to this table.
If an image attachment link is provided (e.g., `![[Finance/Assets/...]]`), you MUST include it inside the transaction notes column or in a notes section so it is linked properly!

If a message source/origin is specified in the transaction prompt, you MUST record this source clearly inside the transaction's "Notes" column (e.g. `[Source: <source>]`).

If the transaction details do NOT specify a quantity/position size, do NOT assume a default value of 1. Instead, leave the Quantity column in the table empty (or blank) and do not alter or recalculate the existing frontmatter fields (like position_size or average_entry) based on this transaction.

Maintain the historical notes/details inside the notes column or in a `## Notes` section at the bottom.
Return the entire updated note content as markdown.
"#;

#[derive(Debug, Deserialize, Serialize)]
struct FinanceClassification {
    #[serde(rename = "type")]
    msg_type: String, // "transaction", "question", "unknown"
    symbol: Option<String>,
    is_general_question: bool,
    is_sell: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TransactionUpdateResponse {
    updated_content: String,
    reply: String,
}

pub fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    Update::filter_message().endpoint(handle_finance_message)
}

/// Core entry point for incoming finance bot messages
pub async fn handle_finance_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    sync_notifier: Option<SyncNotifier>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Check user authorization
    if let Some(user) = msg.from.as_ref() {
        if !config.is_finance_user_allowed(user.id.0) {
            info!(
                user_id = user.id.0,
                "Unauthorized user, ignoring finance bot message"
            );
            return Ok(());
        }
    }

    if msg.photo().is_some() {
        handle_photo_message(bot, msg, config, ai_service, sync_notifier).await
    } else if msg.voice().is_some() {
        handle_voice_message(bot, msg, config, ai_service, sync_notifier).await
    } else if msg.text().is_some() {
        handle_text_message(bot, msg, config, ai_service, sync_notifier).await
    } else {
        bot.send_message(
            msg.chat.id,
            "I can process text, voice, and photo messages for your portfolio. Please send one of those!",
        )
        .await?;
        Ok(())
    }
}

/// Handle voice note updates (download -> transcribe -> handle as text)
async fn handle_voice_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    sync_notifier: Option<SyncNotifier>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let voice = match msg.voice() {
        Some(v) => v.clone(),
        None => return Ok(()),
    };

    info!(
        duration_secs = %voice.duration,
        file_size = voice.file.size,
        "Finance Bot: Processing voice message"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    let audio_bytes = crate::audio::download::download_voice_to_memory(&bot, &voice)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download finance voice message");
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

    let transcript = match ai_service.transcribe(&audio_bytes).await {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "Finance voice transcription failed");
            bot.send_message(msg.chat.id, format!("❌ Voice transcription failed: {}", e))
                .await?;
            return Ok(());
        }
    };

    info!(
        transcript_length = transcript.len(),
        "Voice transcription completed for Finance bot"
    );

    handle_text_inner(
        bot,
        msg,
        config,
        ai_service,
        sync_notifier,
        &transcript,
        None,
    )
    .await
}

/// Handle photo updates (download -> resize -> Vision analysis -> handle as text with photo link)
async fn handle_photo_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    sync_notifier: Option<SyncNotifier>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let photos = msg.photo().ok_or("No photo payload").map_err(|e| {
        Box::new(ImageError::Download(e.to_string())) as Box<dyn std::error::Error + Send + Sync>
    })?;
    let photo = photos.last().ok_or("Empty photos array").map_err(|e| {
        Box::new(ImageError::Download(e.to_string())) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let caption = msg.caption().map(|s| s.to_string());

    info!(
        size_bytes = photo.file.size,
        has_caption = caption.is_some(),
        "Finance Bot: Downloading photo"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::UploadPhoto)
        .await?;

    let file = bot.get_file(&photo.file.id).await.map_err(|e| {
        error!(error = %e, "Failed to get photo metadata");
        Box::new(ImageError::Download(e.to_string())) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let mut bytes = Vec::new();
    bot.download_file(&file.path, &mut bytes)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to download photo");
            Box::new(ImageError::Download(e.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

    // Resize
    let resized =
        crate::image::process::resize_image(&bytes, config.image.max_dimension).map_err(|e| {
            error!(error = %e, "Failed to resize photo");
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })?;

    // Save image
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let uuid_str = uuid::Uuid::new_v4().to_string();
    let uuid_suffix = crate::utils::safe_truncate(&uuid_str, 4);
    let filename = format!("{}-finance-{}.jpg", today, uuid_suffix);

    let resolved_assets_folder = std::path::Path::new(&config.finance.folder)
        .join(&config.finance.assets_folder)
        .to_string_lossy()
        .replace('\\', "/");

    let saved_path = crate::image::process::save_image(
        &resized,
        &config.vault_path,
        &resolved_assets_folder,
        &filename,
    )
    .await
    .map_err(|e| {
        error!(error = %e, "Failed to save photo");
        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
    })?;

    info!(
        path = %saved_path.display(),
        filename = %filename,
        "Finance Bot: Photo saved successfully"
    );

    // Run Vision to get description
    let base64 = crate::image::process::encode_base64(&resized);
    let guide = load_finance_guide(&config.finance.guide_path).await;

    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    let vision_result = ai_service
        .classify_image(
            &base64,
            caption.as_deref(),
            "",
            &config.openrouter_model_classify,
            Some(&guide),
        )
        .await;

    let image_description = match vision_result {
        Ok(c) => c.summary,
        Err(e) => {
            warn!(error = %e, "Vision classification failed, using raw caption");
            caption
                .clone()
                .unwrap_or_else(|| "Attached photo".to_string())
        }
    };

    let processed_text = if let Some(ref cap) = caption {
        format!("{} [Image Details: {}]", cap, image_description)
    } else {
        format!("[Attached photo describing: {}]", image_description)
    };

    let wiki_link = format!("![[{}/{}]]", resolved_assets_folder, filename);

    handle_text_inner(
        bot,
        msg,
        config,
        ai_service,
        sync_notifier,
        &processed_text,
        Some(wiki_link),
    )
    .await
}

/// Handle text note updates (direct or forwards)
async fn handle_text_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    sync_notifier: Option<SyncNotifier>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return Ok(()),
    };

    handle_text_inner(bot, msg, config, ai_service, sync_notifier, &text, None).await
}

/// Core inner text handling helper
async fn handle_text_inner(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    ai_service: Arc<AiService>,
    sync_notifier: Option<SyncNotifier>,
    text: &str,
    photo_wiki_link: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        text_length = text.len(),
        "Finance Bot: Processing message text"
    );

    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    // 1. Classification & Symbol Extraction
    let classification = match classify_message_intent(ai_service.clone(), &config, text).await {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Failed to classify message intent");
            bot.send_message(
                msg.chat.id,
                format!("❌ Failed to understand message intent: {}", e),
            )
            .await?;
            return Ok(());
        }
    };

    info!(
        intent = %classification.msg_type,
        symbol = ?classification.symbol,
        "Finance Bot: Classified intent"
    );

    match classification.msg_type.as_str() {
        "transaction" => {
            let Some(symbol) = classification.symbol else {
                bot.send_message(
                    msg.chat.id,
                    "⚠️ I recognized this is a transaction but couldn't extract the asset symbol. Please specify the symbol (e.g. AAPL, BTC).",
                )
                .await?;
                return Ok(());
            };

            let symbol = symbol.to_uppercase();
            bot.send_chat_action(msg.chat.id, ChatAction::Typing)
                .await?;

            // Pull latest changes from vault before transaction update
            if let Some(ref notifier) = sync_notifier {
                match notifier.pull_if_idle().await {
                    Ok(Some(result)) => {
                        info!(result = %result, "Completed pre-transaction git pull for finance bot");
                    }
                    Ok(None) => {
                        debug!("Skipping pre-transaction git pull because debounce worker is busy");
                    }
                    Err(error) => {
                        warn!(error = %error, "Pre-transaction git pull failed, continuing with local files");
                    }
                }
            }

            let source = get_message_source(&msg);

            // Process transaction note update
            let reply = match handle_transaction_update(
                ai_service,
                &config,
                &symbol,
                text,
                photo_wiki_link,
                source,
                classification.is_sell.unwrap_or(false),
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!(error = %e, symbol = %symbol, "Failed to process ledger update");
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ Failed to update ledger note for {}: {}", symbol, e),
                    )
                    .await?;
                    return Ok(());
                }
            };

            // Notify git sync
            if let Some(ref notifier) = sync_notifier {
                notifier.notify();
            }

            bot.send_message(msg.chat.id, reply).await?;
        }
        "question" => {
            bot.send_chat_action(msg.chat.id, ChatAction::Typing)
                .await?;

            // Pull latest changes from vault before answering a query
            if let Some(ref notifier) = sync_notifier {
                match notifier.pull_if_idle().await {
                    Ok(Some(result)) => {
                        info!(result = %result, "Completed pre-query git pull for finance bot");
                    }
                    Ok(None) => {
                        debug!("Skipping pre-query git pull because debounce worker is busy");
                    }
                    Err(error) => {
                        warn!(error = %error, "Pre-query git pull failed, continuing with local files");
                    }
                }
            }

            let reply = match handle_portfolio_query(
                ai_service,
                &config,
                classification.symbol.as_deref(),
                text,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!(error = %e, "Failed to answer portfolio query");
                    bot.send_message(msg.chat.id, format!("❌ Failed to answer query: {}", e))
                        .await?;
                    return Ok(());
                }
            };

            bot.send_message(msg.chat.id, reply).await?;
        }
        _ => {
            bot.send_message(
                msg.chat.id,
                "❓ I'm not sure how to process that. You can send me position transactions to log (e.g. BUY AAPL 10 shares @ 175) or ask me portfolio questions (e.g. Do I have AAPL?).",
            )
            .await?;
        }
    }

    Ok(())
}

/// Use LLM to classify if message is a transaction or question and extract symbol
async fn classify_message_intent(
    ai_service: Arc<AiService>,
    config: &Config,
    text: &str,
) -> Result<FinanceClassification, AiError> {
    let classification_system_prompt = r#"You are a financial assistant for an Obsidian-based portfolio tracker.
Analyze the user's message and determine:
1. Is it a question (e.g. "Do I have AAPL?", "What are my profits?", "Show my positions", "What is my BTC position?")?
2. Is it a position transaction (buy, sell, open, close, entry, exit, forwarded trading signal, etc.)?
3. Is it unknown/other?

For transaction types, also determine if it is a sell/close order (e.g. sell, close, exit, short).

You must respond with a JSON object in this exact format:
{
  "type": "question" | "transaction" | "unknown",
  "symbol": "extracted asset symbol in uppercase, e.g. AAPL, BTC, or null if none/general",
  "is_general_question": true | false,
  "is_sell": true | false | null
}

Do not include any explanation or markdown formatting in your response. Return raw JSON only."#;

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: classification_system_prompt.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        },
    ];

    let response = ai_service
        .chat_completion(&config.openrouter_model_classify, messages, Some(20480))
        .await?;

    let cleaned = clean_json_response(&response);

    serde_json::from_str::<FinanceClassification>(&cleaned).map_err(|e| {
        AiError::ParseError(format!(
            "Failed to parse intent classification: {}. Raw: {}",
            e, response
        ))
    })
}

/// Process transaction update, read note, call LLM to update ledger, write note
struct PositionCheck {
    has_position: bool,
    account: Option<String>,
}

fn check_existing_position(note_content: &str) -> PositionCheck {
    let (frontmatter, _) = crate::vault::frontmatter::parse_frontmatter(note_content);
    let mut has_position = false;
    let mut account = None;

    if let Some(yaml) = frontmatter {
        if let Some(map) = yaml.as_mapping() {
            // Check position_size
            let pos_size = map
                .get(&serde_yml::Value::String("position_size".to_string()))
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                .unwrap_or(0.0);

            // Check status
            let status = map
                .get(&serde_yml::Value::String("status".to_string()))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if pos_size > 0.0 || status == "open" {
                has_position = true;
            }

            // Look for account/broker/portfolio in frontmatter (case-insensitive keys)
            for (key, val) in map.iter() {
                if let Some(k_str) = key.as_str() {
                    let k_lower = k_str.to_lowercase();
                    if k_lower == "account" || k_lower == "broker" || k_lower == "portfolio" {
                        if let Some(v_str) = val.as_str() {
                            account = Some(v_str.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    PositionCheck {
        has_position,
        account,
    }
}

/// Process transaction update, read note, call LLM to update ledger, write note
async fn handle_transaction_update(
    ai_service: Arc<AiService>,
    config: &Config,
    symbol: &str,
    new_transaction_text: &str,
    photo_wiki_link: Option<String>,
    source: Option<String>,
    is_sell: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let finance_dir = config.vault_path.join(&config.finance.folder);
    if !finance_dir.exists() {
        tokio::fs::create_dir_all(&finance_dir).await?;
        info!(dir = %finance_dir.display(), "Created finance directory");
    }

    let note_path = finance_dir.join(format!("{}.md", symbol));
    let current_note_content = if note_path.exists() {
        tokio::fs::read_to_string(&note_path).await?
    } else {
        String::new()
    };

    // Check position before updating if it's classified as a sell order
    let mut warning_prefix = String::new();
    if is_sell {
        let check = check_existing_position(&current_note_content);
        if !check.has_position {
            warning_prefix.push_str("⚠️ Note: You do not have an active open position for this asset in your ledger.\n\n");
        } else if let Some(acct) = check.account {
            warning_prefix.push_str(&format!(
                "ℹ️ Active position found in account: **{}**.\n\n",
                acct
            ));
        }
    }

    let finance_guide = load_finance_guide(&config.finance.guide_path).await;

    let transaction_update_system_prompt = format!(
        r#"You are a financial portfolio ledger assistant.
Your task is to process a new transaction for the asset symbol: {symbol}.
You must respond with a JSON object in this exact format:
{{
  "updated_content": "the complete new markdown content for the note, including YAML frontmatter, header, transactions table, and notes",
  "reply": "a short, polite, user-friendly summary of what was done, including current position size and average entry price."
}}

Here is the custom portfolio instructions you must follow:
<finance_guide>
{finance_guide}
</finance_guide>

Do not include any explanation or markdown formatting in your response. Return raw JSON only."#
    );

    let image_suffix = if let Some(ref link) = photo_wiki_link {
        format!("An image was attached. You MUST embed/link this image in the ledger using this exact markdown: {}", link)
    } else {
        String::new()
    };

    let source_suffix = if let Some(ref src) = source {
        format!("\nMessage source/origin: {}\n(You MUST record this source in the transaction's Notes column.)", src)
    } else {
        String::new()
    };

    let user_message = format!(
        r#"Here is the current Obsidian note content for {symbol} (it may be empty if this is the first transaction):
=== CURRENT NOTE CONTENT ===
{current_note_content}
============================

The new transaction message from the user is:
"{new_transaction_text}"
{source_suffix}

{image_suffix}
Please update the note and return the JSON object."#
    );

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: transaction_update_system_prompt,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_message,
        },
    ];

    let response = ai_service
        .chat_completion(&config.openrouter_model_classify, messages, Some(2048))
        .await?;

    let cleaned = clean_json_response(&response);

    let parsed = serde_json::from_str::<TransactionUpdateResponse>(&cleaned).map_err(|e| {
        AiError::ParseError(format!(
            "Failed to parse transaction update JSON: {}. Raw: {}",
            e, response
        ))
    })?;

    // Write updated content back to note
    tokio::fs::write(&note_path, &parsed.updated_content).await?;

    let mut final_reply = parsed.reply;
    if !warning_prefix.is_empty() {
        final_reply = format!("{}{}", warning_prefix, final_reply);
    }

    Ok(final_reply)
}

/// Answer natural language portfolio question
async fn handle_portfolio_query(
    ai_service: Arc<AiService>,
    config: &Config,
    target_symbol: Option<&str>,
    user_question: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let finance_dir = config.vault_path.join(&config.finance.folder);
    if !finance_dir.exists() {
        return Ok("You don't have any portfolio tracking files yet. Send me a transaction to log to get started!".to_string());
    }

    let mut portfolio_data = String::new();

    if let Some(symbol) = target_symbol {
        let note_path = finance_dir.join(format!("{}.md", symbol.to_uppercase()));
        if note_path.exists() {
            let content = tokio::fs::read_to_string(&note_path).await?;
            portfolio_data = format!("=== File: {}.md ===\n{}\n", symbol.to_uppercase(), content);
        } else {
            portfolio_data = format!(
                "No record exists for the asset symbol: {}.\n",
                symbol.to_uppercase()
            );
        }
    } else {
        // Read all files in the Finance directory
        let mut entries = tokio::fs::read_dir(&finance_dir).await?;
        let mut file_count = 0;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "md") {
                let filename = path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("unknown");
                let content = tokio::fs::read_to_string(&path).await?;
                portfolio_data.push_str(&format!("=== File: {} ===\n{}\n\n", filename, content));
                file_count += 1;
            }
        }
        if file_count == 0 {
            return Ok("You don't have any portfolio tracking files yet. Send me a transaction to log to get started!".to_string());
        }
    }

    let finance_guide = load_finance_guide(&config.finance.guide_path).await;

    let qa_system_prompt = format!(
        r#"You are a financial portfolio helper. You are answering a question about the user's investments based on their Obsidian ledger notes.
Here is the custom portfolio guide you must follow:
<finance_guide>
{finance_guide}
</finance_guide>

Please answer the question accurately, professionally, and concisely based on the provided ledger notes.
Format the response using clear, clean markdown."#
    );

    let user_message = format!(
        r#"Here is the current portfolio data:
=== PORTFOLIO DATA ===
{portfolio_data}
======================

User question: "{user_question}"
Please answer."#
    );

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: qa_system_prompt,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_message,
        },
    ];

    let response = ai_service
        .chat_completion(&config.openrouter_model_classify, messages, Some(1024))
        .await?;

    Ok(response)
}

/// Helper to load custom finance guide if configured, or fall back to default
async fn load_finance_guide(path: &Option<PathBuf>) -> String {
    if let Some(ref p) = path {
        if p.exists() {
            match tokio::fs::read_to_string(p).await {
                Ok(content) => return content,
                Err(e) => {
                    error!(error = %e, "Failed to read finance guide file, using default");
                }
            }
        }
    }
    DEFAULT_FINANCE_TRANSACTION_GUIDE.to_string()
}

/// Helper to sanitize and clean markdown formatting from LLM JSON responses
fn clean_json_response(content: &str) -> String {
    let mut cleaned = content.trim().to_string();
    if cleaned.starts_with("```json") {
        cleaned = cleaned.replace("```json", "");
    } else if cleaned.starts_with("```") {
        cleaned = cleaned.replace("```", "");
    }
    if cleaned.ends_with("```") {
        cleaned = cleaned[..cleaned.len() - 3].to_string();
    }
    cleaned.trim().to_string()
}

/// Extract origin/source from a Telegram message
fn get_message_source(msg: &Message) -> Option<String> {
    if let Some(chat) = msg.forward_from_chat() {
        if let Some(username) = chat.username() {
            return Some(format!("@{}", username));
        } else if let Some(title) = chat.title() {
            return Some(title.to_string());
        }
    }

    if let Some(user) = msg.forward_from_user() {
        if let Some(username) = &user.username {
            return Some(format!("@{}", username));
        } else {
            return Some(user.full_name());
        }
    }

    if let Some(sig) = msg.forward_author_signature() {
        return Some(sig.to_string());
    }

    if let Some(name) = msg.forward_from_sender_name() {
        return Some(name.to_string());
    }

    if !msg.chat.is_private() {
        if let Some(username) = msg.chat.username() {
            return Some(format!("@{}", username));
        } else if let Some(title) = msg.chat.title() {
            return Some(title.to_string());
        }
    }

    if let Some(user) = &msg.from {
        if let Some(username) = &user.username {
            return Some(format!("@{}", username));
        } else {
            return Some(user.full_name());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_json_response() {
        let raw = "```json\n{\n  \"type\": \"transaction\"\n}\n```";
        let cleaned = clean_json_response(raw);
        assert_eq!(cleaned, "{\n  \"type\": \"transaction\"\n}");

        let raw_no_json = "```\n{\n  \"type\": \"transaction\"\n}\n```";
        let cleaned_no_json = clean_json_response(raw_no_json);
        assert_eq!(cleaned_no_json, "{\n  \"type\": \"transaction\"\n}");
    }

    #[test]
    fn test_check_existing_position_open() {
        let content = "---\nsymbol: AAPL\nstatus: open\nposition_size: 15\naverage_entry: 150.0\naccount: Personal Port\n---";
        let res = check_existing_position(content);
        assert!(res.has_position);
        assert_eq!(res.account, Some("Personal Port".to_string()));
    }

    #[test]
    fn test_check_existing_position_open_with_broker() {
        let content = "---\nsymbol: TSLA\nstatus: open\nposition_size: 5\naverage_entry: 200.0\nbroker: Interactive Brokers\n---";
        let res = check_existing_position(content);
        assert!(res.has_position);
        assert_eq!(res.account, Some("Interactive Brokers".to_string()));
    }

    #[test]
    fn test_check_existing_position_open_with_portfolio() {
        let content = "---\nsymbol: BTC\nstatus: open\nposition_size: 0.5\naverage_entry: 60000.0\nportfolio: Crypto Wallet\n---";
        let res = check_existing_position(content);
        assert!(res.has_position);
        assert_eq!(res.account, Some("Crypto Wallet".to_string()));
    }

    #[test]
    fn test_check_existing_position_closed() {
        let content = "---\nsymbol: AAPL\nstatus: closed\nposition_size: 0\naverage_entry: 0\naccount: Personal Port\n---";
        let res = check_existing_position(content);
        assert!(!res.has_position);
    }

    #[test]
    fn test_check_existing_position_empty_and_no_frontmatter() {
        let res_empty = check_existing_position("");
        assert!(!res_empty.has_position);
        assert_eq!(res_empty.account, None);

        let res_no_fm = check_existing_position("# AAPL Ledger\nSome content");
        assert!(!res_no_fm.has_position);
        assert_eq!(res_no_fm.account, None);
    }
}
