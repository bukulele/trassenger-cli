// Background polling for the daemon
// Polls all conversation queues, adaptive interval based on TUI connection.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use trassenger_lib::{crypto, crypto::Keypair, mailbox::MailboxClient, storage};
use crate::DaemonState;
use crate::ipc::{IpcSignal, IpcState, TuiEventSender};

// ── Adaptive interval ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AdaptiveInterval {
    current_secs: u64,
    min_secs: u64,
    max_secs: u64,
}

impl AdaptiveInterval {
    pub fn new(min_secs: u64, max_secs: u64) -> Self {
        Self { current_secs: min_secs, min_secs, max_secs }
    }

    pub fn reset(&mut self) {
        self.current_secs = self.min_secs;
    }

    pub fn increase(&mut self) {
        self.current_secs = (self.current_secs * 2).min(self.max_secs);
    }

    pub fn get(&self) -> u64 {
        self.current_secs
    }
}

// ── Events sent from the polling thread to the main thread ───────────────────

pub enum DaemonEvent {
    /// New unread count
    UnreadCount(usize),
}

// ── Main polling loop ────────────────────────────────────────────────────────

/// Main polling loop (runs in a dedicated thread with its own tokio runtime)
pub fn run_polling(
    _state: Arc<Mutex<DaemonState>>,
    tx: std::sync::mpsc::Sender<DaemonEvent>,
    ipc_state: Arc<Mutex<IpcState>>,
    signal_rx: tokio::sync::mpsc::UnboundedReceiver<IpcSignal>,
    tui_sender: TuiEventSender,
) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async move {
        polling_loop(tx, ipc_state, signal_rx, tui_sender).await;
    });
}

async fn polling_loop(
    tx: std::sync::mpsc::Sender<DaemonEvent>,
    ipc_state: Arc<Mutex<IpcState>>,
    mut signal_rx: tokio::sync::mpsc::UnboundedReceiver<IpcSignal>,
    tui_sender: TuiEventSender,
) {
    // Load keypair
    let keypair = match storage::load_keypair() {
        Ok(kp) => kp,
        Err(e) => {
            eprintln!("[daemon] Failed to load keypair: {}. Polling disabled.", e);
            return;
        }
    };

    // Store keypair in IPC state so handlers can use it
    if let Ok(mut s) = ipc_state.lock() {
        s.keypair = Some(keypair.clone());
    }

    let config = storage::load_config().unwrap_or_else(|_| storage::Config {
        server_url: trassenger_lib::config::DEFAULT_SERVER_URL.to_string(),
        polling_interval_secs: 60,
    });

    let client = MailboxClient::new(config.server_url.clone());

    // When TUI is connected: fast adaptive polling (5s → 60s)
    // When TUI is not connected: slow fixed polling (60s)
    let mut tui_connected = false;
    let mut fast_interval = AdaptiveInterval::new(5, 60);
    let slow_interval = 60u64;
    let mut unread: usize = 0;

    loop {
        // Poll queues — daemon owns all network I/O
        let new_msgs = poll_all_queues(&client, &keypair, &tui_sender).await;

        if tui_connected {
            if new_msgs > 0 {
                fast_interval.reset();
            } else {
                fast_interval.increase();
            }
            crate::ipc::push_polling_interval(&tui_sender, fast_interval.get());
        } else {
            if new_msgs > 0 {
                unread += new_msgs;
                let _ = tx.send(DaemonEvent::UnreadCount(unread));
                send_notification(new_msgs);
            }
        }

        let sleep_secs = if tui_connected { fast_interval.get() } else { slow_interval };

        // Sleep for the interval, but wake immediately on any IPC signal
        let sleep = tokio::time::sleep(Duration::from_secs(sleep_secs));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                signal = signal_rx.recv() => {
                    match signal {
                        Some(IpcSignal::TuiConnected) => {
                            tui_connected = true;
                            unread = 0;
                            fast_interval.reset();
                            eprintln!("[daemon] TUI connected — switching to fast polling");
                            crate::ipc::push_polling_interval(&tui_sender, fast_interval.get());
                            let _ = tx.send(DaemonEvent::UnreadCount(0));
                            break; // Poll immediately
                        }
                        Some(IpcSignal::TuiDisconnected) => {
                            tui_connected = false;
                            eprintln!("[daemon] TUI disconnected — returning to slow polling");
                            break; // Poll immediately
                        }
                        Some(IpcSignal::ResetPollingInterval) => {
                            fast_interval.reset();
                            crate::ipc::push_polling_interval(&tui_sender, fast_interval.get());
                            break; // Poll immediately
                        }
                        None => break,
                    }
                }
            }
        }
    }
}

