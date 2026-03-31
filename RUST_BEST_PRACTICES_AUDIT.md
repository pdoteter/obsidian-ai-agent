# Rust Best Practices Audit — obsidian-ai-agent

**Date:** 2026-03-31  
**Scope:** Full codebase (36 `.rs` files across 8 modules)  
**Project:** Telegram bot that saves messages to an Obsidian vault (Rust, tokio, teloxide)

---

## Executive Summary

The codebase demonstrates solid Rust fundamentals — proper `thiserror`-based error hierarchy, clean module separation, idiomatic async patterns, and thorough test coverage. However, the custom error system (`AppError`/`AppResult`) is defined but never actually used, leaving handlers with verbose `Box<dyn Error>` boilerplate. There is also a confirmed double-message bug in the text handler and several P2/P3 improvements around performance and ergonomics.

---

## Strengths

### ✅ Error Type Design
- Excellent `thiserror`-based hierarchy: 6 domain-specific error enums (`AiError`, `ConfigError`, `GitError`, `ImageError`, `UrlError`, `VaultError`) plus a top-level `AppError` with `#[from]` conversions.
- Each domain error has meaningful, descriptive variants.

### ✅ Module Organization
- Clean domain separation: `ai/`, `handlers/`, `git/`, `vault/`, `image/`, `url/`, `audio/`.
- Each module has a focused responsibility with clear boundaries.

### ✅ Async Patterns
- Proper use of `tokio::select!`, `tokio::join!`, and `tokio::task::spawn_blocking` for CPU-bound work (image processing).
- Debounced git sync with cancellation via `AtomicBool`.

### ✅ Testing
- 100+ tests with edge cases, Unicode handling, and boundary conditions.
- Tests cover URL detection, image processing, config parsing, vault operations, and more.

### ✅ Configuration
- Clean separation: secrets in `.env`, settings in `config.yaml`.
- `FileConfig` (raw YAML) → `Config` (validated, typed) transformation pattern.

### ✅ Concurrency
- Appropriate use of `Arc`, `Mutex`, `AtomicBool`, and channels.
- No raw unsafe blocks.

---

## Issues Found

### P1 — Critical / High Impact

#### 1. `AppError`/`AppResult` defined but never used
**Files:** `src/error.rs`, all `src/handlers/*.rs`  
**Problem:** The carefully designed error hierarchy exists but handlers return `Box<dyn std::error::Error>` instead. This defeats the purpose of typed errors and loses compile-time exhaustiveness checking.  
**Fix:** Replace `Box<dyn Error>` return types with `AppResult<T>` across all handlers. The `#[from]` conversions already exist — they just aren't being used.

