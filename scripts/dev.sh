#!/bin/bash
# Development mode with hot reload

echo "Starting development environment..."

# Start Tailwind in watch mode (background)
./tailwindcss -i ./styles/input.css -o ./src/web/static/css/output.css --watch &
TAILWIND_PID=$!

echo "Tailwind CSS watching for changes..."
echo "Starting Rust server..."

# Start Rust server
PORT="${PORT:-3001}" \
cargo run -- serve

# Cleanup on exit
kill $TAILWIND_PID 2>/dev/null
