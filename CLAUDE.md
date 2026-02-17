# CLAUDE.md - Trassenger TUI

This file provides guidance to Claude Code when working with the **Trassenger TUI** project.

## Project Overview

Trassenger TUI is a **terminal-based (TUI)** end-to-end encrypted messenger with **zero server-side knowledge**. It consists of two binaries in a Cargo workspace:

- `trassenger-tui` — the interactive terminal app (Ratatui)
- `trassenger-daemon` — background tray icon + polling service

**Key Features:**
- Pure Rust terminal application (no web frontend)
- End-to-end encryption (X25519 + Ed25519, pure Rust — no C deps)
- Adaptive polling (5s → 60s exponential backoff) when TUI is open
- Background daemon polls every 60s when TUI is closed, sends system notifications
- System tray icon with unread count badge
- macOS (.dmg) and Windows (.msi) installers via GitHub Actions
- Shared storage with Tauri version (fully interoperable)

**Tech Stack:**
- **TUI**: Ratatui 0.30 + Crossterm 0.29
- **Async**: Tokio 1.x with mpsc channels
- **Crypto**: chacha20poly1305 + ed25519-dalek + x25519-dalek + sha2 (pure Rust)
- **Storage**: SQLite + JSON files
- **HTTP**: reqwest 0.11
- **Tray**: tray-icon 0.21 + tao 0.33 event loop
- **Notifications**: notify-rust 4
- **Autostart**: auto-launch 0.5 (TUI) / 0.6 (daemon)
- **Server**: Deno Deploy (stateless)

## Workspace Structure

```
trassenger-tui/          ← workspace root
├── Cargo.toml           ← workspace manifest (members = ["tui", "daemon"])
├── tui/                 ← TUI app + shared lib
│   ├── Cargo.toml       ← lib target: trassenger_lib, bin target: trassenger-tui
│   └── src/
│       ├── lib.rs       ← re-exports: storage, crypto, config, mailbox, logger
│       ├── main.rs      ← binary entry point, writes/removes tui.running flag
│       ├── app.rs       ← App state, navigation, event handling
│       ├── event.rs     ← AppEvent enum, EventHandler
│       ├── backend.rs   ← Adaptive polling service (TUI-only, uses AppEvent)
│       ├── crypto.rs    ← Encryption, signing, queue ID generation
│       ├── storage.rs   ← SQLite + JSON persistence
│       ├── mailbox.rs   ← HTTP client for server API
│       ├── config.rs    ← Constants (server URL, defaults)
│       ├── logger.rs    ← Logging utility
│       └── ui/
│           ├── mod.rs
│           └── simple.rs ← All views: messages, contacts, settings
├── daemon/              ← Background tray daemon
│   ├── Cargo.toml       ← depends on trassenger-tui (lib as trassenger_lib)
│   └── src/
│       ├── main.rs      ← tao event loop, tray icon, menu, terminal launch
│       └── polling.rs   ← polls queues every 60s, sends notifications
├── scripts/
│   ├── build-macos.sh   ← creates .app bundle + .dmg
│   └── build-windows.ps1 ← creates .msi via cargo-wix
├── tui/wix/main.wxs     ← WiX installer template (cargo-wix expects it here)
└── .github/workflows/release.yml ← builds DMG + MSI on tag push
```

## Daemon vs TUI Coordination

- TUI writes `<data_dir>/tui.running` on start, removes it on exit
- Daemon checks this flag every poll cycle:
  - **TUI running**: daemon skips polling entirely (TUI handles it via adaptive backend)
  - **TUI closed**: daemon polls every 60s, saves to DB, shows notification, updates tray badge
- TUI re-reads the DB from SQLite every 250ms (on `Tick`) so messages saved by the daemon appear instantly when TUI opens

## Message Lifecycle

```
Sender → Encrypt → Sign → Base64 → POST to /mailbox/{queue_id}
Queue → Store indefinitely (no TTL)
Recipient → Poll → Verify → Decrypt → Save to SQLite → DELETE from queue
TUI → Re-reads SQLite every 250ms → displays messages
```

