# CLAUDE.md - Trassenger TUI

This file provides guidance to Claude Code when working with the **Trassenger TUI** project.

## Project Overview

Trassenger TUI is a **terminal-based (TUI)** end-to-end encrypted messenger with **zero server-side knowledge**. It's a standalone Rust implementation of the Trassenger protocol using the Ratatui framework.

**Key Features:**
- Pure Rust terminal application (no web frontend)
- End-to-end encryption (X25519 + Ed25519)
- Adaptive polling (5s → 60s exponential backoff)
- Shared storage with Tauri version (fully interoperable)
- Lightweight (~5MB binary, ~20MB memory)
- Fast startup (<100ms)

**Tech Stack:**
- **TUI**: Ratatui 0.30 + Crossterm 0.29
- **Async**: Tokio 1.x with mpsc channels
- **Crypto**: sodiumoxide 0.2 (X25519 + Ed25519)
- **Storage**: SQLite + JSON files
- **HTTP**: reqwest 0.11
- **Server**: Deno Deploy (stateless)

## Core Architecture

### Message Lifecycle
```
Sender → Encrypt → Sign → Base64 → POST to /mailbox/{queue_id}
Queue → Store indefinitely (no TTL)
Recipient → Poll → Verify → Decrypt → Save → DELETE from queue
```

### Adaptive Polling
```
Initial: 5s
No messages: 10s → 20s → 40s → 60s (max)
Messages received: Reset to 5s
```

Benefits: 12x reduction in idle requests, maintains active responsiveness

### Deterministic Queue IDs
```rust
queue_id = SHA256(min(pk1, pk2) + max(pk1, pk2))[:16]
```
Both users generate identical queue_id from their public keys!

## Directory Structure

```
src/
├── main.rs          - Terminal setup, event loop, UI rendering
├── app.rs           - App state, navigation, message sending
├── event.rs         - Event system (keyboard, polling, tick)
├── backend.rs       - Adaptive polling service
├── crypto.rs        - Encryption, signing, queue ID generation
├── storage.rs       - SQLite + JSON persistence
├── mailbox.rs       - HTTP client for server API
├── config.rs        - Constants (server URL, defaults)
└── ui/
    ├── messages.rs  - Chat view (peer list, messages, input)
    ├── contacts.rs  - Contact import/export
    └── settings.rs  - Configuration editor
```

## Message Format

### Wire Protocol (Nested)
```
[sender_sign_pk (32)]           # Ed25519 public key
  + [signed_message]            # Ed25519 signature + payload
    └─> [sender_encrypt_pk (32)]   # X25519 public key
          + [encrypted_content]     # XChaCha20-Poly1305
            └─> [nonce + cipher + MAC]
              └─> JSON {type, content, timestamp, sender_id}
```

### Sending (app.rs::send_message_to_peer)
1. Create JSON payload with timestamp
2. Encrypt with recipient's X25519 public key
3. Prepend sender's X25519 public key
4. Sign with sender's Ed25519 private key
5. Prepend sender's Ed25519 public key
6. Base64 encode → POST to server
7. Save to local SQLite

### Receiving (backend.rs::process_message)
1. Poll /mailbox/{queue_id}
2. Base64 decode
3. Extract sender_sign_pk (first 32 bytes)
4. Skip if sender_sign_pk == my_sign_pk (own message)
5. Verify signature
6. Extract sender_encrypt_pk
7. Decrypt with my private key
8. Parse JSON → save to SQLite
9. Send AppEvent::NewMessage to UI
10. DELETE from server

## Key Components

### main.rs (Entry Point)
- Terminal initialization (raw mode, alternate screen)
- Event handler setup (keyboard listener, tick timer)
- Polling service startup
- Main loop: receive events → update app → render UI
- Graceful shutdown (restore terminal on Ctrl+C)

### app.rs (Application State)
**App struct:**
- `keypair`: User's encryption + signing keys
- `config`: Server URL, polling interval
- `peers`: Contact list (Vec<Peer>)
- `messages`: Current conversation (Vec<Message>)
- `db_conn`: SQLite connection
- Navigation state: current_view, selected_peer_index, input_mode