async fn poll_all_queues(
    client: &MailboxClient,
    keypair: &Keypair,
    tui_sender: &TuiEventSender,
) -> usize {
    let peers = match storage::load_peers() {
        Ok(p) => p,
        Err(_) => return 0,
    };

    let mut total = 0;
    for peer in &peers {
        match poll_queue(client, keypair, &peer.queue_id, tui_sender).await {
            Ok(count) => total += count,
            Err(e) => eprintln!("[daemon] Poll error for {}: {}", peer.queue_id, e),
        }
    }
    total
}

async fn poll_queue(
    client: &MailboxClient,
    keypair: &Keypair,
    queue_id: &str,
    tui_sender: &TuiEventSender,
) -> Result<usize, String> {
    let messages = client.fetch_messages(queue_id).await?;
    if messages.is_empty() {
        return Ok(0);
    }

    let mut count = 0;
    for msg in &messages {
        match process_message(msg, queue_id, keypair) {
            Ok(message) => {
                let saved = storage::init_message_db()
                    .and_then(|conn| storage::save_message(&conn, &message))
                    .is_ok();
                if saved {
                    count += 1;
                    // Push to TUI if connected
                    crate::ipc::push_new_message(tui_sender, message);
                    // Only delete from server after successfully saving locally
                    let _ = client.delete_message(queue_id, &msg.id).await;
                } else {
                    eprintln!("[daemon] Failed to save message {}, keeping on server for retry", msg.id);
                }
            }
            Err(e) if e.contains("Skipping own message") => {
                // Don't delete own messages - the other side needs to fetch them
            }
            Err(e) => {
                // Log and skip — keep message on server for retry
                // Never delete on crypto failure: could be a transient error or
                // the message was not meant for us.
                eprintln!("[daemon] Failed to process {}: {}", msg.id, e);
            }
        }
    }
    Ok(count)
}

fn process_message(
    server_msg: &trassenger_lib::mailbox::ServerMessage,
    queue_id: &str,
    keypair: &Keypair,
) -> Result<storage::Message, String> {
    use base64::{Engine as _, engine::general_purpose};

    let full_message = general_purpose::STANDARD.decode(&server_msg.data)
        .map_err(|e| format!("base64 decode: {}", e))?;

    if full_message.len() < 32 {
        return Err("Message too short".to_string());
    }

    let sender_sign_pk = &full_message[..32];
    let signed_message = &full_message[32..];

    // Skip own messages
    if sender_sign_pk == keypair.sign_pk.as_slice() {
        return Err("Skipping own message".to_string());
    }

    let unsigned = crypto::verify_signature(signed_message, sender_sign_pk)?;
    if unsigned.len() < 32 {
        return Err("Unsigned message too short".to_string());
    }

    let sender_encrypt_pk = &unsigned[..32];
    let ciphertext = &unsigned[32..];
    let plaintext = crypto::decrypt_message(ciphertext, sender_encrypt_pk, &keypair.encrypt_sk)?;

    let payload: serde_json::Value = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("JSON parse: {}", e))?;

    let content = payload["content"].as_str().ok_or("Missing content")?.to_string();
    let mut timestamp = payload["timestamp"].as_i64().ok_or("Missing timestamp")?;
    if timestamp > 9_999_999_999 {
        timestamp /= 1000;
    }
    let sender_id = payload["sender_id"].as_str().ok_or("Missing sender_id")?.to_string();
    let msg_type = payload["type"].as_str().unwrap_or("text").to_string();

    Ok(storage::Message {
        id: server_msg.id.clone(),
        queue_id: queue_id.to_string(),
        sender: sender_id,
        content,
        timestamp,
        msg_type,
        status: "delivered".to_string(),
        is_outbound: false,
    })
}

fn send_notification(count: usize) {
    let body = if count == 1 {
        "You have 1 new message".to_string()
    } else {
        format!("You have {} new messages", count)
    };

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        let _ = notify_rust::Notification::new()
            .summary("Trassenger")
            .body(&body)
            .sound_name("default")
            .show();
    }
}
