// IPC layer for daemon — listens on a local socket, handles TUI commands,
// pushes events back to connected TUI.

use std::sync::{Arc, Mutex};
use trassenger_lib::{crypto, crypto::Keypair, storage};

// ── Socket path ───────────────────────────────────────────────────────────────

#[cfg(unix)]
pub fn socket_path() -> std::path::PathBuf {
    storage::get_app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        .join("trassenger.sock")
}

#[cfg(windows)]
pub fn pipe_name() -> String {
    r"\\.\pipe\trassenger".to_string()
}

// ── Shared IPC state ──────────────────────────────────────────────────────────

/// Signals between IPC and polling layers
pub enum IpcSignal {
    /// TUI connected — switch to fast polling
    TuiConnected,
    /// TUI disconnected — return to slow polling
    TuiDisconnected,
    /// TUI requests interval reset (user just sent a message)
    ResetPollingInterval,
}

/// Shared state for IPC, updated by polling thread
pub struct IpcState {
    pub keypair: Option<Keypair>,
    pub server_url: String,
    /// Sender to notify polling thread of TUI connect/disconnect
    pub signal_tx: tokio::sync::mpsc::UnboundedSender<IpcSignal>,
    /// Current adaptive interval (pushed here by polling thread)
    pub current_interval_secs: u64,
}

// ── Commands from TUI ─────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
pub enum TuiCommand {
    SendMessage {
        queue_id: String,
        plaintext: String,
        peer_encrypt_pk: String,
    },
    LoadMessages {
        queue_id: String,
    },
    LoadPeers,
    ImportContact {
        json: String,
    },
    ExportContact {
        name: String,
    },
    UpdateConfig {
        server_url: String,
        polling_interval_secs: u64,
    },
    ResetPollingInterval,
}

// ── Events to TUI ─────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, Clone)]
#[serde(tag = "type")]
pub enum DaemonEvent {
    NewMessage {
        message: storage::Message,
    },
    Messages {
        queue_id: String,
        messages: Vec<storage::Message>,
    },
    Peers {
        peers: Vec<storage::Peer>,
    },
    ContactImported {
        peer: storage::Peer,
    },
    ContactExported {
        json: String,
    },
    MessageSent,
    PollingInterval {
        secs: u64,
    },
    Error {
        message: String,
    },
}

// ── Sender handle for pushing events to connected TUI ────────────────────────

/// Cloneable handle to send events to the currently connected TUI session.
/// Wrapped in Arc<Mutex<Option<...>>> so the polling thread can push NewMessage.
pub type TuiEventSender = Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<DaemonEvent>>>>;

// ── Main IPC listener ────────────────────────────────────────────────────────

/// Spawn the IPC listener in a background thread with its own tokio runtime.
pub fn start_ipc_listener(
    state: Arc<Mutex<IpcState>>,
    tui_sender: TuiEventSender,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("IPC tokio runtime");
        rt.block_on(ipc_accept_loop(state, tui_sender));
    });
}

