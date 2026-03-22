# Obsidian AI Agent

Telegram bot that converts text and voice messages into structured Obsidian daily notes.

## Requirements

- Rust 1.85+
- Telegram Bot Token ([BotFather](https://t.me/BotFather))
- OpenRouter API Key ([openrouter.ai](https://openrouter.ai))
- OpenAI API Key ([platform.openai.com](https://platform.openai.com)) — for Whisper transcription

## Setup

1. Copy `.env.example` to `.env` and fill in your API keys
2. Copy `config.yaml.example` to `config.yaml` and adjust settings
3. `cargo build --release`
4. `./target/release/obsidian-ai-agent`

## Docker

Pre-built images are available on [Docker Hub](https://hub.docker.com/r/peterluxem/obsidian-ai-agent):

```bash
cp .env.docker.example .env.docker                # Fill in API keys
cp config.docker.yaml.example config.docker.yaml   # Adjust settings
docker compose up -d
```

Images are automatically built and pushed on every commit to `main`.

To build locally instead, uncomment `build: .` in `docker-compose.yaml`.

### Docker Environment Variables

These variables are set in `docker-compose.yaml` or passed via `environment` / shell:

| Variable | Default | Description |
|----------|---------|-------------|
| `HOST_VAULT_PATH` | `./.vault` | Host path to your Obsidian vault |
| `HOST_CONFIG_PATH` | `./config.docker.yaml` | Host path to `config.yaml` |
| `HOST_SSH_DIR` | `~/.ssh` | Host SSH directory (mounted read-only) |
| `TZ` | `Europe/Brussels` | Container timezone |
| `USER_ID` | Auto-detected from vault | UID for the container user |
| `GROUP_ID` | Auto-detected from vault | GID for the container user |
| `GIT_USER_NAME` | `Obsidian AI Agent` | Git committer name |
| `GIT_USER_EMAIL` | `bot@obsidian-ai-agent` | Git committer email |

### CI/CD Setup (GitHub Actions)

The workflow at `.github/workflows/docker.yml` builds and pushes to Docker Hub. Add these secrets to your GitHub repo (`Settings > Secrets > Actions`):

| Secret | Value |
|--------|-------|
| `DOCKERHUB_USERNAME` | Your Docker Hub username |
| `DOCKERHUB_TOKEN` | Docker Hub access token ([create one here](https://hub.docker.com/settings/security)) |

## Configuration

Configuration is split into two files:

- **`.env`** — API keys and secrets only
- **`config.yaml`** — All other settings (paths, models, git, timezone, etc.)

### API Keys (`.env`)

| Variable | Required | Description |
|----------|----------|-------------|
| `TELOXIDE_TOKEN` | ✅ | Telegram Bot API token |
| `OPENROUTER_API_KEY` | ✅ | OpenRouter API key (classification) |
| `OPENAI_API_KEY` | ✅ | OpenAI API key (Whisper transcription) |
| `CONFIG_PATH` | ❌ | Path to config file (default: `./config.yaml`) |

### Settings (`config.yaml`)

```yaml
vault_path: /path/to/your/obsidian/vault

git:
  sync_enabled: true                        # default: true
  path: /path/to/your/git-root-path         # required when sync_enabled is true
  remote_name: origin                       # default: origin
  branch: main                              # default: main
  ssh_key_path:                             # default: auto-detect
  sync_debounce_secs: 300                   # default: 300

ai:
  whisper_model: whisper-1                  # default: whisper-1
  whisper_language: nl                      # optional, ISO-639-1
  classify_model: google/gemini-2.5-flash   # default: google/gemini-2.5-flash

access:
  allowed_user_ids: []                      # default: [] (allow all)

timezone: Europe/Brussels                   # default: Europe/Brussels
log_level: info                             # default: info
```

## Model Recommendations

### Transcription (`ai.whisper_model`)

These models are available via the OpenAI `/v1/audio/transcriptions` endpoint:

| Model | Quality | Speed | Cost | Notes |
|-------|---------|-------|------|-------|
| `gpt-4o-mini-transcribe` | Near-best accuracy | Fast | ~$0.003/min | **Recommended default** — cheaper and more accurate than `whisper-1` |
| `gpt-4o-transcribe` | Best accuracy | Slower | ~$0.006/min | Best choice when accuracy matters most |
| `whisper-1` | Good baseline | Fast | ~$0.006/min | Legacy model, can misdetect language (e.g. Dutch → German) |

**Tip:** If you primarily send voice messages in a single language, set `ai.whisper_language` to avoid misdetection (e.g. `nl` for Dutch, `en` for English).

### Classification (`ai.classify_model`)

These models are available via [OpenRouter](https://openrouter.ai/models). Classification is a lightweight task, so fast/cheap models work well:

| Model | Input $/M tokens | Notes |
|-------|-------------------|-------|
| `google/gemini-2.5-flash` | $0.30 | **Current default** — good balance of speed, quality, and cost |
| `google/gemini-2.0-flash-lite-001` | $0.25 | Cheaper alternative, 2.5× faster time-to-first-token |
| `anthropic/claude-haiku-3.5` | $1.00 | Higher quality instruction following, more expensive |
| `deepseek/deepseek-chat-v3-0324` | $0.27 | Near-frontier quality at low cost |

For classification tasks (short input, structured output), the default `google/gemini-2.5-flash` is a solid choice. Switch to a cheaper model if you process high volumes, or a more capable model if classification accuracy is critical.
