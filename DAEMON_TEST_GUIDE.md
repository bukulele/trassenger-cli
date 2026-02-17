# Trassenger Daemon & Installer - Testing Guide

## Overview

This guide covers testing the `trassenger-daemon` binary and the macOS/Windows installers added in the workspace refactor.

The daemon is a background process that:
- Shows a system tray icon (macOS menu bar / Windows taskbar)
- Polls for new messages every 60s when the TUI is not running
- Sends system notifications on new messages
- Lets you open the TUI from the tray
- Can register itself as a login item

---

## Prerequisites

Build both binaries first:

```bash
cargo build --workspace --release
```

Outputs:
- `target/release/trassenger-tui`
- `target/release/trassenger-daemon`

---

## 1. Basic Daemon Launch

```bash
./target/release/trassenger-daemon
```

**Expected:**
- A tray icon (T shape) appears in the macOS menu bar
- No terminal output (daemon runs silently)

Click the tray icon to open the menu:

```
Open Trassenger
───────────────
☐ Start at Login
Quit
```

---

## 2. Open TUI from Tray

Click **Open Trassenger** in the tray menu.

**Expected:**
- Terminal.app opens
- `trassenger-tui` binary launches inside it
- Full TUI is displayed

The TUI path is resolved from the daemon's location: `$(dirname trassenger-daemon)/trassenger-tui`. If both binaries are in the same directory (as they are under `target/release/`), this works automatically.

---

## 3. TUI Running Flag

The TUI writes a flag file while it's running so the daemon knows not to duplicate-notify.

**Test:**

1. Start the daemon:
   ```bash
   ./target/release/trassenger-daemon &
   ```

2. Start the TUI in another terminal:
   ```bash
   ./target/release/trassenger-tui
   ```

3. Verify the flag file exists while the TUI is running:
   ```bash
   ls ~/Library/Application\ Support/trassenger/tui.running
   ```
   Should print the file path (file exists).

4. Quit the TUI (`Ctrl+C` or `/quit`).

5. Verify the flag is removed:
   ```bash
   ls ~/Library/Application\ Support/trassenger/tui.running
   ```
   Should print: `No such file or directory`.

---

## 4. Single Instance Guard

Only one daemon should run at a time.

**Test:**

1. Start the daemon:
   ```bash
   ./target/release/trassenger-daemon &
   ```

2. Try to start a second instance:
   ```bash
   ./target/release/trassenger-daemon
   ```

**Expected:**
- Second instance prints: `Trassenger daemon is already running.`
- Only one tray icon is visible

Check the PID file:
```bash
cat ~/Library/Application\ Support/trassenger/daemon.pid
```

---

## 5. Background Polling & Notifications

The daemon polls all conversation queues every 60s when the TUI is not running. On new messages, it sends a system notification and updates the tray icon.

**Test setup:**

You need two user identities. The easiest way is to use a second data directory.

**Terminal 1 — Alice's daemon (default storage):**
```bash
./target/release/trassenger-daemon &
```

**Terminal 2 — Bob's TUI (separate storage, sends a message to Alice):**
```bash
TRASSENGER_DATA_DIR=/tmp/trassenger-bob ./target/release/trassenger-tui
```

In Bob's TUI:
1. Export Bob's contact (`/export` → enter name `Bob`)
2. Import Alice's contact (`/import` → paste Alice's contact JSON)
3. Send a message to Alice

**Back on Alice's machine:**
- Wait up to 60s for the daemon to poll
- A system notification should appear: **"Trassenger — You have 1 new message"**
- The tray icon gains a red dot (unread indicator)

When you open the TUI (via tray or directly), the dot disappears and unread count resets.

---

## 6. Autostart Toggle

### Via tray menu

1. Click the tray icon
2. Click **Start at Login** (checkbox becomes checked)
3. Verify the LaunchAgent was created:
   ```bash
   ls ~/Library/LaunchAgents/ | grep -i trassenger
   ```
   Should show: `com.trassenger.app.plist` or similar.

