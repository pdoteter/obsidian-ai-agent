## Why

The current implementation is tightly coupled to the OpenRouter and OpenAI Whisper APIs. This creates a single point of failure and prevents the use of alternative AI providers (like local LLMs via Ollama, or direct Anthropic/Google API usage) or local transcription services. Modularizing the AI layer is the first step towards a more flexible and resilient architecture.

## What Changes

- **AI Abstraction Layer**: Introduction of an `AiProvider` trait that defines standard methods for classification, summarization, and transcription.
- **Provider Adapters**: Refactoring the existing `OpenRouterClient` and `WhisperClient` into adapters that implement the `AiProvider` trait.
- **Config Refactoring**: Updating `config.yaml` to support selectable AI providers for different tasks.
- **Handler Decoupling**: Handlers will now interact with the `AiProvider` trait instead of concrete API client implementations.

## Capabilities

### New Capabilities
- `ai-provider-abstraction`: Defines the contract and selection logic for different AI backends.

### Modified Capabilities
- (None - this refactor changes the internal architecture but maintains existing behavioral requirements for Obsidian integration.)

## Impact

- `src/ai/`: Major refactoring to introduce the trait and separate client logic into adapters.
- `src/handlers/`: Updated to take `dyn AiProvider` instead of concrete clients.
- `src/config.rs`: Updated to support multi-provider configuration schemas.
- `src/main.rs`: Updated for the new initialization flow.