### Adaptive Polling (TUI open)
```
Initial: 5s
No messages: 10s → 20s → 40s → 60s (max)
Messages received: Reset to 5s
```

### Daemon Polling (TUI closed)
- Fixed 60s interval
- On new message: save to SQLite, send system notification, update tray icon badge
- Tray icon: normal icon when 0 unread, unread-dot icon when count > 0

### Deterministic Queue IDs
```rust
queue_id = SHA256(min(pk1, pk2) + max(pk1, pk2))[:16]
```

## Key Components

### tui/src/main.rs (Entry Point)
- Terminal initialization (raw mode, alternate screen)
- Writes `tui.running` flag on start, removes on exit
- Starts adaptive polling service and event handler
- Main loop: receive events → update app → render UI
- Graceful shutdown on Ctrl+C/Ctrl+Q

### tui/src/app.rs (Application State)
**App struct fields:**
- `keypair`: User's encryption + signing keys
- `config`: Server URL, polling interval
- `peers`: Contact list (Vec<Peer>)
- `messages`: Current conversation (Vec<Message>) — reloaded from DB every Tick
- `db_conn`: SQLite connection
- `settings_autostart_enabled`: cached autostart state
- Navigation: current_view, selected_peer_index, input_mode

**Key methods:**
- `initialize()`: Load/generate keypair, config, peers, DB
- `handle_event()`: Route events to handlers
- `send_message_to_peer()`: Full encryption pipeline
- `import_contact()`: Parse JSON, validate, generate queue_id
- `export_contact()`: Generate JSON with public keys
- `submit_settings()`: Save config; field 2 toggles autostart
- `load_messages_for_selected_peer()`: Read DB for current peer (called every Tick)

**Views:**
- Messages: Peer list | Chat history | Input box
- Contacts: List | Import form | Export form
- Settings: Server URL | Polling interval | Start at Login toggle

### tui/src/event.rs (Event System)
**AppEvent enum:**
- `Key(KeyEvent)`: Keyboard input
- `NewMessage(Message)`: Received from polling
- `Tick`: 250ms periodic tick — triggers DB reload
- `PollingIntervalUpdate(u64)`: Adaptive interval changed
- `Paste(String)`: Clipboard paste
- `Quit`: App should exit

### tui/src/backend.rs (Adaptive Polling — TUI only)
- Only runs while TUI is open
- Polls all queues sequentially
- Saves messages to SQLite, sends `AppEvent::NewMessage`
- Doubles interval on idle, resets to 5s on message

### tui/src/ui/simple.rs (All UI Views)
- `render_messages_view()`: peer list (25%) + chat + input (75%)
- `render_contacts_view()`: list / import / export
- `render_settings_view()`: 3 fields — Server URL, Polling Interval, Start at Login
  - Field 2 is a toggle (Enter to flip), not a text input

### daemon/src/main.rs (Tray Daemon)
- `#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]` — no console window
- `tao` event loop required on macOS to pump NSApplication (makes tray icon appear)
- Menu: Open Trassenger | Start at Login (checkbox) | Quit
- Terminal detection on macOS (in order): Warp → iTerm2 → Alacritty → kitty → Terminal.app
- Windows launch: `wt.exe` first, fallback `cmd /c start "" <path>` (empty title string required)
- Single-instance guard via PID file (`<data_dir>/daemon.pid`)
- SIGTERM handler removes PID file and exits cleanly

### daemon/src/polling.rs (Background Polling)
- Dedicated thread with own tokio runtime
- Skips polling when `tui.running` flag exists
- On new message: save to SQLite, `send_notification()`, send `DaemonEvent::UnreadCount`
- `notify-rust` for system notifications on macOS/Windows/Linux

## Wire Protocol

```
[sender_sign_pk (32)]           # Ed25519 public key
  + [signed_message]            # Ed25519 signature + payload
    └─> [sender_encrypt_pk (32)]   # X25519 public key
          + [encrypted_content]     # XChaCha20-Poly1305
            └─> [nonce + cipher + MAC]
              └─> JSON {type, content, timestamp, sender_id}
```

