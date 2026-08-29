# Video Processing & Multimodal Investigation

This document provides a deep architectural and feature investigation into handling video messages, round video notes, and animation media within the **Obsidian AI Agent**.

---

## 1. Video Input Types in Telegram & WebUI

When a user interacts with the bot, videos can arrive via several distinct channels:

| Source | Identifier / Method | Typical Characteristics | Available Metadata |
| :--- | :--- | :--- | :--- |
| **Telegram Video** | `msg.video()` | Standard video file captured or shared from the phone camera/gallery (`.mp4`, `.mov`) | Duration, width, height, mime type, file size, built-in thumbnail (`PhotoSize`), user caption |
| **Telegram Video Note** | `msg.video_note()` | Quick round bubble "telescope" video recorded directly in chat (up to 60 seconds) | Duration, dimensions (1:1 aspect ratio), file size, built-in thumbnail |
| **Telegram Animation** | `msg.animation()` | Silent loops, screen captures, or GIFs (`.mp4` / `.gif`) | Duration, width, height, file size, thumbnail, user caption |
| **Video Document** | `msg.document()` | Uncompressed video files (`.mkv`, `.avi`, `.webm`, `.mp4`) | File name, mime type, size, user caption |
| **WebUI Portal** | `/api/video` or file upload | Browser-recorded camera snippets or dropped video files | Mime type, file name, user caption |

---

## 2. Video Processing Pipelines & Architectural Strategies

```
                      ┌──────────────────────────────────────┐
                      │           Incoming Video             │
                      │  (Telegram Video, VideoNote, WebUI)  │
                      └──────────────────┬───────────────────┘
                                         │
                   ┌─────────────────────┼─────────────────────┐
                   ▼                     ▼                     ▼
          [ Strategy 1 ]          [ Strategy 2 ]        [ Strategy 3 ]
       Full Multimodal AI       Whisper Audio Only    Thumbnail + Metadata
       (Gemini 2.5 / 1.5)       (Audio Track Speech)   (Fallback / Offline)
                   │                     │                     │
                   │ • Visual OCR        │ • Fast STT speech   │ • Extract frame
                   │ • Scene breakdown   │ • Text classifier   │ • EXIF / duration
                   │ • Audio dialogue    │   post-processing   │ • As-is reference
                   │ • Timestamps        │                     │
                   └─────────────────────┼─────────────────────┘
                                         │
                                         ▼
                      ┌──────────────────────────────────────┐
                      │          Obsidian Output             │
                      │  1. Save Video to `assets/`          │
                      │  2. Save Markdown Transcript         │
                      │  3. Embed Player & Log in Daily Note │
                      └──────────────────────────────────────┘
```

### Strategy 1: Full Multimodal Video Understanding (Google Gemini)
*Google Gemini natively supports audio and video inputs (`video/mp4`, `video/webm`, `video/quicktime`, `video/x-matroska`).*

- **Visual Analysis**: Gemini inspects visual frames across the video timeline to perform OCR on screens/whiteboards, recognize physical objects, identify locations, analyze charts, and describe actions.
- **Audio Analysis**: Gemini simultaneously listens to the audio track to transcribe spoken dialogue, identify speaker tone/intent, and correlate speech directly with on-screen visual context.
- **Generated Outputs**:
  1. **Concise Summary**: 1–3 sentence abstract for daily overview.
  2. **Timestamped Scene Breakdown**: Chapter-style timestamps (e.g., `00:00 - Introduction`, `00:15 - Code walkthrough`, `00:45 - Key conclusion`).
  3. **Searchable Markdown Transcript**: Generated as a standalone `.md` sidecar transcript file saved in the vault assets folder (matching the workflow used for YouTube transcripts and PDF OCR).
  4. **Categorization & Frontmatter**: Automatic classification into `## ✅ Todos`, `## 📋 Log`, or `## 📝 Notes`.

### Strategy 2: Audio Track Transcription (OpenAI Whisper)
*Focused on speech-heavy videos such as Telegram Video Notes.*

- Extracts or streams the audio channel directly into OpenAI Whisper for speech-to-text transcription.
- Passes the transcribed text through the standard text classification engine.
- Ideal when the user records a talking-head video message or when visual inference is not needed.

### Strategy 3: Lightweight Thumbnail & Metadata Storage (Offline / Fast Fallback)
*Used when AI providers are unavailable or offline.*

- Saves the video file directly into `<vault>/assets/`.
- Extracts and saves Telegram's built-in thumbnail (`video.thumbnail`) as a poster image.
- Logs video metadata (duration, dimensions, file size, timestamp, caption) into `## 📋 Log`.

---

## 3. Obsidian Vault Integration & Rendering

Obsidian natively supports embedded HTML5 video playback in Markdown notes:

### 1. Native Wikilink Video Embedding
```markdown
![[assets/2026-08-29-meeting-demo-a1b2.mp4]]
```

### 2. Structured Daily Note Entry with Sidecar Transcript Link
```markdown
- 16:30 — 🎥 **Video: Product Demonstration**
  - **Video**: ![[assets/2026-08-29-product-demo-a1b2.mp4]]
  - **Transcript**: [[assets/2026-08-29-product-demo-a1b2.md]]
  - **Summary**:
    > Walkthrough of the newly implemented search UI, keyboard shortcuts, and responsiveness improvements.
  - **Key Moments**:
    - `00:00` — Overview of new layout
    - `00:20` — Demonstration of shortcut triggers
    - `00:45` — Discussion on mobile responsiveness
  #work #demo #ui
```

---

## 4. Technical Constraints & Design Considerations

| Constraint / Factor | Details | Proposed Handling |
| :--- | :--- | :--- |
| **Telegram Bot API Limit** | Standard Telegram Bot API limits file downloads to **20 MB** (or 50 MB with a Local Bot API Server). | Files under 20 MB (which includes almost all round video notes and short camera clips) are downloaded and processed. Oversized files trigger a helpful notification. |
| **Gemini Inline Base64 Limit** | Gemini inline data accepts payloads up to **20 MB**. | Fits within the Telegram download limit. For larger files, Gemini's Resumable Files API (`POST /upload/v1beta/files`) can be used. |
| **Vault Disk & Git Sync** | Storing large video files in the vault may increase Git repository size. | Consider max video size settings, Git sync debouncing, or Git LFS if large video archives are needed. |
| **Processing Latency** | Multimodal video analysis typically requires 3–10 seconds. | Send `ChatAction::UploadVideo` / `ChatAction::Typing` indicators in Telegram while processing. |
