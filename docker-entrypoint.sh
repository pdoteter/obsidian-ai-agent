#!/bin/bash
set -e

# If USER_ID and GROUP_ID are set, update the botuser to match the host user
# This prevents permission issues with the vault volume
if [ -n "$USER_ID" ] && [ -n "$GROUP_ID" ]; then
    echo "Adjusting UID/GID to ${USER_ID}:${GROUP_ID}"
    groupmod -g "$GROUP_ID" botuser 2>/dev/null || true
    usermod -u "$USER_ID" -g "$GROUP_ID" botuser 2>/dev/null || true
    chown -R botuser:botuser /app
fi

# Setup SSH known hosts for GitHub/GitLab
if [ -d "/root/.ssh" ] || [ -d "/home/botuser/.ssh" ]; then
    mkdir -p /home/botuser/.ssh
    if [ -d "/root/.ssh" ]; then
        cp -r /root/.ssh/* /home/botuser/.ssh/ 2>/dev/null || true
    fi
    chown -R botuser:botuser /home/botuser/.ssh
    chmod 700 /home/botuser/.ssh
    chmod 600 /home/botuser/.ssh/* 2>/dev/null || true

    # Add common git hosts to known_hosts if not present
    if [ ! -f /home/botuser/.ssh/known_hosts ]; then
        ssh-keyscan -t ed25519 github.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        ssh-keyscan -t ed25519 gitlab.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
    fi
fi

# Run as botuser
exec gosu botuser "$@" 2>/dev/null || exec su-exec botuser "$@" 2>/dev/null || exec "$@"
