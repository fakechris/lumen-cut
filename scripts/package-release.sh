#!/bin/bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/src-tauri/target}"
RELEASE_DIR="$TARGET_DIR/release"
BUNDLE_DIR="$RELEASE_DIR/bundle"
OUTPUT_DIR="${1:-$ROOT_DIR/build}"
VERSION="$(node -p "require('$ROOT_DIR/package.json').version")"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Release packaging currently supports macOS only." >&2
  exit 1
fi

case "$(uname -m)" in
  arm64) ARCH="aarch64" ;;
  x86_64) ARCH="x86_64" ;;
  *)
    echo "Unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

APP_PATH="$BUNDLE_DIR/macos/lumen-cut.app"
CLI_PATH="$RELEASE_DIR/lumen-cut-cli"
DMG_PATH="$(find "$BUNDLE_DIR/dmg" -maxdepth 1 -type f -name '*.dmg' -print -quit)"

for required in "$APP_PATH" "$CLI_PATH" "$DMG_PATH"; do
  if [[ -z "$required" || ! -e "$required" ]]; then
    echo "Missing release artifact: ${required:-DMG}" >&2
    echo "Run 'pnpm build:desktop' before packaging." >&2
    exit 1
  fi
done

mkdir -p "$OUTPUT_DIR"
find "$OUTPUT_DIR" -mindepth 1 -maxdepth 1 -delete

ditto -c -k --sequesterRsrc --keepParent \
  "$APP_PATH" \
  "$OUTPUT_DIR/lumen-cut_${VERSION}_${ARCH}.app.zip"

cp "$DMG_PATH" "$OUTPUT_DIR/lumen-cut_${VERSION}_${ARCH}.dmg"

tar -C "$RELEASE_DIR" -czf \
  "$OUTPUT_DIR/lumen-cut-cli_${VERSION}_${ARCH}-apple-darwin.tar.gz" \
  lumen-cut-cli

(
  cd "$OUTPUT_DIR"
  shasum -a 256 lumen-cut_* lumen-cut-cli_* > SHA256SUMS.txt
)

echo "Release artifacts: $OUTPUT_DIR"
find "$OUTPUT_DIR" -maxdepth 1 -type f -print | sort