**Key methods:**
- `initialize()`: Load/generate keypair, config, peers, DB
- `handle_event()`: Route events to handlers
- `send_message_to_peer()`: Full encryption pipeline
- `import_contact()`: Parse JSON, validate, generate queue_id
- `export_contact()`: Generate JSON with public keys
- `submit_settings()`: Save config to file

**Views:**
- Messages: Peer list | Chat history | Input box
- Contacts: List | Import form | Export form
- Settings: Server URL | Polling interval | Status

### event.rs (Event System)
**AppEvent enum:**
- `Key(KeyEvent)`: Keyboard input
- `NewMessage(Message)`: Received from polling
- `Tick`: Periodic UI refresh (250ms)
- `PollingIntervalUpdate(u64)`: Adaptive interval changed
- `Quit`: App should exit

**EventHandler:**
- Uses tokio::mpsc::unbounded_channel
- `spawn_keyboard_listener()`: Crossterm event stream
- `spawn_tick_timer()`: 250ms ticker
- `sender()`: Clone sender for other components
- `next()`: Receive next event (async)

### backend.rs (Adaptive Polling)
**AdaptiveInterval:**
```rust
struct AdaptiveInterval {
    current_secs: u64,  // Current interval
    min_secs: 5,        // Minimum (active)
    max_secs: 60,       // Maximum (idle)
}
```

**PollingService:**
- `run()`: Main polling loop
  - Poll all queues sequentially
  - If messages: reset interval to 5s
  - If empty: double interval (up to 60s)
  - Send PollingIntervalUpdate event
  - Sleep for current interval
- `poll_all_queues()`: Load peers, poll each queue_id
- `poll_once()`: Fetch messages, decrypt, save, delete
- `process_message()`: Decrypt + verify signature

### crypto.rs (Cryptography)
All crypto operations using sodiumoxide (libsodium):
- `generate_keypair()`: X25519 + Ed25519
- `encrypt_message()`: XChaCha20-Poly1305
- `decrypt_message()`: Decrypt with sender's public key
- `sign_message()`: Ed25519 signature
- `verify_signature()`: Signature verification
- `generate_conversation_queue_id()`: SHA256-based
- `to_hex()` / `from_hex()`: Encoding utilities

### storage.rs (Persistence)
**Data structures:**
- `Keypair`: encrypt_pk, encrypt_sk, sign_pk, sign_sk
- `Peer`: name, encrypt_pk, sign_pk, queue_id
- `Config`: server_url, polling_interval_secs
- `Message`: id, queue_id, sender, content, timestamp, is_outbound

**Storage location:**
- macOS: `~/Library/Application Support/trassenger/`
- Linux: `~/.local/share/trassenger/`
- Windows: `%APPDATA%/trassenger/`

**Files:**
- `keys/keypair.json`: User's keypairs (unencrypted)
- `peers.json`: Contact list
- `config.json`: Settings
- `data/messages.db`: SQLite database

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

### ui/messages.rs (Messages View)
- `render_messages_view()`: 2-column layout
  - Left: Peer list (25% width)
  - Right: Messages + Input (75% width)
- `render_peer_list()`: Contact list with selection highlight
- `render_messages()`: Chat history
  - Sent messages: Cyan
  - Received messages: Green
  - Timestamps: HH:MM:SS format
- `render_input()`: Message composition box

### ui/contacts.rs (Contacts View)
- `render_contacts_view()`: Routes to list/import/export
- `render_contact_list()`: Shows all contacts + instructions
- `render_import_form()`: Paste JSON → validate → save
- `render_export_form()`: Enter name → generate JSON

### ui/settings.rs (Settings View)
- Server URL editor
- Polling interval editor
- Current polling status (read-only)
- App version info
- Field selection highlighting

## Development Commands

