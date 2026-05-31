#!/usr/bin/env bash
# Fully hands-off QA install for this Mac: CLOSE the running app → BUILD → INSTALL → REOPEN.
# You never have to touch the app. Usage: scripts/qa-install.sh [debug|release]  (default: debug)
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE="${1:-debug}"
if [ "$PROFILE" != "debug" ] && [ "$PROFILE" != "release" ]; then
  echo "✗ profile must be 'debug' or 'release' (got '$PROFILE')" >&2
  exit 1
fi

# App/bundle name comes from tauri config so a rename can't break this script.
APP_NAME="$(jq -r '.productName' src-tauri/tauri.conf.json)"

quit_app() {
  echo "▶ Closing any running ${APP_NAME}…"
  osascript -e "quit app \"${APP_NAME}\"" >/dev/null 2>&1 || true
  pkill -x "${APP_NAME}" >/dev/null 2>&1 || true
  pkill -f "/${APP_NAME}.app/" >/dev/null 2>&1 || true
  # Wait (up to ~5s) for the process to actually exit before we overwrite the bundle.
  for _ in 1 2 3 4 5; do
    pgrep -x "${APP_NAME}" >/dev/null 2>&1 || break
    sleep 1
  done
}

# 1) CLOSE
quit_app

# 2) BUILD
echo "▶ Building ${APP_NAME} ($PROFILE)…"
if [ "$PROFILE" = "release" ]; then
  pnpm tauri build --bundles app
else
  pnpm tauri build --debug --bundles app
fi

APP="$(ls -d "target/$PROFILE/bundle/macos/"*.app 2>/dev/null | head -1 || true)"
if [ -z "${APP:-}" ] || [ ! -d "$APP" ]; then
  echo "✗ No .app bundle found under target/$PROFILE/bundle/macos/ — build may have failed" >&2
  exit 1
fi
NAME="$(basename "$APP")"

# 3) INSTALL (prefer /Applications, fall back to ~/Applications)
DEST_DIR="/Applications"
[ -w "$DEST_DIR" ] || DEST_DIR="$HOME/Applications"
mkdir -p "$DEST_DIR"
DEST="$DEST_DIR/$NAME"
echo "▶ Installing → $DEST"
rm -rf "$DEST"
cp -R "$APP" "$DEST"
# Locally-built app is unsigned; clear quarantine so it launches without a Gatekeeper prompt.
xattr -dr com.apple.quarantine "$DEST" >/dev/null 2>&1 || true

# 4) REOPEN
echo "▶ Reopening…"
open "$DEST"

cat <<EOF

✅ ${APP_NAME} rebuilt, installed, and reopened ($PROFILE)
   Location: $DEST
   It's a menu-bar app — the icon is in the macOS menu bar (top-right); click it for the popover.
   No Dock icon by design. (First launch may show one Keychain "Always Allow" prompt.)
EOF
