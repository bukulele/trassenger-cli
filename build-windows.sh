#!/bin/bash
# Build script for Windows (cross-compile from macOS/Linux)

set -e

echo "🚀 Building Trassenger TUI for Windows..."

# Check if Windows target is installed
if ! rustup target list --installed | grep -q "x86_64-pc-windows-gnu"; then
    echo "📥 Installing Windows target..."
    rustup target add x86_64-pc-windows-gnu
fi

# Check if mingw-w64 is installed (needed for cross-compilation)
if ! command -v x86_64-w64-mingw32-gcc &> /dev/null; then
    echo "⚠️  MinGW-w64 not found. Installing..."
    echo "On macOS: brew install mingw-w64"
    echo "On Linux: sudo apt install mingw-w64"
    echo ""
    read -p "Install mingw-w64 now? (requires Homebrew on macOS) [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        if [[ "$OSTYPE" == "darwin"* ]]; then
            brew install mingw-w64
        else
            echo "Please install manually: sudo apt install mingw-w64"
            exit 1
        fi
    else
        echo "Cannot proceed without mingw-w64. Exiting."
        exit 1
    fi
fi

# Build release binary for Windows
echo "📦 Compiling Windows release build..."
cargo build --release --target x86_64-pc-windows-gnu

# Create distribution directory
DIST_DIR="dist/windows"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# Copy binary
cp target/x86_64-pc-windows-gnu/release/trassenger-tui.exe "$DIST_DIR/trassenger.exe"

# Strip debug symbols to reduce size
echo "🔧 Stripping debug symbols..."
x86_64-w64-mingw32-strip "$DIST_DIR/trassenger.exe" 2>/dev/null || strip "$DIST_DIR/trassenger.exe" || true

# Create README
cat > "$DIST_DIR/README.txt" << 'EOF'
Trassenger TUI - Terminal Encrypted Messenger
==============================================

Installation:
1. Extract this folder anywhere
2. Double-click trassenger.exe to run
   OR open Command Prompt/PowerShell and run: trassenger.exe

Windows Defender may warn about unknown publisher - this is normal
for unsigned apps. Click "More info" -> "Run anyway"

Features:
- End-to-end encryption (X25519 + Ed25519)
- Zero server-side knowledge
- Adaptive polling (5s-60s)
- Shared storage with Tauri version

Data Location:
%APPDATA%\trassenger\

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

Troubleshooting:
- If terminal looks broken, resize the window or maximize it
- For best experience, use Windows Terminal (from Microsoft Store)
- Command Prompt also works but has limited color support

Support: https://github.com/your-repo/trassenger-tui
EOF

# Create launcher batch file
cat > "$DIST_DIR/launch.bat" << 'EOF'
@echo off
trassenger.exe
pause
EOF

# Get binary size
SIZE=$(du -h "$DIST_DIR/trassenger.exe" | cut -f1)

echo ""
echo "✅ Build complete!"
echo "📍 Location: $DIST_DIR"
echo "📊 Binary size: $SIZE"
echo ""
echo "Distribution includes:"
echo "  - trassenger.exe (executable)"
echo "  - launch.bat (launcher with pause on exit)"
echo "  - README.txt (instructions)"
echo ""
echo "To create a ZIP:"
echo "  cd $DIST_DIR && zip -r ../Trassenger-Windows.zip . && cd -"
echo ""
echo "Note: Binary is built for x86_64 Windows (64-bit)"
