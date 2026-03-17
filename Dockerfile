# ============================================
# Stage 1: Build
# ============================================
FROM rust:1.85-bookworm AS builder

# Install build dependencies for libgit2 and openssl
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libgit2-dev \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies by copying only Cargo files first
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true

# Now copy the real source code
COPY src/ src/

# Force rebuild of our code (touch to invalidate cache)
RUN touch src/main.rs
RUN cargo build --release

# ============================================
# Stage 2: Runtime
# ============================================
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ffmpeg \
    libssl3 \
    libgit2-1.5 \
    ca-certificates \
    openssh-client \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r botuser && useradd -r -g botuser -m botuser

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/obsidian-ai-agent /app/obsidian-ai-agent

# Create vault mount point
RUN mkdir -p /app/vault && chown -R botuser:botuser /app

# Default environment variables
ENV VAULT_PATH=/app/vault
ENV RUST_LOG=info
ENV GIT_SYNC_DEBOUNCE_SECS=300

# Entrypoint script for UID/GID mapping
COPY docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["/app/obsidian-ai-agent"]
