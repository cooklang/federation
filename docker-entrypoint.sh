#!/bin/bash
set -e

# Ensure data directory exists and has correct permissions
mkdir -p /app/data/index
chown -R app:app /app/data

# Switch to app user and run the command
exec runuser -u app -- "$@"