4. Click **Start at Login** again to disable
5. Verify the plist is removed:
   ```bash
   ls ~/Library/LaunchAgents/ | grep -i trassenger
   ```
   Should show nothing.

### Via TUI settings

1. Open the TUI:
   ```bash
   ./target/release/trassenger-tui
   ```

2. Type `/settings` and press Enter

3. Navigate to **Start at Login** with ↓ (field 2)

4. Press Enter to toggle

5. Status bar shows: `✓ Daemon will start at login` or `✓ Autostart disabled`

6. Verify same way as above (`ls ~/Library/LaunchAgents/`)

---

## 7. Quit Daemon

Click **Quit** in the tray menu.

**Expected:**
- Tray icon disappears
- `daemon.pid` file is removed:
  ```bash
  ls ~/Library/Application\ Support/trassenger/daemon.pid
  # → No such file or directory
  ```

---

## 8. macOS Build Script

Requires: `brew install create-dmg` (optional, for DMG creation)

```bash
./scripts/build-macos.sh
```

**Expected output:**
```
==> Building release binaries...
==> Creating .app bundle structure...
==> .app bundle created at: target/release/bundle/osx/Trassenger.app
==> Creating .dmg...          ← only if create-dmg is installed
==> Done!
```

**Test the .app:**
```bash
open target/release/bundle/osx/Trassenger.app
```

Should open Terminal.app with the TUI running (and silently start the daemon if not already running).

**Inspect the bundle structure:**
```bash
find target/release/bundle/osx/Trassenger.app -type f
```

Expected:
```
Trassenger.app/Contents/Info.plist
Trassenger.app/Contents/MacOS/Trassenger          ← launcher script
Trassenger.app/Contents/MacOS/trassenger-tui
Trassenger.app/Contents/MacOS/trassenger-daemon
```

---

## 9. Windows MSI (on Windows only)

Prerequisites:
```powershell
cargo install cargo-wix
# Also requires WiX Toolset v3: https://wixtoolset.org/releases/
```

Build:
```powershell
.\scripts\build-windows.ps1
```

**Expected:** `target/release/Trassenger-0.1.0-x86_64.msi`

Install the MSI and verify:
- Both `trassenger-tui.exe` and `trassenger-daemon.exe` appear in `%PROGRAMFILES%\Trassenger\`
- Desktop and Start Menu shortcuts are created
- The daemon starts automatically after install (post-install custom action)

---

## Storage Layout Reference

```
~/Library/Application Support/trassenger/   (macOS)
~/.local/share/trassenger/                  (Linux)
%APPDATA%\trassenger\                       (Windows)
├── keys/
│   └── keypair.json
├── peers.json
├── config.json
├── tui.running          ← written by TUI on start, deleted on exit
├── daemon.pid           ← written by daemon on start, deleted on quit
└── data/
    └── messages.db
```

---

## Troubleshooting

**Tray icon doesn't appear:**
- macOS may require accessibility permission — check System Settings → Privacy → Accessibility
- Kill any leftover daemon process: `pkill trassenger-daemon`
- Remove stale PID file: `rm ~/Library/Application\ Support/trassenger/daemon.pid`

**"Already running" but no tray icon:**
- Stale PID file from a crash — delete it:
  ```bash
  rm ~/Library/Application\ Support/trassenger/daemon.pid
  ```

**Notifications not appearing:**
- Check System Settings → Notifications → allow notifications from Terminal (or the .app)
- macOS 14+: the first notification may require user approval

**"Open Trassenger" opens wrong terminal:**
- The daemon uses AppleScript to open Terminal.app specifically
- If you prefer iTerm2 or another terminal, you'd need to modify `launch_tui()` in `daemon/src/main.rs`

**Autostart not working after reboot:**
- Verify the LaunchAgent plist: `cat ~/Library/LaunchAgents/*.trassenger*`
- Check it points to the correct binary path
- Manually load: `launchctl load ~/Library/LaunchAgents/<plist-name>`
