# Obsidian AI Agent

Telegram bot that converts text and voice messages into structured Obsidian daily notes.

## Requirements

- Rust 1.85+
- Telegram Bot Token ([BotFather](https://t.me/BotFather))
- OpenRouter API Key ([openrouter.ai](https://openrouter.ai))
- OpenAI API Key ([platform.openai.com](https://platform.openai.com)) — for Whisper transcription

## Setup

1. Copy `.env.example` to `.env` and fill in the values
2. `cargo build --release`
3. `./target/release/obsidian-ai-agent`

## Docker

```bash
docker compose up -d
```

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `TELOXIDE_TOKEN` | ✅ | - | Telegram Bot API token |
| `OPENROUTER_API_KEY` | ✅ | - | OpenRouter API key (classification) |
| `OPENAI_API_KEY` | ✅ | - | OpenAI API key (Whisper transcription) |
| `VAULT_PATH` | ✅ | - | Path to Obsidian vault |
| `GIT_SYNC_ENABLED` | ❌ | `true` | Enable/disable git sync (`false` to disable) |
| `GIT_PATH` | ⚠️ | - | Path to git repo (required when git sync is enabled) |
| `GIT_REMOTE_NAME` | ❌ | `origin` | Git remote name |
| `GIT_BRANCH` | ❌ | `main` | Git branch |
| `GIT_SSH_KEY_PATH` | ❌ | auto | Path to SSH private key |
| `GIT_SYNC_DEBOUNCE_SECS` | ❌ | `300` | Seconds to wait before git sync |
| `WHISPER_MODEL` | ❌ | `whisper-1` | OpenAI Whisper model for transcription |
| `WHISPER_LANGUAGE` | ❌ | - | ISO-639-1 language code for Whisper (e.g. `nl`, `en`, `de`) |
| `OPENROUTER_MODEL_CLASSIFY` | ❌ | `google/gemini-2.5-flash` | Model for classification |
| `ALLOWED_USER_IDS` | ❌ | all | Comma-separated Telegram user IDs |
| `RUST_LOG` | ❌ | `info` | Log level |