#[cfg(unix)]
async fn ipc_accept_loop(state: Arc<Mutex<IpcState>>, tui_sender: TuiEventSender) {
    use tokio::net::UnixListener;

    let path = socket_path();
    // Remove stale socket file if present
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ipc] Failed to bind socket {:?}: {}", path, e);
            return;
        }
    };

    eprintln!("[ipc] Listening on {:?}", path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                eprintln!("[ipc] TUI connected");
                // Signal polling thread: switch to fast polling
                {
                    if let Ok(s) = state.lock() {
                        let _ = s.signal_tx.send(IpcSignal::TuiConnected);
                    }
                }

                // Create event channel for this TUI session
                let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<DaemonEvent>();

                // Register sender so polling thread can push NewMessage
                {
                    if let Ok(mut guard) = tui_sender.lock() {
                        *guard = Some(event_tx.clone());
                    }
                }

                // Send current polling interval immediately on connect
                {
                    if let Ok(s) = state.lock() {
                        let _ = event_tx.send(DaemonEvent::PollingInterval {
                            secs: s.current_interval_secs,
                        });
                    }
                }

                let state_clone = state.clone();
                let tui_sender_clone = tui_sender.clone();

                // Spawn task to handle this TUI connection
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

                    let (reader, mut writer) = tokio::io::split(stream);
                    let mut lines = BufReader::new(reader).lines();

                    loop {
                        tokio::select! {
                            // Commands from TUI
                            line = lines.next_line() => {
                                match line {
                                    Ok(Some(json)) => {
                                        match serde_json::from_str::<TuiCommand>(&json) {
                                            Ok(cmd) => {
                                                let events = handle_command(cmd, &state_clone).await;
                                                for ev in events {
                                                    let serialized = match serde_json::to_string(&ev) {
                                                        Ok(s) => s,
                                                        Err(e) => {
                                                            eprintln!("[ipc] Serialize error: {}", e);
                                                            continue;
                                                        }
                                                    };
                                                    if let Err(e) = writer.write_all(format!("{}\n", serialized).as_bytes()).await {
                                                        eprintln!("[ipc] Write error: {}", e);
                                                        break;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("[ipc] Parse error: {} for: {}", e, json);
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        eprintln!("[ipc] TUI disconnected");
                                        break;
                                    }
                                    Err(e) => {
                                        eprintln!("[ipc] Read error: {}", e);
                                        break;
                                    }
                                }
                            }

                            // Events to push to TUI
                            ev = event_rx.recv() => {
                                match ev {
                                    Some(event) => {
                                        let serialized = match serde_json::to_string(&event) {
                                            Ok(s) => s,
                                            Err(e) => {
                                                eprintln!("[ipc] Serialize error: {}", e);
                                                continue;
                                            }
                                        };
                                        if let Err(e) = writer.write_all(format!("{}\n", serialized).as_bytes()).await {
                                            eprintln!("[ipc] Write error: {}", e);
                                            break;
                                        }
                                    }
                                    None => break,
                                }
                            }
                        }
                    }

                    // TUI disconnected — clear sender, signal polling thread
                    if let Ok(mut guard) = tui_sender_clone.lock() {
                        *guard = None;
                    }
                    if let Ok(s) = state_clone.lock() {
                        let _ = s.signal_tx.send(IpcSignal::TuiDisconnected);
                    }
                });
            }
            Err(e) => {
                eprintln!("[ipc] Accept error: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(windows)]
async fn ipc_accept_loop(state: Arc<Mutex<IpcState>>, tui_sender: TuiEventSender) {
    use tokio::net::windows::named_pipe::{ServerOptions};

    let pipe_name = pipe_name();

    loop {
        let server = match ServerOptions::new()
            .first_pipe_instance(false)
            .create(&pipe_name)
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[ipc] Failed to create named pipe: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        if let Err(e) = server.connect().await {
            eprintln!("[ipc] Pipe connect error: {}", e);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }

        eprintln!("[ipc] TUI connected via named pipe");

        // Signal polling thread
        {
            if let Ok(s) = state.lock() {
                let _ = s.signal_tx.send(IpcSignal::TuiConnected);
            }
        }

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<DaemonEvent>();

        {
            if let Ok(mut guard) = tui_sender.lock() {
                *guard = Some(event_tx.clone());
            }
        }

        // Send current interval
        {
            if let Ok(s) = state.lock() {
                let _ = event_tx.send(DaemonEvent::PollingInterval {
                    secs: s.current_interval_secs,
                });
            }
        }

        let state_clone = state.clone();
        let tui_sender_clone = tui_sender.clone();

        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

            let (reader, mut writer) = tokio::io::split(server);
            let mut lines = BufReader::new(reader).lines();

            loop {
                tokio::select! {
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(json)) => {
                                match serde_json::from_str::<TuiCommand>(&json) {
                                    Ok(cmd) => {
                                        let events = handle_command(cmd, &state_clone).await;
                                        for ev in events {
                                            let serialized = match serde_json::to_string(&ev) {
                                                Ok(s) => s,
                                                Err(e) => {
                                                    eprintln!("[ipc] Serialize error: {}", e);
                                                    continue;
                                                }
                                            };
                                            if let Err(e) = writer.write_all(format!("{}\n", serialized).as_bytes()).await {
                                                eprintln!("[ipc] Write error: {}", e);
                                                break;
                                            }
                                        }
                                    }
                                    Err(e) => eprintln!("[ipc] Parse error: {}", e),
                                }
                            }
                            Ok(None) | Err(_) => {
                                eprintln!("[ipc] TUI disconnected");
                                break;
                            }
                        }
                    }
                    ev = event_rx.recv() => {
                        match ev {
                            Some(event) => {
                                let serialized = match serde_json::to_string(&event) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        eprintln!("[ipc] Serialize error: {}", e);
                                        continue;
                                    }
                                };
                                if let Err(e) = writer.write_all(format!("{}\n", serialized).as_bytes()).await {
                                    eprintln!("[ipc] Write error: {}", e);
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }

            if let Ok(mut guard) = tui_sender_clone.lock() {
                *guard = None;
            }
            if let Ok(s) = state_clone.lock() {
                let _ = s.signal_tx.send(IpcSignal::TuiDisconnected);
            }
        });
    }
}

// ── Command handlers ──────────────────────────────────────────────────────────

async fn handle_command(cmd: TuiCommand, state: &Arc<Mutex<IpcState>>) -> Vec<DaemonEvent> {
    match cmd {
        TuiCommand::LoadPeers => handle_load_peers(),

        TuiCommand::LoadMessages { queue_id } => handle_load_messages(queue_id),

        TuiCommand::SendMessage { queue_id, plaintext, peer_encrypt_pk } => {
            handle_send_message(queue_id, plaintext, peer_encrypt_pk, state).await
        }

        TuiCommand::ImportContact { json } => handle_import_contact(json, state),

        TuiCommand::ExportContact { name } => handle_export_contact(name, state),

        TuiCommand::UpdateConfig { server_url, polling_interval_secs } => {
            handle_update_config(server_url, polling_interval_secs)
        }

        TuiCommand::ResetPollingInterval => {
            if let Ok(s) = state.lock() {
                let _ = s.signal_tx.send(IpcSignal::ResetPollingInterval);
            }
            vec![]
        }

    }
}

fn handle_load_peers() -> Vec<DaemonEvent> {
    match storage::load_peers() {
        Ok(peers) => vec![DaemonEvent::Peers { peers }],
        Err(e) => vec![DaemonEvent::Error { message: e }],
    }
}

fn handle_load_messages(queue_id: String) -> Vec<DaemonEvent> {
    match storage::init_message_db().and_then(|conn| storage::load_messages_for_queue(&conn, &queue_id)) {
        Ok(messages) => vec![DaemonEvent::Messages { queue_id, messages }],
        Err(e) => vec![DaemonEvent::Error { message: e }],
    }
}

async fn handle_send_message(
    queue_id: String,
    plaintext: String,
    peer_encrypt_pk: String,
    state: &Arc<Mutex<IpcState>>,
) -> Vec<DaemonEvent> {
    let (keypair, server_url) = {
        let s = match state.lock() {
            Ok(s) => s,
            Err(_) => return vec![DaemonEvent::Error { message: "State lock poisoned".to_string() }],
        };
        (s.keypair.clone(), s.server_url.clone())
    };

    let keypair = match keypair {
        Some(kp) => kp,
        None => return vec![DaemonEvent::Error { message: "Keypair not loaded".to_string() }],
    };

    let recipient_encrypt_pk = match crypto::from_hex(&peer_encrypt_pk) {
        Ok(pk) => pk,
        Err(e) => return vec![DaemonEvent::Error { message: format!("Invalid peer_encrypt_pk: {}", e) }],
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let payload = serde_json::json!({
        "type": "text",
        "content": plaintext,
        "timestamp": timestamp,
        "sender_id": crypto::to_hex(&keypair.encrypt_pk),
    });

    let payload_bytes = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(e) => return vec![DaemonEvent::Error { message: format!("Serialize payload: {}", e) }],
    };

    let mut message_to_sign = keypair.encrypt_pk.clone();
    let encrypted = match crypto::encrypt_message(&payload_bytes, &recipient_encrypt_pk, &keypair.encrypt_sk) {
        Ok(e) => e,
        Err(e) => return vec![DaemonEvent::Error { message: format!("Encrypt: {}", e) }],
    };
    message_to_sign.extend(encrypted);

    let signed = match crypto::sign_message(&message_to_sign, &keypair.sign_sk) {
        Ok(s) => s,
        Err(e) => return vec![DaemonEvent::Error { message: format!("Sign: {}", e) }],
    };

    let mut final_message = keypair.sign_pk.clone();
    final_message.extend(signed);

    use base64::{Engine as _, engine::general_purpose};
    let encoded = general_purpose::STANDARD.encode(&final_message);

    let local_id = uuid::Uuid::new_v4().to_string();

    // Save outbound message to DB immediately
    let local_message = storage::Message {
        id: local_id.clone(),
        queue_id: queue_id.clone(),
        sender: "You".to_string(),
        content: plaintext,
        timestamp,
        msg_type: "text".to_string(),
        status: "sending".to_string(),
        is_outbound: true,
    };

    let saved = storage::init_message_db()
        .and_then(|conn| storage::save_message(&conn, &local_message))
        .is_ok();

    if !saved {
        return vec![DaemonEvent::Error { message: "Failed to save message to DB".to_string() }];
    }

    // Send to server async — return local_id immediately
    let local_id_clone = local_id.clone();
    let queue_id_clone = queue_id.clone();
    tokio::spawn(async move {
        use trassenger_lib::mailbox::{MailboxClient, MessageMeta};
        let client = MailboxClient::new(server_url);
        match client.send_message(&queue_id_clone, encoded, MessageMeta { filename: None, size: None }).await {
            Ok(_) => {
                // Update status to "sent"
                if let Ok(conn) = storage::init_message_db() {
                    let _ = conn.execute(
                        "UPDATE messages SET status = 'sent' WHERE id = ?1",
                        [&local_id_clone],
                    );
                }
            }
            Err(e) => {
                eprintln!("[ipc] Failed to send message to server: {}", e);
                if let Ok(conn) = storage::init_message_db() {
                    let _ = conn.execute(
                        "UPDATE messages SET status = 'failed' WHERE id = ?1",
                        [&local_id_clone],
                    );
                }
            }
        }
    });

    let _ = local_id;
    vec![DaemonEvent::MessageSent]
}

fn handle_import_contact(json: String, state: &Arc<Mutex<IpcState>>) -> Vec<DaemonEvent> {
    // Parse JSON
    let contact_data: serde_json::Value = match serde_json::from_str(&json) {
        Ok(d) => d,
        Err(e) => return vec![DaemonEvent::Error { message: format!("Invalid JSON: {}", e) }],
    };

    let name = match contact_data["name"].as_str() {
        Some(n) => n.to_string(),
        None => return vec![DaemonEvent::Error { message: "Missing 'name' field".to_string() }],
    };

    let encrypt_pk = match contact_data["encrypt_pk"].as_str() {
        Some(pk) => pk.to_string(),
        None => return vec![DaemonEvent::Error { message: "Missing 'encrypt_pk' field".to_string() }],
    };

    let sign_pk = match contact_data["sign_pk"].as_str() {
        Some(pk) => pk.to_string(),
        None => return vec![DaemonEvent::Error { message: "Missing 'sign_pk' field".to_string() }],
    };

    if let Err(e) = crypto::from_hex(&encrypt_pk) {
        return vec![DaemonEvent::Error { message: format!("Invalid encrypt_pk: {}", e) }];
    }
    if let Err(e) = crypto::from_hex(&sign_pk) {
        return vec![DaemonEvent::Error { message: format!("Invalid sign_pk: {}", e) }];
    }

    let my_encrypt_pk = {
        let s = match state.lock() {
            Ok(s) => s,
            Err(_) => return vec![DaemonEvent::Error { message: "State lock poisoned".to_string() }],
        };
        s.keypair.as_ref().map(|kp| crypto::to_hex(&kp.encrypt_pk))
    };

    if let Some(ref my_pk) = my_encrypt_pk {
        if *my_pk == encrypt_pk {
            return vec![DaemonEvent::Error { message: "Cannot import your own contact".to_string() }];
        }
    }

    // Check duplicates
    let existing = storage::load_peers().unwrap_or_default();
    if existing.iter().any(|p| p.encrypt_pk == encrypt_pk) {
        return vec![DaemonEvent::Error { message: "Contact already exists".to_string() }];
    }

    let my_pk_hex = my_encrypt_pk.unwrap_or_default();
    let queue_id = match crypto::generate_conversation_queue_id(&my_pk_hex, &encrypt_pk) {
        Ok(q) => q,
        Err(e) => return vec![DaemonEvent::Error { message: format!("Queue ID error: {}", e) }],
    };

    let peer = storage::Peer {
        name: name.clone(),
        encrypt_pk,
        sign_pk,
        queue_id,
    };

    match storage::save_peer(&peer) {
        Ok(_) => vec![DaemonEvent::ContactImported { peer }],
        Err(e) => vec![DaemonEvent::Error { message: format!("Save peer failed: {}", e) }],
    }
}

fn handle_export_contact(name: String, state: &Arc<Mutex<IpcState>>) -> Vec<DaemonEvent> {
    let keypair = {
        let s = match state.lock() {
            Ok(s) => s,
            Err(_) => return vec![DaemonEvent::Error { message: "State lock poisoned".to_string() }],
        };
        s.keypair.clone()
    };

    let keypair = match keypair {
        Some(kp) => kp,
        None => return vec![DaemonEvent::Error { message: "Keypair not loaded".to_string() }],
    };

    let contact_json = serde_json::json!({
        "name": name,
        "encrypt_pk": crypto::to_hex(&keypair.encrypt_pk),
        "sign_pk": crypto::to_hex(&keypair.sign_pk),
    });

    let json_string = match serde_json::to_string_pretty(&contact_json) {
        Ok(s) => s,
        Err(e) => return vec![DaemonEvent::Error { message: format!("Serialize: {}", e) }],
    };

    // Save to Downloads folder
    if let Some(home_dir) = dirs::home_dir() {
        let filename = format!("contact-{}.json", name.replace(' ', "-"));
        let path = home_dir.join("Downloads").join(&filename);
        if let Err(e) = std::fs::write(&path, &json_string) {
            return vec![DaemonEvent::Error { message: format!("Write file: {}", e) }];
        }
    }

    vec![DaemonEvent::ContactExported { json: json_string }]
}

fn handle_update_config(server_url: String, polling_interval_secs: u64) -> Vec<DaemonEvent> {
    let config = storage::Config {
        server_url: server_url.clone(),
        polling_interval_secs,
    };
    match storage::save_config(&config) {
        Ok(_) => vec![],
        Err(e) => vec![DaemonEvent::Error { message: format!("Save config: {}", e) }],
    }
}

/// Push a NewMessage event to the connected TUI (if any).
pub fn push_new_message(tui_sender: &TuiEventSender, message: storage::Message) {
    if let Ok(guard) = tui_sender.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(DaemonEvent::NewMessage { message });
        }
    }
}

/// Push a PollingInterval event to the connected TUI (if any).
pub fn push_polling_interval(tui_sender: &TuiEventSender, secs: u64) {
    if let Ok(guard) = tui_sender.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(DaemonEvent::PollingInterval { secs });
        }
    }
}

/// Returns true if a TUI is currently connected.
pub fn is_tui_connected(tui_sender: &TuiEventSender) -> bool {
    tui_sender.lock().ok().and_then(|g| g.as_ref().map(|_| ())).is_some()
}
