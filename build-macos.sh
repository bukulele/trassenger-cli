#!/bin/bash
# Build script for macOS release

set -e

echo "🚀 Building Trassenger TUI for macOS..."

# Build release binary
echo "📦 Compiling release build..."
cargo build --release

# Create distribution directory
DIST_DIR="dist/macos"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# Copy binary
cp target/release/trassenger-tui "$DIST_DIR/trassenger"

# Strip debug symbols to reduce size
echo "🔧 Stripping debug symbols..."
strip "$DIST_DIR/trassenger"

# Create README
cat > "$DIST_DIR/README.txt" << 'EOF'
Trassenger TUI - Terminal Encrypted Messenger
==============================================

Installation:
1. Open Terminal
2. Navigate to this folder: cd /path/to/this/folder
3. Make executable: chmod +x trassenger
4. Run: ./trassenger

OR copy to /usr/local/bin for global access:
  sudo cp trassenger /usr/local/bin/
  Then run from anywhere: trassenger

Features:
- End-to-end encryption (X25519 + Ed25519)
- Zero server-side knowledge
- Adaptive polling (5s-60s)
- Shared storage with Tauri version

Commands:
- /import - Import contact from JSON file
- /export - Export your contact to Downloads
- /contacts - View contacts
- /settings - View settings
- /quit - Exit app

Shortcuts:
- / - Open command menu
- ↑↓ - Navigate contacts
- Enter - Send message
- Shift+Enter - New line in message
- Esc - Cancel/go back
- Ctrl+C - Quit

Support: https://github.com/your-repo/trassenger-tui
EOF

# Create launcher script for easy execution
cat > "$DIST_DIR/launch.command" << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
chmod +x trassenger
./trassenger
EOF
chmod +x "$DIST_DIR/launch.command"

# Get binary size
SIZE=$(du -h "$DIST_DIR/trassenger" | cut -f1)

echo ""
echo "✅ Build complete!"
echo "📍 Location: $DIST_DIR"
echo "📊 Binary size: $SIZE"
echo ""
echo "Distribution includes:"
echo "  - trassenger (executable)"
echo "  - launch.command (double-click to run)"
echo "  - README.txt (instructions)"
echo ""
echo "To create a DMG:"
echo "  hdiutil create -volname Trassenger -srcfolder $DIST_DIR -ov -format UDZO dist/Trassenger-macOS.dmg"
