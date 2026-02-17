// Background polling for the daemon
// Polls all conversation queues every 60s, sends notifications on new messages.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use trassenger_lib::{crypto, crypto::Keypair, mailbox::MailboxClient, storage};
use crate::DaemonState;

/// Events sent from the polling thread to the main thread
pub enum DaemonEvent {
    /// New unread count
    UnreadCount(usize),
    /// TUI process was detected as opened
    TuiOpened,
    /// TUI process was detected as closed
    TuiClosed,
}

/// Main polling loop (runs in a dedicated thread with its own tokio runtime)
pub fn run_polling(state: Arc<Mutex<DaemonState>>, tx: std::sync::mpsc::Sender<DaemonEvent>) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async move {
        polling_loop(state, tx).await;
    });
}

async fn polling_loop(_state: Arc<Mutex<DaemonState>>, tx: std::sync::mpsc::Sender<DaemonEvent>) {
    // Load keypair
    let keypair = match storage::load_keypair() {
        Ok(kp) => kp,
        Err(e) => {
            eprintln!("[daemon] Failed to load keypair: {}. Polling disabled.", e);
            return;
        }
    };

    let config = storage::load_config().unwrap_or_else(|_| storage::Config {
        server_url: trassenger_lib::config::DEFAULT_SERVER_URL.to_string(),
        polling_interval_secs: 60,
    });

    let client = MailboxClient::new(config.server_url.clone());
    let mut tui_was_running = false;
    let mut unread: usize = 0;

    loop {
        // Check TUI running flag file
        let tui_running = tui_running_flag_exists();
        if tui_running && !tui_was_running {
            tui_was_running = true;
            unread = 0;
            let _ = tx.send(DaemonEvent::TuiOpened);
        } else if !tui_running && tui_was_running {
            tui_was_running = false;
            let _ = tx.send(DaemonEvent::TuiClosed);
        }

        // Poll all queues
        let new_msgs = poll_all_queues(&client, &keypair, tui_running).await;
        if new_msgs > 0 && !tui_running {
            unread += new_msgs;
            let _ = tx.send(DaemonEvent::UnreadCount(unread));
            send_notification(new_msgs);
        }
        if tui_running && unread > 0 {
            unread = 0;
            let _ = tx.send(DaemonEvent::UnreadCount(0));
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn poll_all_queues(
    client: &MailboxClient,
    keypair: &Keypair,
    tui_running: bool,
) -> usize {
    let peers = match storage::load_peers() {
        Ok(p) => p,
        Err(_) => return 0,
    };

    let mut total = 0;
    for peer in &peers {
        match poll_queue(client, keypair, &peer.queue_id, tui_running).await {
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
    tui_running: bool,
) -> Result<usize, String> {
    let messages = client.fetch_messages(queue_id).await?;
    if messages.is_empty() {
        return Ok(0);
    }

    let mut count = 0;
    for msg in &messages {
        match process_message(msg, queue_id, keypair) {
            Ok(message) => {
                // If TUI is not running, save and notify; if TUI is running it handles its own
                if !tui_running {
                    if let Ok(conn) = storage::init_message_db() {
                        let _ = storage::save_message(&conn, &message);
                    }
                    count += 1;
                }
                // Delete from server
                let _ = client.delete_message(queue_id, &msg.id).await;
            }
            Err(e) if e.contains("Skipping own message") => {
                // Don't delete own messages - TUI recipient needs them
            }
            Err(e) => {
                eprintln!("[daemon] Failed to process {}: {}", msg.id, e);
                if e.contains("Decryption failed") || e.contains("Signature verification failed") {
                    let _ = client.delete_message(queue_id, &msg.id).await;
                }
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

fn tui_running_flag_exists() -> bool {
    let path = storage::get_app_data_dir()
        .unwrap_or_default()
        .join("tui.running");
    path.exists()
}

fn send_notification(count: usize) {
    let body = if count == 1 {
        "You have 1 new message".to_string()
    } else {
        format!("You have {} new messages", count)
    };

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = notify_rust::Notification::new()
            .summary("Trassenger")
            .body(&body)
            .show();
    }

    #[cfg(target_os = "windows")]
    {
        // Windows toast notifications via notify-rust
        let _ = notify_rust::Notification::new()
            .summary("Trassenger")
            .body(&body)
            .show();
    }
}
