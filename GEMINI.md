# GEMINI.md

## Project Overview

**Obsidian AI Agent** is a specialized Telegram bot written in Rust that bridges the gap between mobile information capture and an Obsidian vault. It allows users to send text, voice notes, and photos to a Telegram bot, which then automatically:
- **Transcribes** voice messages using OpenAI Whisper.
- **Describes and extracts EXIF metadata** from photos.
- **Classifies** the content using AI (via OpenRouter/Gemini).
- **Appends** the structured entry to the user's Obsidian Daily Note.
- **Summarizes URLs** and optionally extracts full YouTube transcripts.
- **Synchronizes** the vault using Git with automated conflict resolution.

### Core Architecture
- **`src/main.rs`**: Entry point and Telegram update dispatcher.
- **`src/handlers/`**: Type-specific message processing (Text, Voice, Photo, URL).
- **`src/ai/`**: Clients for OpenRouter (classification) and Whisper (transcription).
- **`src/vault/`**: Obsidian-specific logic, including Daily Note template parsing and Frontmatter management.
- **`src/git/`**: Automated Git sync, debouncing, and a Telegram-based conflict resolution workflow.

## Building and Running

### Prerequisites
- Rust 1.85+
- `yt-dlp` (optional, for YouTube transcripts)
- API Keys: Telegram (BotFather), OpenRouter, OpenAI.

### Commands
- **Build**: `cargo build --release`
- **Run**: `./target/release/obsidian-ai-agent`
- **Test**: `cargo test`
- **Lint**: `cargo clippy`
- **Format**: `cargo fmt`

### Docker Deployment
```bash
docker compose up -d
```
The project includes a `Dockerfile` and `docker-compose.yaml` for easy deployment, including automatic UID/GID matching for vault permissions.

## Development Conventions

- **Async Runtime**: Built on `tokio`. Use `async/await` throughout.
- **Error Handling**: Uses `thiserror` for defining custom error types. Prefer returning `Result` and using the `?` operator.
- **Logging**: Uses `tracing`. Log levels are configurable via `config.yaml`.
- **Configuration**:
    - **Secrets**: Managed via `.env` files (never commit these).
    - **Settings**: Managed via `config.yaml`.
    - **AI Behavior**: Custom rules are defined in `system-guide.md`.
- **Testing**: Includes unit tests (especially for config and vault logic). Always check for existing tests in the module you are modifying.
- **Vault Integrity**: Operations on the vault are performed via `DailyNoteManager` and `VaultWriter` to ensure consistent formatting and template adherence.

## Key Files

- `config.yaml.example`: Template for application settings.
- `system-guide.md`: Instructions for the AI classifier.
- `src/vault/daily_note.rs`: Core logic for managing Obsidian daily notes.
- `src/git/sync.rs`: Git synchronization implementation.
- `src/handlers/url.rs`: Extensive URL and YouTube processing logic.
