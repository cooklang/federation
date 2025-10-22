#!/bin/bash
set -e

echo "Downloading Tailwind CSS standalone CLI..."

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin*)
        if [ "$ARCH" = "arm64" ]; then
            URL="https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-macos-arm64"
        else
            URL="https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-macos-x64"
        fi
        ;;
    Linux*)
        if [ "$ARCH" = "aarch64" ]; then
            URL="https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-arm64"
        else
            URL="https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-x64"
        fi
        ;;
    MINGW*|MSYS*|CYGWIN*)
        URL="https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-windows-x64.exe"
        ;;
    *)
        echo "Unsupported OS: $OS"
        exit 1
        ;;
esac

echo "Platform: $OS $ARCH"
echo "Downloading from: $URL"

curl -sLO "$URL"
chmod +x tailwindcss-*

# Rename to simple 'tailwindcss' for convenience
if [[ "$URL" == *".exe"* ]]; then
    mv tailwindcss-*.exe tailwindcss.exe
    echo "✓ Downloaded: tailwindcss.exe"
else
    mv tailwindcss-* tailwindcss
    echo "✓ Downloaded: tailwindcss"
fi

echo ""
echo "Tailwind CLI ready! You can now run:"
echo "  ./scripts/dev.sh       # Development with hot reload"
echo "  ./scripts/build.sh     # Production build"
