# Modular Refactoring Blueprint (Phases 3 & 4)

This document prepares the conceptual groundwork for the subsequent messenger and multi-client refactoring phases.

## Phase 3: Messenger Abstraction

### Goal
Decouple handlers from Telegram-specific types (`teloxide::Bot`, `teloxide::types::Message`).

### Core Abstractions

#### 1. The `Messenger` Trait
```rust
#[async_trait]
pub trait Messenger: Send + Sync {
    /// Send a plain text message
    async fn send_text(&self, chat_id: &str, text: &str) -> Result<()>;

    /// Acknowledge a message (Reaction in TG, Emoji in Discord)
    async fn acknowledge(&self, msg_id: &str, chat_id: &str) -> Result<()>;

    /// Download a file from the platform (Voice/Photo)
    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>>;

    /// Show a platform-specific activity indicator (Typing...)
    async fn show_activity(&self, chat_id: &str, activity: Activity) -> Result<()>;
}

pub enum Activity {
    Typing,
    UploadingPhoto,
    None,
}
```

#### 2. Event Normalization
```rust
pub enum MessageContent {
    Text(String),
    Voice { file_id: String, duration: u32 },
    Photo { file_id: String, caption: Option<String> },
}

pub struct IncomingEvent {
    pub source_id: String, // e.g., "telegram_123"
    pub chat_id: String,
    pub user_id: u64,
    pub content: MessageContent,
}
```

---

## Phase 4: Multi-Client Support

### Goal
Run multiple platform loops simultaneously, sharing the same "Brain" (Core Services).

### Architecture Sketch

```ascii
                     ┌──────────────────┐
                     │    AgentCore     │
                     │ (Shared Services)│
                     └────────┬─────────┘
                              │
          ┌───────────────────┼───────────────────┐
          │                   │                   │
          ▼                   ▼                   ▼
   ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
   │  TG Client  │     │  DS Client  │     │ CLI Client  │
   │ (Listener)  │     │ (Listener)  │     │ (Listener)  │
   └─────────────┘     └─────────────┘     └─────────────┘
```

### Key Changes
- **`AgentCore`**: Central struct holding `Arc<DailyNoteManager>`, `Arc<AiService>`, `Arc<GitSync>`.
- **Event Dispatcher**: A single `match event` loop that routes `IncomingEvent` to the appropriate handler.
- **Initialization**: `main.rs` will spawn a task for each enabled messenger.

### Integration Challenges
- **Conflict Resolution**: Conflict notifications must be sent back to the *source* client that triggered the sync.
- **State Management**: Callback IDs for YouTube transcripts must be platform-specific or globally unique.
