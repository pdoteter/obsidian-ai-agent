#!/bin/bash
set -e

# Match botuser UID/GID to the vault mount owner so writes don't get PermissionDenied.
# Uses USER_ID/GROUP_ID env vars if set, otherwise auto-detects from /app/vault.
VAULT_UID="${USER_ID:-$(stat -c '%u' /app/vault 2>/dev/null || echo '')}"
VAULT_GID="${GROUP_ID:-$(stat -c '%g' /app/vault 2>/dev/null || echo '')}"

if [ -n "$VAULT_UID" ] && [ -n "$VAULT_GID" ] && [ "$VAULT_UID" != "0" ]; then
    echo "Adjusting botuser UID/GID to ${VAULT_UID}:${VAULT_GID}"
    groupmod -g "$VAULT_GID" botuser 2>/dev/null || true
    usermod -u "$VAULT_UID" -g "$VAULT_GID" botuser 2>/dev/null || true
    # chown files we own; skip read-only mounts (e.g. config.yaml mounted :ro)
    chown botuser:botuser /app /app/obsidian-ai-agent 2>/dev/null || true
    chown -R botuser:botuser /app/vault 2>/dev/null || true
fi

# Setup SSH keys: copy from read-only mount to writable home dir
if [ -d "/tmp/.ssh-host" ]; then
    echo "Copying SSH keys from /tmp/.ssh-host to /home/botuser/.ssh"
    mkdir -p /home/botuser/.ssh
    cp -r /tmp/.ssh-host/* /home/botuser/.ssh/ 2>/dev/null || true
    chown -R botuser:botuser /home/botuser/.ssh
    chmod 700 /home/botuser/.ssh
    chmod 600 /home/botuser/.ssh/* 2>/dev/null || true

    # Configure SSH to use port 443 for GitHub (bypasses firewalls blocking port 22)
    if [ ! -f /home/botuser/.ssh/config ]; then
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
        ssh-keyscan -t ed25519 -p 443 ssh.github.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        ssh-keyscan -t ed25519 github.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        ssh-keyscan -t ed25519 gitlab.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        chown botuser:botuser /home/botuser/.ssh/known_hosts
    fi
else
    echo "No SSH keys mounted at /tmp/.ssh-host — git push over SSH will not work"
fi

# Mark vault as safe directory for botuser (ownership differs between mount and botuser)
# Must use botuser's HOME so the config is visible when running as botuser
su -s /bin/sh botuser -c "git config --global --add safe.directory /app/vault"

# Auto-convert HTTPS remote URLs to SSH when SSH keys are available
# Fixes: "could not read Username for 'https://github.com'" in non-interactive containers
# Converts both fetch and push URLs (git remotes can have separate push URLs)
if [ -d "/tmp/.ssh-host" ] && [ -d "/app/vault/.git" ]; then
    for URL_FLAG in "" "--push"; do
        REMOTE_URL=$(git -C /app/vault remote get-url $URL_FLAG origin 2>/dev/null || true)
        if echo "$REMOTE_URL" | grep -q '^https://github\.com/'; then
            SSH_URL=$(echo "$REMOTE_URL" | sed 's|^https://github\.com/|git@github.com:|')
            echo "Converting remote ${URL_FLAG:-fetch} URL to SSH: $SSH_URL"
            git -C /app/vault remote set-url $URL_FLAG origin "$SSH_URL"
        fi
    done
fi

# Run as botuser via su, falling back to direct exec
exec su -s /bin/sh botuser -c "exec $*" 2>/dev/null || exec "$@"
