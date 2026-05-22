#!/bin/bash
set -e

echo "========================================================================"
echo "               Obsidian AI Agent Container Startup                      "
echo "========================================================================"
echo "[Startup] Date/Time: $(date)"
echo "[Startup] Current shell user: $(whoami) (UID: $(id -u), GID: $(id -g))"

# Match botuser UID/GID to the vault mount owner so writes don't get PermissionDenied.
# Uses USER_ID/GROUP_ID env vars if set, otherwise auto-detects from /app/vault.
VAULT_UID="${USER_ID:-$(stat -c '%u' /app/vault 2>/dev/null || echo '')}"
VAULT_GID="${GROUP_ID:-$(stat -c '%g' /app/vault 2>/dev/null || echo '')}"

if [ -n "$VAULT_UID" ] && [ -n "$VAULT_GID" ] && [ "$VAULT_UID" != "0" ]; then
    echo "[Perms] Matching botuser UID/GID to host vault directory: ${VAULT_UID}:${VAULT_GID}"
    groupmod -g "$VAULT_GID" botuser 2>/dev/null || true
    usermod -u "$VAULT_UID" -g "$VAULT_GID" botuser 2>/dev/null || true
    # chown files we own; skip read-only mounts (e.g. config.yaml mounted :ro)
    chown botuser:botuser /app /app/obsidian-ai-agent 2>/dev/null || true
    chown -R botuser:botuser /app/vault 2>/dev/null || true
else
    echo "[Perms] Using default container user permissions for botuser"
fi

# Setup SSH keys: copy from read-only mount to writable home dir
if [ -d "/tmp/.ssh-host" ]; then
    echo "[SSH] SSH keys detected at /tmp/.ssh-host. Copying to home directory..."
    mkdir -p /home/botuser/.ssh
    cp -r /tmp/.ssh-host/* /home/botuser/.ssh/ 2>/dev/null || true
    chown -R botuser:botuser /home/botuser/.ssh
    chmod 700 /home/botuser/.ssh
    chmod 600 /home/botuser/.ssh/* 2>/dev/null || true
    echo "[SSH] SSH keys configured: $(ls -A /home/botuser/.ssh | tr '\n' ' ')"

    # Configure SSH to use port 443 for GitHub (bypasses firewalls blocking port 22)
    if [ ! -f /home/botuser/.ssh/config ]; then
        echo "[SSH] Creating custom ~/.ssh/config to force port 443 for ssh.github.com..."
        cat > /home/botuser/.ssh/config <<'SSHEOF'
Host github.com
    Hostname ssh.github.com
    Port 443
    User git
SSHEOF
        chmod 600 /home/botuser/.ssh/config
        chown botuser:botuser /home/botuser/.ssh/config
    fi

    # Add common git hosts to known_hosts if not present
    if [ ! -f /home/botuser/.ssh/known_hosts ]; then
        echo "[SSH] Populating ~/.ssh/known_hosts with github/gitlab keys..."
        ssh-keyscan -t ed25519 -p 443 ssh.github.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        ssh-keyscan -t ed25519 github.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        ssh-keyscan -t ed25519 gitlab.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        chown botuser:botuser /home/botuser/.ssh/known_hosts
    fi
else
    echo "[SSH] No SSH keys mounted at /tmp/.ssh-host — automated git push over SSH will be disabled"
fi

# Configure git identity and safe directory for botuser
# --author in `git commit` sets the author, but git still requires a committer identity
echo "[Git] Initializing Git configuration for botuser..."
su -s /bin/sh botuser -c "
    git config --global user.name '${GIT_USER_NAME:-Obsidian AI Agent}'
    git config --global user.email '${GIT_USER_EMAIL:-bot@obsidian-ai-agent}'
    git config --global --add safe.directory /app/vault
"

# Auto-convert HTTPS remote URLs to SSH when SSH keys are available
# Fixes: "could not read Username for 'https://github.com'" in non-interactive containers
# Converts both fetch and push URLs (git remotes can have separate push URLs)
if [ -d "/tmp/.ssh-host" ] && [ -d "/app/vault/.git" ]; then
    for URL_FLAG in "" "--push"; do
        REMOTE_URL=$(git -C /app/vault remote get-url $URL_FLAG origin 2>/dev/null || true)
        if echo "$REMOTE_URL" | grep -q '^https://github\.com/'; then
            SSH_URL=$(echo "$REMOTE_URL" | sed 's|^https://github\.com/|git@github.com:|')
            echo "[Git] Converting HTTPS remote ${URL_FLAG:-fetch} URL to SSH: $SSH_URL"
            git -C /app/vault remote set-url $URL_FLAG origin "$SSH_URL"
        fi
    done
fi

# Ensure vault path directory exists inside container
if [ -f "/app/config.yaml" ]; then
    VAULT_PATH_CONFIG=$(grep -E '^\s*vault_path:' /app/config.yaml | head -n1 | sed -E "s/^\s*vault_path:\s*['\"]?([^'\"]+)['\"]?/\1/")
    if [ -n "$VAULT_PATH_CONFIG" ]; then
        echo "[Vault] Parsed vault_path from config.yaml: $VAULT_PATH_CONFIG"
        if [ ! -d "$VAULT_PATH_CONFIG" ]; then
            echo "[Vault] Vault path '$VAULT_PATH_CONFIG' does not exist. Creating it now..."
            mkdir -p "$VAULT_PATH_CONFIG"
            chown -R botuser:botuser "$VAULT_PATH_CONFIG" 2>/dev/null || true
            echo "[Vault] Successfully created vault path."
        else
            echo "[Vault] Vault path '$VAULT_PATH_CONFIG' already exists."
        fi
    else
        echo "[Vault] WARNING: Could not parse vault_path from config.yaml — skipping auto-creation"
    fi
else
    echo "[Vault] WARNING: No config.yaml found at /app/config.yaml — skipping auto-creation"
fi

# Run the primary command
echo "[Startup] Starting Obsidian AI Agent with command: $*"
echo "------------------------------------------------------------------------"
if su -s /bin/sh botuser -c "true" 2>/dev/null; then
    echo "[Startup] Executing as user 'botuser' (non-root)..."
    exec su -s /bin/sh botuser -c "exec $*"
else
    echo "[Startup] WARNING: su as botuser failed, falling back to root execution..."
    exec "$@"
fi