## Storage

**Location:**
- macOS: `~/Library/Application Support/trassenger/`
- Linux: `~/.local/share/trassenger/`
- Windows: `%APPDATA%/trassenger/`

**Files:**
- `keys/keypair.json`: User's keypairs (unencrypted)
- `peers.json`: Contact list
- `config.json`: Settings
- `data/messages.db`: SQLite database
- `tui.running`: Flag file (exists while TUI is open)
- `daemon.pid`: Daemon PID for single-instance guard

**Database schema:**
```sql
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    queue_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    msg_type TEXT NOT NULL,
    status TEXT NOT NULL,
    is_outbound INTEGER NOT NULL
);
```

## Settings View Fields

- Field 0: Server URL (text input)
- Field 1: Polling Interval (text input)
- Field 2: Start at Login (toggle — Enter to flip, no text editing)

Navigate with Up/Down in Normal mode, Enter to edit/toggle.

## Build Commands

```bash
# Development
cargo run --package trassenger-tui

# Build both binaries
cargo build --workspace --release

# macOS .app + .dmg
./scripts/build-macos.sh

# Windows .msi (requires cargo-wix + WiX Toolset v3)
.\scripts\build-windows.ps1

# Release (triggers GitHub Actions → DMG + MSI)
git tag v0.x.y && git push origin v0.x.y
```

## Keyboard Shortcuts

**Global:**
- `Tab` / `Shift+Tab`: Switch views
- `Ctrl+C` / `Ctrl+Q`: Quit

**Messages:**
- `Up` / `Down`: Select conversation
- `Enter`: Start typing
- `Enter` (while typing): Send message
- `Esc`: Cancel input
- `/`: Open slash command menu

**Contacts:**
- `i`: Import contact
- `e`: Export contact
- `Enter`: Submit
- `Esc`: Cancel

**Settings:**
- `Up` / `Down`: Select field
- `Enter`: Edit field / toggle (field 2)
- `Esc`: Cancel

## Installer Details

### macOS (.dmg)
- Built by `scripts/build-macos.sh`
- `.app` bundle with `LSUIElement = true` (no Dock bounce)
- Launcher script auto-detects terminal: Warp → iTerm2 → Alacritty → kitty → Terminal.app
- Both binaries (`trassenger-tui`, `trassenger-daemon`) in `Contents/MacOS/`

### Windows (.msi)
- Built by `cargo wix --package trassenger-tui` (WXS at `tui/wix/main.wxs`)
- Per-user install to `%LOCALAPPDATA%\Trassenger\` (no admin required)
- Desktop + Start Menu shortcuts point to `trassenger-daemon.exe`
- Post-install custom action launches daemon immediately

## Common Issues

**Messages not appearing:**
- Check queue_id matches on both sides
- Verify server: `curl https://trassenger-mailbox.deno.dev/mailbox/test`
- Inspect DB: `sqlite3 ~/Library/.../messages.db`

**Tray icon missing (macOS):**
- Must use `tao` event loop — `thread::sleep` loop does not pump NSApplication

**Windows: "cannot find file" error on tray click:**
- `cmd /c start` requires empty string `""` as first arg (window title), then path

**Daemon not stopping after quit:**
- SIGTERM handler removes PID file and calls `std::process::exit(0)`

## Security Notes

- **No authentication**: Anyone with queue_id can access (by design)
- **Keypairs unencrypted**: Stored in plaintext JSON
- **No forward secrecy**: Compromised key exposes all messages
- **Server sees metadata**: Queue ID, size, timestamp (not content)
- **Contact files safe**: Only public keys, no private keys

## See Also

- `DAEMON_TEST_GUIDE.md` — daemon and installer testing instructions
- `tui/wix/main.wxs` — Windows installer template
- `.github/workflows/release.yml` — CI/CD release pipeline
