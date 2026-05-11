## ADDED Requirements

### Requirement: Pluggable AI Backends
The system SHALL support multiple AI backends for classification, summarization, and transcription tasks. These backends MUST implement a common interface to ensure implementation-agnostic behavior in higher-level handlers.

#### Scenario: Switching providers via config
- **WHEN** the `config.yaml` is updated to use a different AI provider (e.g., from `openrouter` to `openai`)
- **THEN** the system SHALL initialize and use the newly specified provider for all subsequent AI tasks without code changes.

### Requirement: Graceful Provider Degradation
If a primary AI provider fails (e.g., due to API downtime or rate limits), the system SHALL either fallback to a secondary provider (if configured) or return a standardized error that handlers can use to trigger local fallback logic (like saving as a raw log).

#### Scenario: Provider failure during classification
- **WHEN** the active AI provider returns a server error or rate limit during classification
- **THEN** the system SHALL propagate a standardized `AiError` that triggers the handler's "Save as Raw Log" fallback path.

### Requirement: Standardized Transcription Interface
The transcription service SHALL accept raw audio bytes and return a text transcript, hiding details about specific API endpoints or models from the voice handler.

#### Scenario: Transcribing voice notes
- **WHEN** Ogg Opus bytes are passed to the transcription interface
- **THEN** the interface SHALL return the full text transcript as a string or a specific transcription error.
