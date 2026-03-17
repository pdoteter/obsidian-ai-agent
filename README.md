# Obsidian AI Agent

Telegram bot die tekst- en spraakberichten omzet naar gestructureerde Obsidian daily notes.

## Vereisten

- Rust 1.85+
- ffmpeg (voor audio-conversie)
- Telegram Bot Token ([BotFather](https://t.me/BotFather))
- OpenRouter API Key ([openrouter.ai](https://openrouter.ai))

## Setup

1. Kopieer `.env.example` naar `.env` en vul de waardes in
2. `cargo build --release`
3. `./target/release/obsidian-ai-agent`

## Docker

```bash
docker compose up -d
```

## Omgevingsvariabelen

| Variabele | Verplicht | Default | Beschrijving |
|-----------|-----------|---------|-------------|
| `TELOXIDE_TOKEN` | ✅ | - | Telegram Bot API token |
| `OPENROUTER_API_KEY` | ✅ | - | OpenRouter API key |
| `VAULT_PATH` | ✅ | - | Pad naar Obsidian vault |
| `GIT_REMOTE_NAME` | ❌ | `origin` | Git remote naam |
| `GIT_BRANCH` | ❌ | `main` | Git branch |
| `GIT_SSH_KEY_PATH` | ❌ | auto | Pad naar SSH private key |
| `GIT_SYNC_DEBOUNCE_SECS` | ❌ | `300` | Seconden wachten voor git sync |
| `OPENROUTER_MODEL_TRANSCRIBE` | ❌ | `google/gemini-2.5-flash` | Model voor transcriptie |
| `OPENROUTER_MODEL_CLASSIFY` | ❌ | `google/gemini-2.5-flash` | Model voor classificatie |
| `ALLOWED_USER_IDS` | ❌ | alle | Comma-separated Telegram user IDs |
| `RUST_LOG` | ❌ | `info` | Log level |
