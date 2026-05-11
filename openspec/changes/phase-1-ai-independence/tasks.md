## 1. Foundation

- [x] 1.1 Define the `AiProvider` trait in `src/ai/mod.rs`
- [x] 1.2 Refine `AiError` in `src/error.rs` to support general provider errors
- [x] 1.3 Update `Config` struct in `src/config.rs` to support the new provider-selection schema

## 2. Refactor Existing Clients

- [x] 2.1 Refactor `OpenRouterClient` into `src/ai/providers/openrouter.rs` as an `AiProvider` implementation
- [x] 2.2 Refactor `WhisperClient` into `src/ai/providers/openai_whisper.rs` as an `AiProvider` implementation
- [x] 2.3 Implement the `AiService` orchestrator in `src/ai/mod.rs`

## 3. Handler Integration

- [x] 3.1 Update `src/main.rs` to initialize the `AiService` and providers
- [ ] 3.2 Refactor `handle_text_message` in `src/handlers/text.rs` to use `AiService`
- [ ] 3.3 Refactor `handle_voice_message` in `src/handlers/voice.rs` to use `AiService`
- [ ] 3.4 Refactor `handle_url_message` and `handle_transcript_callback` in `src/handlers/url.rs` to use `AiService`
- [ ] 3.5 Refactor photo handlers in `src/handlers/photo.rs`

## 4. Verification

- [ ] 4.1 Run `cargo test` to ensure existing logic is preserved
- [ ] 4.2 Add unit tests for `AiService` provider selection logic
- [ ] 4.3 Verify provider switching works via `config.yaml` using a mock or stub provider
