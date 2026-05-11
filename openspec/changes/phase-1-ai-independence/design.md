## Context

The current `src/ai/client.rs` and `src/ai/transcribe.rs` contain hardcoded logic for OpenRouter and OpenAI respectively. Handlers in `src/handlers/` directly depend on these concrete types. This makes testing difficult and inhibits the addition of new AI backends.

## Goals / Non-Goals

**Goals:**
- Define a unified `AiProvider` trait for all AI-driven capabilities.
- Implement an `AiService` wrapper that handles provider selection and error mapping.
- Refactor existing handlers to use the `AiService` or `dyn AiProvider`.
- Update the configuration to allow per-task provider selection (e.g., Use Gemini for classification, but local Whisper for transcription).

**Non-Goals:**
- Adding NEW AI providers in this phase (this phase only refactors existing ones into the new architecture).
- Modifying the Obsidian vault logic.
- Adding multiple client support (Phase 3/4).

## Decisions

### Decision: Trait-based Abstraction
We will use a central trait `AiProvider` to define capabilities.
- **Rationale**: Traits are the idiomatic way in Rust to achieve polymorphism.
- **Alternatives**: Enum-based dispatch. While slightly faster, it requires modifying the enum every time a new provider is added, violating the Open/Closed principle.

### Decision: `AiService` vs direct Trait usage in Handlers
Handlers will receive an `Arc<AiService>` which internally holds the configured providers.
- **Rationale**: This centralizes the logic for "which provider do I use for X?" and allows for unified logging/metrics.

### Decision: Error Standardization
A new `AiError` enum in `src/error.rs` will be refined to act as a "Babel fish" for different provider errors.
- **Rationale**: Handlers should not need to know if a rate limit came from OpenRouter or a local service.

## Risks / Trade-offs

- **Risk**: Increased complexity in `Config` management.
  - **Mitigation**: Use sensible defaults in `config.yaml` so existing users don't need to change anything immediately.
- **Risk**: Performance overhead of dynamic dispatch.
  - **Mitigation**: The overhead of `dyn Trait` is negligible compared to the network latency of AI API calls.
