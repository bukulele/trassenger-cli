# Building Trassenger TUI for Distribution

This guide covers building executable releases for macOS and Windows.

## Quick Start

### macOS Build
```bash
./build-macos.sh
```

Output: `dist/macos/` containing:
- `trassenger` - Executable binary
- `launch.command` - Double-click launcher
- `README.txt` - User instructions

**Create DMG (optional):**
```bash
hdiutil create -volname Trassenger -srcfolder dist/macos -ov -format UDZO dist/Trassenger-macOS.dmg
```

### Windows Build (Cross-compile)
```bash
./build-windows.sh
```

Output: `dist/windows/` containing:
- `trassenger.exe` - Executable binary
- `launch.bat` - Launcher with pause
- `README.txt` - User instructions

**Create ZIP (optional):**
```bash
cd dist/windows && zip -r ../Trassenger-Windows.zip . && cd -
```

---

## Prerequisites

### macOS Development
- **Xcode Command Line Tools**
  ```bash
  xcode-select --install
  ```

- **Rust** (if not installed)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

### Windows Cross-Compilation (from macOS)
- **MinGW-w64** (for cross-compiling to Windows)
  ```bash
  brew install mingw-w64
  ```

- **Windows Rust Target**
  ```bash
  rustup target add x86_64-pc-windows-gnu
  ```

### Windows Native Build
If building on a Windows machine:
```powershell
# Install Rust from https://rustup.rs
# Then build:
cargo build --release

# Binary will be at: target\release\trassenger-tui.exe
```

---

## Build Details

### Binary Sizes (Approximate)
- **macOS**: ~5MB (stripped)
- **Windows**: ~6MB (stripped)
- **Debug builds**: ~20MB (unstripped)

### Optimization
Both scripts use `cargo build --release` with these optimizations:
- Link-time optimization (LTO)
- Debug symbols stripped
- Code optimization level: 3

### Data Storage Locations

**macOS:**
```
~/Library/Application Support/trassenger/
├── keys/keypair.json
├── peers.json
├── config.json
└── data/messages.db
```

**Windows:**
```
%APPDATA%\trassenger\
├── keys\keypair.json
├── peers.json
├── config.json
└── data\messages.db
```

**Linux:**
```
~/.local/share/trassenger/
├── keys/keypair.json
├── peers.json
├── config.json
└── data/messages.db
```

---

## Distribution Checklist

### Before Release
- [ ] Update version in `Cargo.toml`
- [ ] Update `CLAUDE.md` with new features
- [ ] Test on clean machine (no existing config)
- [ ] Test message exchange between platforms
- [ ] Verify export/import works
- [ ] Check adaptive polling behavior

### macOS Distribution
- [ ] Build with `./build-macos.sh`
- [ ] Test binary on clean macOS installation
- [ ] Sign binary (optional, requires Apple Developer ID)
  ```bash
  codesign --sign "Developer ID" dist/macos/trassenger
  ```
- [ ] Notarize (optional, for Gatekeeper)
- [ ] Create DMG or ZIP
- [ ] Test DMG/ZIP installation

### Windows Distribution
- [ ] Build with `./build-windows.sh`
- [ ] Test on Windows VM or real Windows machine
- [ ] Sign binary (optional, requires code signing certificate)
- [ ] Create installer with Inno Setup (optional)
- [ ] Create ZIP archive
- [ ] Test on clean Windows installation

---

## Advanced: GitHub Actions CI/CD

To automate builds, create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build-macos:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo build --release
      - run: strip target/release/trassenger-tui
      - uses: actions/upload-artifact@v3
        with:
          name: trassenger-macos
          path: target/release/trassenger-tui

  build-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo build --release
      - uses: actions/upload-artifact@v3
        with:
          name: trassenger-windows
          path: target/release/trassenger-tui.exe
```

---

## Troubleshooting

### macOS: "App cannot be opened because it is from an unidentified developer"

**Solution 1 (Recommended):**
Right-click → Open → Click "Open" in dialog

**Solution 2:**
```bash
xattr -d com.apple.quarantine dist/macos/trassenger
```

### Windows: "Windows protected your PC" warning

**Solution:**
Click "More info" → "Run anyway"

This is normal for unsigned apps. To avoid this, sign the binary with a code signing certificate.

### Windows: Missing DLL errors

The binary is statically linked and should not require external DLLs. If you see DLL errors, ensure you're using the GNU toolchain:
```bash
rustup default stable-x86_64-pc-windows-gnu
```

### Cross-compilation fails

If Windows cross-compilation fails on macOS:
```bash
# Reinstall mingw-w64
brew uninstall mingw-w64
brew install mingw-w64

# Reinstall Windows target
rustup target remove x86_64-pc-windows-gnu
rustup target add x86_64-pc-windows-gnu

# Try again
./build-windows.sh
```

---

## Linux Build (Bonus)

For Linux users:
```bash
cargo build --release
strip target/release/trassenger-tui
cp target/release/trassenger-tui dist/trassenger-linux
```

Create AppImage or .deb package using tools like:
- **AppImage**: https://appimage.org/
- **cargo-deb**: `cargo install cargo-deb && cargo deb`

---

## Code Signing (Optional but Recommended)

### macOS
```bash
# Get your signing identity
security find-identity -v -p codesigning

# Sign the binary
codesign --sign "Developer ID Application: Your Name" \
  --options runtime \
  --entitlements entitlements.plist \
  dist/macos/trassenger

# Verify
codesign -vvv --deep --strict dist/macos/trassenger
```

### Windows
Use `signtool.exe` (requires Windows SDK):
```powershell
signtool sign /f certificate.pfx /p password /t http://timestamp.digicert.com dist/windows/trassenger.exe
```

---

## Support

For build issues, check:
- Rust version: `rustc --version` (should be 1.70+)
- Cargo version: `cargo --version`
- Installed targets: `rustup target list --installed`

If problems persist, file an issue with:
- Operating system and version
- Rust/Cargo version
- Full error output