```bash
# Run in development mode
cargo run

# Build release binary
cargo build --release

# Check for errors (fast)
cargo check

# Run tests
cargo test

# Clean build artifacts
cargo clean
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

**Contacts:**
- `i`: Import contact
- `e`: Export contact
- `Enter`: Submit
- `Esc`: Cancel

**Settings:**
- `Up` / `Down`: Select field
- `Enter`: Edit field
- `Enter` (while editing): Save
- `Esc`: Cancel

## Testing End-to-End

### Setup Two Instances

**Terminal 1 (Alice):**
```bash
cargo run
```

**Terminal 2 (Bob):**
```bash
rm -rf ~/Library/Application\ Support/trassenger/
cargo run
```

### Exchange Contacts

1. Alice exports: Tab → Contacts → `e` → Enter name → Enter
2. Copy JSON (manually)
3. Bob imports: `i` → Paste JSON → Enter
4. Bob exports his contact
5. Alice imports Bob's contact

### Send Messages

1. Alice: Tab → Messages → Select Bob → Enter
2. Type "Hello Bob!" → Enter
3. Wait 5-10s for Bob's polling
4. Bob sees message (green)
5. Bob replies: Enter → Type "Hi Alice!" → Enter
6. Alice receives after polling

### Verify Adaptive Polling

Watch status bar: "Polling: 5s"
- Idle 10s → "Polling: 10s"
- Idle 30s → "Polling: 20s"
- Send message → "Polling: 5s" (reset)

## Storage Compatibility

**TUI and Tauri app share the same storage!**

You can:
- Import contact in TUI, see it in Tauri
- Send message in Tauri, receive in TUI
- Switch between apps seamlessly

Both use identical:
- Keypair format
- Peer JSON structure
- Config JSON structure
- SQLite schema

## Important Details

### Polling Strategy
One polling service polls **all** conversation queues:
```rust
for peer in peers {
    poll_once(&peer.queue_id).await;  // Sequential
}
```

### Skip Own Messages
```rust
if sender_sign_pk == my_sign_pk {
    return Err("Skipping own message");  // Can't decrypt
}
```

### Message Deletion
**Recipient** deletes from server after reading, not sender.
Server stores messages indefinitely until deleted.

### Queue Per Conversation
One queue shared by two users, not per-user queues.
Queue ID is deterministic from both public keys.

## Common Issues

**Messages not appearing:**
- Check queue_id matches on both sides
- Verify server is up: `curl https://trassenger-mailbox.deno.dev/mailbox/test`
- Check polling logs in console
- Inspect database: `sqlite3 ~/Library/.../messages.db`

**Can't import contact:**
- Validate JSON has: name, encrypt_pk, sign_pk
- Check hex format (64 chars each)
- Ensure no duplicates (same encrypt_pk)

**Adaptive polling not working:**
- Look for console logs: "💤 No messages", "📨 Messages received"
- Check status bar shows changing interval
- Verify polling service started

## Security Notes

- **No authentication**: Anyone with queue_id can access (by design)
- **Keypairs unencrypted**: Stored in plaintext JSON
- **No forward secrecy**: Compromised key exposes all messages
- **Server sees metadata**: Queue ID, size, timestamp (not content)
- **Contact files safe**: Only public keys, no private keys

## Performance

**Binary:**
- Debug: ~20MB
- Release: ~5MB

**Memory:**
- Idle: ~10MB
- 10 contacts: ~15MB
- 100 messages: ~20MB

**Startup:**
- Debug: ~200ms
- Release: ~50ms

**Scalability:**
- 10 contacts: ~1s per poll cycle
- 100 contacts: ~10s per poll cycle
- Adaptive polling reduces load when idle

## Dependencies

```toml
ratatui = "0.30"
crossterm = { version = "0.29", features = ["event-stream"] }
tui-textarea = "0.7"
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
futures = "0.3"
sodiumoxide = "0.2"
reqwest = { version = "0.11", features = ["json"] }
rusqlite = { version = "0.30", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.21"
hex = "0.4"
uuid = { version = "1", features = ["v4"] }
dirs = "5.0"
chrono = "0.4"
```

## Comparison: TUI vs Tauri

| Feature | TUI | Tauri |
|---------|-----|-------|
| UI | Terminal (Ratatui) | Web (React) |
| Binary Size | ~5MB | ~100MB |
| Memory | ~15MB | ~150MB |
| Startup | <100ms | 2-5s |
| Platform | Terminal/SSH | Desktop GUI |
| Storage | Shared ✅ | Shared ✅ |
| Protocol | Identical ✅ | Identical ✅ |

**Use TUI for:** SSH, low-resource, terminal workflows
**Use Tauri for:** Desktop GUI, system tray, rich media

Both are fully compatible!

## Files Documentation

See also:
- `PROGRESS.md` - Implementation status
- `TEST_GUIDE.md` - Testing instructions
- `Cargo.toml` - Dependencies
