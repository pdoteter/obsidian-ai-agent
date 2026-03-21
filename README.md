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

Pre-built images are available on [Docker Hub](https://hub.docker.com/r/peterluxem/obsidian-ai-agent):

```bash
docker compose up -d
```

Images are automatically built and pushed on every commit to `main`.

To build locally instead, uncomment `build: .` in `docker-compose.yaml`.

### CI/CD Setup (GitHub Actions)

The workflow at `.github/workflows/docker.yml` builds and pushes to Docker Hub. Add these secrets to your GitHub repo (`Settings > Secrets > Actions`):

| Secret | Value |
|--------|-------|
| `DOCKERHUB_USERNAME` | Your Docker Hub username |
| `DOCKERHUB_TOKEN` | Docker Hub access token ([create one here](https://hub.docker.com/settings/security)) |

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
| `TZ` | ❌ | `Europe/Brussels` | Timezone for timestamps in daily notes (IANA format, e.g. `America/New_York`) |
| `RUST_LOG` | ❌ | `info` | Log level |

## Model Recommendations

### Transcription (`WHISPER_MODEL`)

These models are available via the OpenAI `/v1/audio/transcriptions` endpoint:

| Model | Quality | Speed | Cost | Notes |
|-------|---------|-------|------|-------|
| `gpt-4o-mini-transcribe` | Near-best accuracy | Fast | ~$0.003/min | **Recommended default** — cheaper and more accurate than `whisper-1` |
| `gpt-4o-transcribe` | Best accuracy | Slower | ~$0.006/min | Best choice when accuracy matters most |
| `whisper-1` | Good baseline | Fast | ~$0.006/min | Legacy model, can misdetect language (e.g. Dutch → German) |

**Tip:** If you primarily send voice messages in a single language, set `WHISPER_LANGUAGE` to avoid misdetection (e.g. `nl` for Dutch, `en` for English).

### Classification (`OPENROUTER_MODEL_CLASSIFY`)

These models are available via [OpenRouter](https://openrouter.ai/models). Classification is a lightweight task, so fast/cheap models work well:

| Model | Input $/M tokens | Notes |
|-------|-------------------|-------|
| `google/gemini-2.5-flash` | $0.30 | **Current default** — good balance of speed, quality, and cost |
| `google/gemini-2.0-flash-lite-001` | $0.25 | Cheaper alternative, 2.5× faster time-to-first-token |
| `anthropic/claude-haiku-3.5` | $1.00 | Higher quality instruction following, more expensive |
| `deepseek/deepseek-chat-v3-0324` | $0.27 | Near-frontier quality at low cost |

For classification tasks (short input, structured output), the default `google/gemini-2.5-flash` is a solid choice. Switch to a cheaper model if you process high volumes, or a more capable model if classification accuracy is critical.
