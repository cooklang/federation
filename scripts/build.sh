#!/bin/bash
set -e

echo "Building Cooklang Federation for production..."

echo "1. Building Tailwind CSS (minified)..."
./tailwindcss -i ./styles/input.css -o ./src/web/static/css/output.css --minify

echo "2. Building Rust application (release)..."
cargo build --release

echo ""
echo "✓ Build complete!"
echo ""
echo "Output:"
echo "  Binary: ./target/release/federation"
echo "  CSS size: $(du -h ./src/web/static/css/output.css | awk '{print $1}')"
echo ""
echo "Run with: ./target/release/federation serve"
