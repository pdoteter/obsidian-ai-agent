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

# Setup SSH keys: copy from read-only mount to writable home dir
if [ -d "/tmp/.ssh-host" ]; then
    echo "Copying SSH keys from /tmp/.ssh-host to /home/botuser/.ssh"
    mkdir -p /home/botuser/.ssh
    cp -r /tmp/.ssh-host/* /home/botuser/.ssh/ 2>/dev/null || true
    chown -R botuser:botuser /home/botuser/.ssh
    chmod 700 /home/botuser/.ssh
    chmod 600 /home/botuser/.ssh/* 2>/dev/null || true

    # Add common git hosts to known_hosts if not present
    if [ ! -f /home/botuser/.ssh/known_hosts ]; then
        ssh-keyscan -t ed25519 github.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        ssh-keyscan -t ed25519 gitlab.com >> /home/botuser/.ssh/known_hosts 2>/dev/null || true
        chown botuser:botuser /home/botuser/.ssh/known_hosts
    fi
else
    echo "No SSH keys mounted at /tmp/.ssh-host — git push over SSH will not work"
fi

# Run as botuser via su, falling back to direct exec
exec su -s /bin/sh botuser -c "exec $*" 2>/dev/null || exec "$@"
