#!/usr/bin/env bash
# Build the MLT app bundle and install + launch it on this Mac for manual QA — so you can
# exercise it as a real user would (menu-bar icon, popover, notifications) rather than via
# `tauri dev`. Usage: scripts/qa-install.sh [debug|release]   (default: debug = faster build)
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE="${1:-debug}"
if [ "$PROFILE" != "debug" ] && [ "$PROFILE" != "release" ]; then
  echo "✗ profile must be 'debug' or 'release' (got '$PROFILE')" >&2
  exit 1
fi

echo "▶ Building MLT app bundle ($PROFILE)…"
if [ "$PROFILE" = "release" ]; then
  pnpm tauri build --bundles app
else
  pnpm tauri build --debug --bundles app
fi

APP="$(ls -d "target/$PROFILE/bundle/macos/"*.app 2>/dev/null | head -1 || true)"
if [ -z "${APP:-}" ] || [ ! -d "$APP" ]; then
  echo "✗ No .app bundle found under target/$PROFILE/bundle/macos/" >&2
  exit 1
fi
NAME="$(basename "$APP")"

# Install location: prefer /Applications, fall back to ~/Applications if not writable.
DEST_DIR="/Applications"
[ -w "$DEST_DIR" ] || DEST_DIR="$HOME/Applications"
mkdir -p "$DEST_DIR"
DEST="$DEST_DIR/$NAME"

echo "▶ Quitting any running instance…"
osascript -e "quit app \"${NAME%.app}\"" >/dev/null 2>&1 || true
pkill -f "$NAME" >/dev/null 2>&1 || true
sleep 1

echo "▶ Installing → $DEST"
rm -rf "$DEST"
cp -R "$APP" "$DEST"
# Locally-built app is unsigned; clear quarantine so it launches without a Gatekeeper prompt.
xattr -dr com.apple.quarantine "$DEST" >/dev/null 2>&1 || true

echo "▶ Launching…"
open "$DEST"

cat <<EOF

✅ MLT installed and launched ($PROFILE build)
   Location: $DEST
   It's a menu-bar app — look for the MLT icon in the macOS menu bar (top-right), then click
   it to open the popover. There is no Dock icon by design.
   First launch may show one Keychain "Always Allow" prompt for Claude credentials.
EOF
