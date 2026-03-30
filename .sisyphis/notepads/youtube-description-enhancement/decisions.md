# Decisions

## [2026-03-26] Architecture Decisions

### Decision: Use Raw oEmbed Title for Heading
**Context**: User wants `### Video Name` heading above YouTube todos.
**Decision**: Use raw oEmbed title (not AI-reformulated) for the `### ` heading because it's the authentic video name.
**Rationale**: AI title is for link text; raw title preserves original video name.

### Decision: Parallel Description Fetch
**Context**: Avoid adding latency to YouTube URL processing.
**Decision**: Run yt-dlp description fetch in parallel with oEmbed using `tokio::join!`.
**Rationale**: Description fetch is best-effort; shouldn't block the fast path.

### Decision: Silent Fallback on yt-dlp Failure
**Context**: yt-dlp might not be installed or description fetch might fail.
**Decision**: Silently fall back to current oEmbed-only behavior. Still show heading with oEmbed title.
**Rationale**: Best-effort enhancement; user experience shouldn't degrade if yt-dlp is unavailable.

### Decision: No Retry Logic
**Context**: yt-dlp call might fail due to network, video unavailable, etc.
**Decision**: Single attempt with timeout, no retry.
**Rationale**: Not requested by user, adds complexity without clear benefit.

### Decision: Reuse Existing Timeout Config
**Context**: Need timeout for yt-dlp subprocess.
**Decision**: Use `config.url.fetch_timeout_secs` (same as HTTP fetch timeout).
**Rationale**: No new config keys; reasonable default for external command execution.

### Decision: Heading in Transcript Flow Too
**Context**: Transcript button callback also creates YouTube todos.
**Decision**: Add `### Video Name` heading to transcript callback flow for consistency.
**Rationale**: User expects consistent formatting across all YouTube URL entry points.
