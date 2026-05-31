# Obsidian AI Agent

Telegram bot that converts text, voice, and photo messages into structured Obsidian daily notes.

- **Transcribes** voice messages using OpenAI Whisper.
- **Transcribes and Summarizes** PDF documents using Gemini Multimodal.
- **Describes and extracts EXIF metadata** from photos.
- **Classifies** the content using AI (via OpenRouter/Gemini).
- **Appends** the structured entry to the user's Obsidian Daily Note.
- **Summarizes URLs** and optionally extracts full YouTube transcripts.
- **Synchronizes** the vault using Git with automated conflict resolution.
- **Manages a Financial Portfolio**: Dedicated bot handler for position tracking and Q&A.
- **WebUI Companion**: Secure companion web portal for direct capture and real-time sync.

## Requirements

- Rust 1.85+
- Telegram Bot Token ([BotFather](https://t.me/BotFather))
- OpenRouter API Key ([openrouter.ai](https://openrouter.ai))
- OpenAI API Key ([platform.openai.com](https://platform.openai.com)) — for Whisper transcription
- yt-dlp (optional) — for YouTube full transcript extraction

## Setup

1. Copy `.env.example` to `.env` and fill in your API keys
2. Copy `config.yaml.example` to `config.yaml` and adjust settings
3. `cargo build --release`
4. `./target/release/obsidian-ai-agent`

## Photo Messages

Send photos via Telegram to automatically:
- Resize and save to your vault's assets folder
- Generate AI description and classification
- Extract EXIF metadata (if available)
- Append to daily note with Obsidian wiki link

Photos are processed through the same classification pipeline as text/voice messages, including frontmatter extraction and guide support.

## PDF Messages

Send PDF documents via Telegram to automatically:
- **Transcribe and OCR**: Full text extraction (including tables) via Gemini Multimodal.
- **AI Summary**: Concise summary of the document's content.
- **Save to Vault**: Both the original PDF and the generated transcript are saved to your vault.
- **Log in Daily Note**: Entries are automatically linked and tagged in your daily note.

> [!IMPORTANT]
> PDF transcription currently requires the native **Google Gemini** provider to be configured. See the [AI Providers](#ai-providers) section for setup details.

## URL Messages

Send any URL via Telegram to automatically create a TODO entry in your daily note with an AI-generated summary.

### Features
- **Automatic Detection**: Finds URLs within any text message.
- **Web Extraction**: Fetches page titles and content for summarization.
- **YouTube Fast Mode**: Uses metadata for instant summaries (default).
- **YouTube Transcripts**: Extract full transcripts via the "📝 Full Transcript" button or by including the "transcript" keyword in your message.
- **Transcript Storage**: Transcripts are saved as separate markdown files in your vault with wiki-links from the daily note.
- **Multiple URLs**: Process up to 5 URLs per message, each getting its own TODO entry.
- **Graceful Degradation**: If content fetching fails, the URL is still saved as a plain TODO.

**Format**:
```markdown
- [ ] [Title](url)
  > AI summary of the page content
  #tags
```

## Financial Portfolio Bot

A configurable, secondary Telegram bot/handler dedicated to financial portfolio management that interacts with specific, dedicated stock/equity markdown files (e.g. `Finance/AAPL.md`) within your Obsidian vault.

### Core Features
1. **Trade Logging & Update Positions**: Send or forward position updates (e.g., "Buy 10 AAPL @ 175", "Closed BTC position") to the bot. It automatically parses the message using structured AI classification, locates the stock's ledger file (creates it if missing), updates the YAML frontmatter (status, size, average entry, realized profit, last updated), and appends the transaction to a Markdown ledger table.
2. **Chart & Photo Attachments**: Forward trade fill confirmations or chart screenshots. The bot resizes and downloads them to a configurable vault subdirectory relative to the finance folder (e.g., `Finance/Assets/`), runs Vision AI to describe the image content, and embeds a clean wiki-link (e.g., `![[Finance/Assets/...]]`) directly inside the trade notes.
3. **Voice Transactions**: Transcribes OGG voice messages via Whisper and processes them as trade transaction updates or queries.
4. **Natural Language Portfolio Q&A**: Ask the bot questions like *"Do I have AAPL?"*, *"How large is my position?"*, or *"What are my total profits?"*. The bot reads and indexes the frontmatter and transactions across all equity notes in the Finance folder to generate clean, concise markdown answers.
5. **Configurable Prompt Guide**: Fully customize the ledger calculations, formatting rules, and Q&A tone by creating a local markdown file (e.g., `finance-system-guide.md`) and pointing `finance.guide_path` to it.

## WebUI Portal (Companion Web App)

The Obsidian AI Agent includes a secure, beautiful, real-time companion WebUI web app. It runs concurrently with the Telegram bot and acts as a premium direct capture portal.

### Features
- **Modern Glassmorphic Visuals**: Fully custom Vanilla CSS dark mode styling, frosted glass filter effects, glowing borders, slide transitions, and a clean responsive split-pane layout (note preview on the left, chat console on the right).
- **Direct Appending**: Inputs sent in the chat bypass Telegram and append instantly to your daily note, utilizing the same core classification/enrichment pipeline (Text, Voice, Photo).
- **Real-Time WebSocket Sync**: The sidebar parses and displays your Obsidian Daily Note markdown, updating in real-time instantly when you or the Telegram bot write to the vault.
- **Passcode Authentication Gateway**: Secured with a JWT-like bearer authorization system (configured via `WEBUI_AUTH_TOKEN` in your `.env` file).
- **Browser-Native Ingestions**: Supports image preview files, file-dialog attachments, and microphone recording using browser-native audio capturing APIs.

### Setup and Running E2E Tests
1. Add `WEBUI_AUTH_TOKEN=your_secure_passcode` to your `.env` file.
2. In your `config.yaml`, configure the WebUI port (defaults to 3000):
   ```yaml
   webui:
     enabled: true
     port: 3000
   ```
3. To run the comprehensive Playwright End-to-End integration test suite locally on Windows, execute:
   ```powershell
   powershell -File ./run-e2e.ps1
   ```

### First-Time Access Guide
Once the agent is running locally or in Docker:
1. **Open your browser** and navigate to: `http://localhost:3000` (or your custom configured port).
2. **Access Passcode**: You will be greeted by a secure, glassmorphic passcode authentication gateway. 
3. **Unlock**: Enter the value of the `WEBUI_AUTH_TOKEN` that you defined in your `.env` file (e.g., `your_secure_passcode`) and click **Unlock Portal**.
4. **Seamless Auto-Login**: Alternatively, you can log in instantly and bypass the passcode prompt by appending a `token` query parameter to the URL:
   ```
   http://localhost:3000/?token=your_secure_passcode
   ```
5. **Persistent Sessions**: Once successfully authenticated, your token is securely persisted in the browser's `localStorage`. You will remain logged in automatically on future visits from the same browser/device without having to re-enter your passcode!

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

## AI Providers

The Obsidian AI Agent supports multiple AI providers for classification, summarization, and OCR.

### OpenRouter (Default)
[OpenRouter](https://openrouter.ai/) provides a unified interface to hundreds of models (Gemini, Claude, GPT, etc.). It is the easiest way to get started.

- **Setup**: Set `ai.provider: openrouter` in `config.yaml` and provide `OPENROUTER_API_KEY` in `.env`.
- **Classification**: Recommended model is `google/gemini-2.0-flash-lite-001` or `google/gemini-2.5-flash`.

### Google Gemini (Native)
Using the Google Gemini API directly can be faster and cheaper for classification. **Note: Native Gemini is required for PDF transcription.**

- **Setup**: Set `ai.provider: gemini` in `config.yaml`.
- **Classification**: Recommended model is `gemini-2.0-flash` or `gemini-1.5-flash`.

#### Authentication Methods

You can authenticate with Gemini in two ways:

1. **Standard API Key**:
   - Get a key from [Google AI Studio](https://aistudio.google.com/).
   - Add `GEMINI_API_KEY=your_key` to your `.env` file.

2. **Google Cloud Service Account (OAuth 2.0)**:
   - Create a Service Account in your Google Cloud Project with "Generative Language Client" permissions.
   - Download the JSON key file.
   - Add `GEMINI_SERVICE_ACCOUNT_KEY_PATH=/path/to/key.json` to your `.env` file.
   - *Note: If neither key is provided, the agent will attempt to use Application Default Credentials (ADC).*

## Configuration

Configuration is split into two files:

- **`.env`** — API keys and secrets only
- **`config.yaml`** — All other settings (paths, models, git, timezone, etc.)

## System Guide

Create a `system-guide.md` file (next to `config.yaml`) to customize AI classification behavior with your own rules. The guide is appended to the AI prompt for every message.

Example use cases:
- Extract health metrics (weight, body fat %) as frontmatter
- Define custom trigger words for specific categories
- Set language preferences

See the included `system-guide.md` for an example.

### API Keys (`.env`)

| Variable | Required | Description |
|----------|----------|-------------|
| `TELOXIDE_TOKEN` | ✅ | Telegram Bot API token |
| `FINANCE_TELOXIDE_TOKEN` | ❌ | Telegram Bot API token for the finance bot (required only if finance bot is enabled) |
| `OPENROUTER_API_KEY` | ❌ | OpenRouter API key (required if `ai.provider: openrouter`) |
| `OPENAI_API_KEY` | ✅ | OpenAI API key (Whisper transcription) |
| `WEBUI_AUTH_TOKEN` | ❌ | Secure passcode for companion WebUI (required if WebUI is enabled) |
| `GEMINI_API_KEY` | ❌ | Google AI Studio API key (required for Gemini standard API Key auth) |
| `GEMINI_SERVICE_ACCOUNT_KEY_PATH` | ❌ | Path to a Google Cloud Service Account JSON key (for Gemini OAuth 2.0 auth) |
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
  provider: openrouter                      # AI provider: openrouter | gemini (default: openrouter)
  whisper_model: whisper-1                  # default: whisper-1
  whisper_language: nl                      # optional, ISO-639-1
  classify_model: google/gemini-2.5-flash   # default: google/gemini-2.5-flash (e.g. gemini-1.5-flash if provider is gemini)

access:
  allowed_user_ids: []                      # default: [] (allow all)

guide_path: ./system-guide.md               # default: ./system-guide.md (optional)

image:
  max_dimension: 1280                       # default: 1280
  jpeg_quality: 85                          # default: 85
  assets_folder: assets                     # default: assets

webui:
  enabled: true                             # default: true
  port: 3000                                # default: 3000

timezone: Europe/Brussels                   # default: Europe/Brussels
date_display_format: YYYY/MM/DD             # default: YYYY/MM/DD (Moment.js syntax)
log_level: info                             # default: info
```

### Settings Table (`config.yaml`)

| Field | Required | Description |
|-------|----------|-------------|
| `vault_path` | ✅ | Path to your Obsidian vault |
| `git.sync_enabled` | ❌ | Enable Git sync (default: `true`) |
| `git.path` | ❌ | Git repository root path |
| `git.remote_name` | ❌ | Git remote name (default: `origin`) |
| `git.branch` | ❌ | Git branch (default: `main`) |
| `git.ssh_key_path` | ❌ | SSH key path (default: auto-detect) |
| `git.sync_debounce_secs` | ❌ | Debounce sync timer in seconds (default: 300) |
| `ai.provider` | ❌ | AI provider selection: `openrouter` or `gemini` (default: `openrouter`) |
| `ai.whisper_model` | ❌ | Whisper model for voice transcription (default: `whisper-1`) |
| `ai.whisper_language` | ❌ | Language code for Whisper (default: auto-detect) |
| `ai.classify_model` | ❌ | Classification model name (default: `google/gemini-2.5-flash` for OpenRouter, or e.g. `gemini-1.5-flash` for Gemini) |
| `access.allowed_user_ids` | ❌ | Allowed Telegram user IDs (default: `[]` = all users) |
| `guide_path` | ❌ | Path to custom AI guide file (default: `./system-guide.md`) |
| `image.max_dimension` | ❌ | Maximum image dimension in pixels (default: 1280) |
| `image.jpeg_quality` | ❌ | JPEG compression quality 1-100 (default: 85) |
| `image.assets_folder` | ❌ | Folder name for saved images (default: `assets`) |
| `url.transcript_folder` | ❌ | Folder for YouTube transcript files (default: `transcripts`) |
| `url.fetch_timeout_secs` | ❌ | URL fetch timeout in seconds (default: 15) |
| `url.max_content_bytes` | ❌ | Maximum content size in bytes (default: 524288 = 512KB) |
| `url.max_urls_per_message` | ❌ | Maximum URLs to process per message (default: 5) |
| `finance.enabled` | ❌ | Enable secondary finance bot (default: `false`) |
| `finance.folder` | ❌ | Folder in vault for finance ledger files (default: `Finance`) |
| `finance.assets_folder` | ❌ | Subfolder relative to `finance.folder` for trade attachments (default: `Assets`) |
| `finance.guide_path` | ❌ | Custom AI guide rules for finance bot (default: none, falls back to built-in rules) |
| `finance.allowed_user_ids` | ❌ | Finance bot authorized Telegram user IDs (default: `[]` = falls back to global) |
| `timezone` | ❌ | Timezone for timestamps (default: `Europe/Brussels`) |
| `webui.enabled` | ❌ | Enable WebUI companion app (default: `true`) |
| `webui.port` | ❌ | Network port for the WebUI HTTP/WS server (default: `3000`) |
| `date_display_format` | ❌ | Moment.js format for dates (default: `YYYY/MM/DD`) |
| `log_level` | ❌ | Log level (default: `info`) |

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

These models are available via [OpenRouter](https://openrouter.ai/models) (if `ai.provider: openrouter`) or natively via Google Gemini (if `ai.provider: gemini`). Classification is a lightweight task, so fast/cheap models work well:

| Model | Input $/M tokens | Notes |
|-------|-------------------|-------|
| `google/gemini-2.5-flash` | $0.30 | **Current OpenRouter default** — good balance of speed, quality, and cost |
| `gemini-1.5-flash` / `gemini-2.5-flash` | - | **Gemini native defaults** — very fast and cost-effective |
| `google/gemini-2.0-flash-lite-001` | $0.25 | Cheaper alternative, 2.5× faster time-to-first-token |
| `anthropic/claude-haiku-3.5` | $1.00 | Higher quality instruction following, more expensive |
| `deepseek/deepseek-chat-v3-0324` | $0.27 | Near-frontier quality at low cost |

For classification tasks (short input, structured output), `gemini-1.5-flash` or `google/gemini-2.5-flash` are solid choices. Switch to a cheaper model if you process high volumes, or a more capable model if classification accuracy is critical.
