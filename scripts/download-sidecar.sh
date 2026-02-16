#!/usr/bin/env bash
#
# Download a prebuilt hledger binary for the given target triple and place it
# in src-tauri/binaries/ with the Tauri sidecar naming convention.
#
# Usage:
#   scripts/download-sidecar.sh <target-triple>
#
# Example:
#   scripts/download-sidecar.sh aarch64-apple-darwin
#
# The script downloads the official hledger release from GitHub.

set -euo pipefail

HLEDGER_VERSION="${HLEDGER_VERSION:-1.42}"

if [ $# -lt 1 ]; then
  echo "Usage: $0 <target-triple>" >&2
  exit 1
fi

TARGET="$1"

case "$TARGET" in
  aarch64-apple-darwin)
    ASSET="hledger-mac-arm64.tar.gz"
    ;;
  x86_64-apple-darwin)
    ASSET="hledger-mac-x64.tar.gz"
    ;;
  x86_64-unknown-linux-gnu)
    ASSET="hledger-linux-x64.tar.gz"
    ;;
  x86_64-pc-windows-msvc)
    ASSET="hledger-windows-x64.zip"
    ;;
  *)
    echo "Unsupported target: $TARGET" >&2
    exit 1
    ;;
esac

URL="https://github.com/simonmichael/hledger/releases/download/${HLEDGER_VERSION}/${ASSET}"
OUTDIR="src-tauri/binaries"
mkdir -p "$OUTDIR"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading $URL ..."
curl -fSL "$URL" -o "$TMPDIR/$ASSET"

echo "Extracting ..."
if [[ "$ASSET" == *.zip ]]; then
  unzip -o "$TMPDIR/$ASSET" -d "$TMPDIR/extracted"
elif [[ "$ASSET" == *.tar.gz ]]; then
  mkdir -p "$TMPDIR/extracted"
  tar -xzf "$TMPDIR/$ASSET" -C "$TMPDIR/extracted"
else
  echo "Unsupported archive format: $ASSET" >&2
  exit 1
fi

# Find the hledger binary inside the extracted archive
HLEDGER_BIN=$(find "$TMPDIR/extracted" \( -name "hledger" -o -name "hledger.exe" \) | head -1)
if [ -z "$HLEDGER_BIN" ]; then
  echo "Could not find hledger binary in archive" >&2
  exit 1
fi

if [[ "$TARGET" == *"windows"* ]]; then
  DEST="$OUTDIR/hledger-${TARGET}.exe"
else
  DEST="$OUTDIR/hledger-${TARGET}"
fi

cp "$HLEDGER_BIN" "$DEST"
chmod +x "$DEST"
echo "Installed $DEST"