#### 2. Verbose `Box<dyn Error>` casting boilerplate
**Files:** `src/handlers/photo.rs`, `src/handlers/voice.rs`, `src/handlers/url.rs`  
**Problem:** Handlers manually cast errors via `.map_err(|e| Box::new(e) as Box<dyn Error>)` throughout. This is exactly the boilerplate that `thiserror` + `?` operator eliminates.  
**Fix:** Adopting `AppResult` (issue #1) eliminates all of this boilerplate automatically.

#### 3. `env::set_var` unsafe in multi-threaded context
**File:** `src/config.rs:223-228`  
**Problem:** `std::env::set_var` is called in tests. As of Rust 1.66+, this is documented as unsound in multi-threaded programs. Since `cargo test` runs tests in parallel threads, this creates a race condition.  
**Fix:** Use `temp_env` crate or `serial_test` to isolate environment variable mutations, or refactor config loading to accept env vars as parameters instead of reading `std::env` directly.

#### 4. Double confirmation message bug
**File:** `src/handlers/text.rs:117-145`  
**Problem:** The text handler always sends two confirmation messages: one inside the `if config.git.enabled` block and one unconditionally after it. Every text message triggers two bot replies.  
**Fix:** Make the second confirmation conditional (in an `else` branch) or remove the duplicate.

---

### P2 — Moderate Impact

#### 5. Auth check after URL detection in text handler
**File:** `src/handlers/text.rs:32-61`  
**Problem:** The handler detects URLs and does work before checking if the user is authorized. Unauthorized users trigger unnecessary processing.  
**Fix:** Move the authorization check to the top of the handler, before any URL detection logic.

#### 6. YouTube regex recompiled on every call
**File:** `src/url/detect.rs:29-36`  
**Problem:** `detect_urls()` compiles the YouTube regex pattern on every invocation instead of using `lazy_static!` or `std::sync::OnceLock`.  
**Fix:** Use `std::sync::LazyLock` (stable since Rust 1.80) or `once_cell::sync::Lazy` to compile the regex once.

#### 7. `sanitize_slug` O(n²) double-hyphen collapse
**File:** `src/image/process.rs:76-78`  
**Problem:** Uses a `while slug.contains("--") { slug = slug.replace("--", "-"); }` loop which is O(n²) for pathological inputs (e.g., long strings of hyphens).  
**Fix:** Use a single regex replacement `Regex::new(r"-{2,}")` → `"-"`, or a single-pass char iterator.

#### 8. `ChatIdTracker` should use `watch` channel
**File:** `src/git/chat_tracker.rs`  
**Problem:** Uses `Arc<Mutex<Option<ChatId>>>` for a single-value that's written occasionally and read frequently. This is a classic `tokio::sync::watch` use case — cheaper reads, no lock contention.  
**Fix:** Replace with `tokio::sync::watch::channel`.

#### 9. Git sync uses blocking `Command`
**File:** `src/git/sync.rs`  
**Problem:** Uses `std::process::Command` (blocking) for git operations instead of `tokio::process::Command`. This blocks the tokio runtime thread during git operations.  
**Fix:** Either switch to `tokio::process::Command` or wrap calls in `spawn_blocking`.

---

### P3 — Low Impact / Ergonomics

#### 10. `#[allow(clippy::too_many_arguments)]` — needs `HandlerContext` struct
**Files:** `src/main.rs`, all handler functions  
**Problem:** Handler functions take 7+ arguments (bot, message, config, vault_path, tracker, etc.). The clippy suppression masks a real readability issue.  
**Fix:** Create a `HandlerContext` struct bundling shared state, pass it to all handlers.

#### 11. `#[allow(dead_code)]` on error types
**File:** `src/error.rs`  
**Problem:** Some error variants are marked `#[allow(dead_code)]`. These may be forward-looking or genuinely unused.  
**Fix:** Periodic review — remove truly unused variants, remove the allow attribute from used ones.

#### 12. Byte length vs char length in truncation
**File:** `src/handlers/url.rs:527`  
**Problem:** `truncate_confirmation_if_needed` checks `message.len()` (byte length) against a character limit. For ASCII this works, but for Unicode content (common in a multilingual Telegram bot), byte length ≠ character count.  
**Fix:** Use `.chars().count()` for the length check, and truncate at a char boundary.

#### 13. Missing `Default` impl for `ChatIdTracker`
**File:** `src/git/chat_tracker.rs`  
**Problem:** `ChatIdTracker::new()` just wraps `Arc::new(Mutex::new(None))`. This is a natural `Default` implementation.  
**Fix:** `#[derive(Default)]` or manual `impl Default`.

#### 14. Hardcoded JPEG quality ignores config
**File:** `src/image/process.rs:43`  
**Problem:** JPEG quality is hardcoded to `85` despite `config.image.jpeg_quality` existing in the config struct.  
**Fix:** Pass the config value through to the image processing function.

---

## Top 3 Recommended Improvements

| Priority | Change | Impact |
|----------|--------|--------|
| 1 | **Adopt `AppError`/`AppResult`** across all handlers | Eliminates `Box<dyn Error>` boilerplate, enables typed error matching, leverages existing `#[from]` conversions |
| 2 | **Fix double-message bug** in `handlers/text.rs` | User-facing bug — every text message sends two confirmations |
| 3 | **Create `HandlerContext` struct** | Reduces argument count, improves readability, removes clippy suppressions |

---

## Project Metadata

| Property | Value |
|----------|-------|
| Rust Edition | 2021 |
| Async Runtime | tokio 1 |
| Telegram Framework | teloxide 0.13 |
| HTTP Client | reqwest 0.12 |
| Error Library | thiserror 2 |
| Serialization | serde / serde_json |
| Image Processing | image 0.25 |
| Source Files | 36 `.rs` files |
| Test Count | 100+ |
