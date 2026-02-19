#!/usr/bin/env bash
# Build Trassenger for macOS: produces a .app bundle and .dmg installer
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
VERSION="0.3.1"
APP_NAME="Trassenger"
BUNDLE_ID="com.trassenger.app"

cd "$ROOT_DIR"

echo "==> Building release binaries..."
cargo build --workspace --release

TUI_BIN="target/release/trassenger-tui"
DAEMON_BIN="target/release/trassenger-daemon"

echo "==> Creating .app bundle structure..."
APP_DIR="target/release/bundle/osx/${APP_NAME}.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

# Copy binaries
cp "$TUI_BIN" "${MACOS_DIR}/trassenger-tui"
cp "$DAEMON_BIN" "${MACOS_DIR}/trassenger-daemon"

# Create launcher script
# Detects the best available terminal and opens the TUI in it
cat > "${MACOS_DIR}/${APP_NAME}" << 'LAUNCHER'
#!/bin/bash
DIR="$(cd "$(dirname "$0")" && pwd)"
TUI="$DIR/trassenger-tui"

# Launch daemon if not running
if ! pgrep -f "trassenger-daemon" > /dev/null; then
    "$DIR/trassenger-daemon" &
fi

app_installed() { osascript -e "exists application \"$1\"" 2>/dev/null | grep -q true; }

if app_installed "Warp"; then
    open -a "Warp" --args "$TUI"
elif app_installed "iTerm2"; then
    osascript -e "tell application \"iTerm2\" to create window with default profile command \"$TUI\""
elif app_installed "Alacritty"; then
    open -a "Alacritty" --args -e "$TUI"
elif which kitty &>/dev/null; then
    kitty "$TUI"
else
    osascript -e "tell application \"Terminal\" to do script \"$TUI\""
fi
LAUNCHER
chmod +x "${MACOS_DIR}/${APP_NAME}"

# Info.plist
cat > "${CONTENTS_DIR}/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSUserNotificationAlertStyle</key>
    <string>alert</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST

echo "==> .app bundle created at: ${APP_DIR}"

# Create DMG (requires create-dmg: brew install create-dmg)
if command -v create-dmg &> /dev/null; then
    echo "==> Creating .dmg..."
    DMG_OUT="target/release/${APP_NAME}-${VERSION}.dmg"
    rm -f "$DMG_OUT"
    create-dmg \
        --volname "${APP_NAME} ${VERSION}" \
        --window-size 600 400 \
        --icon-size 128 \
        --app-drop-link 450 180 \
        --icon "${APP_NAME}.app" 150 180 \
        "$DMG_OUT" \
        "$(dirname "${APP_DIR}")"
    echo "==> DMG created: ${DMG_OUT}"
else
    echo "==> Skipping DMG (create-dmg not found; install with: brew install create-dmg)"
    echo "==> App bundle is at: ${APP_DIR}"
fi

echo "==> Done!"
