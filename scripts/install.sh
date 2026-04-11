#!/bin/sh
# NTK system installer — macOS / Linux
# Usage: curl -sSf https://raw.githubusercontent.com/user/ntk/main/install.sh | sh
set -e

REPO="user/ntk"
LATEST=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | cut -d'"' -f4)

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  ARCH="x86_64"  ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

URL="https://github.com/${REPO}/releases/download/${LATEST}/ntk-${OS}-${ARCH}"
DEST="/usr/local/bin/ntk"

echo "Downloading NTK ${LATEST} for ${OS}-${ARCH}..."
curl -sSfL "$URL" -o /tmp/ntk
chmod +x /tmp/ntk

if [ -w /usr/local/bin ]; then
    mv /tmp/ntk "$DEST"
else
    sudo mv /tmp/ntk "$DEST"
fi

echo "NTK installed to $DEST"
echo "Run: ntk init -g"
