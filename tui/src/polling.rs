use crate::crypto;
use crate::mailbox::{MailboxClient, ServerMessage};
use crate::storage::{self, Message};
use tauri::{AppHandle, Emitter};
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

pub struct PollingService {
    cancel_token: CancellationToken,
}

impl PollingService {
    pub fn new() -> Self {
        Self {
            cancel_token: CancellationToken::new(),
        }
    }

    pub async fn start(
        &self,
        app_handle: AppHandle,
        server_url: String,
        interval_secs: u64,
        recipient_sk: Vec<u8>,
        recipient_sign_pk: Vec<u8>,
    ) {
        let cancel_token = self.cancel_token.clone();
        let mut ticker = interval(Duration::from_secs(interval_secs));

        let mailbox_client = MailboxClient::new(server_url);

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    println!("Polling service stopped");
                    break;
                }
                _ = ticker.tick() => {
                    // Load all conversations and poll each queue
                    match storage::load_peers() {
                        Ok(peers) => {
                            for peer in peers {
                                if let Err(e) = Self::poll_once(
                                    &mailbox_client,
                                    &peer.queue_id,
                                    &recipient_sk,
                                    &recipient_sign_pk,
                                    &app_handle,
                                ).await {
                                    eprintln!("Polling error for queue {}: {}", peer.queue_id, e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to load peers: {}", e);
                        }
                    }
                }
            }
        }
    }

    async fn poll_once(
        mailbox_client: &MailboxClient,
        queue_id: &str,
        recipient_sk: &[u8],
        recipient_sign_pk: &[u8],
        app_handle: &AppHandle,
    ) -> Result<(), String> {
        // Fetch messages from server
        let server_messages = mailbox_client.fetch_messages(queue_id).await?;

        if server_messages.is_empty() {
            return Ok(());
        }

        println!("Fetched {} messages from server", server_messages.len());

        // Process each message
        for server_msg in server_messages {
            println!("Processing message {} (timestamp: {}, meta: {:?})",
                server_msg.id, server_msg.timestamp, server_msg.meta);
            match Self::process_message(&server_msg, queue_id, recipient_sk, recipient_sign_pk).await {
                Ok(message) => {
                    // Save to database
                    if let Ok(conn) = storage::init_message_db() {
                        if let Err(e) = storage::save_message(&conn, &message) {
                            eprintln!("Failed to save message: {}", e);
                        }
                    }

                    // Emit event to frontend
                    if let Err(e) = app_handle.emit("new-message", message.clone()) {
                        eprintln!("Failed to emit new-message event: {}", e);
                    }

                    // Delete message from server after successful processing
                    if let Err(e) = mailbox_client.delete_message(queue_id, &server_msg.id).await {
                        eprintln!("Failed to delete message {}: {}", server_msg.id, e);
                    } else {
                        println!("Deleted message {} from server", server_msg.id);
                    }
                }
                Err(e) => {
                    // Skip own messages silently (this is normal)
                    if e.contains("Skipping own message") {
                        continue;
                    }

                    eprintln!("Failed to process message {}: {}", server_msg.id, e);

                    // If decryption failed, this message is probably not for us or corrupted
                    // Delete it to avoid reprocessing it forever
                    if e.contains("Decryption failed") || e.contains("Signature verification failed") {
                        eprintln!("Message appears to be invalid, deleting from server");
                        if let Err(delete_err) = mailbox_client.delete_message(queue_id, &server_msg.id).await {
                            eprintln!("Failed to delete invalid message {}: {}", server_msg.id, delete_err);
                        } else {
                            println!("Deleted invalid message {} from server", server_msg.id);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn process_message(
        server_msg: &ServerMessage,
        queue_id: &str,
        recipient_sk: &[u8],
        recipient_sign_pk: &[u8],
    ) -> Result<Message, String> {
        // Decode from base64
        use base64::{Engine as _, engine::general_purpose};
        let full_message = general_purpose::STANDARD.decode(&server_msg.data)
            .map_err(|e| format!("Failed to decode base64: {}", e))?;

        // Message format: [sender_sign_pk (32)] + [signed_message]
        if full_message.len() < 32 {
            return Err("Message too short to contain sender signing key".to_string());
        }

        let sender_sign_pk = &full_message[..32];
        let signed_message = &full_message[32..];

        // Skip messages sent by yourself (can't decrypt your own messages)
        if sender_sign_pk == recipient_sign_pk {
            return Err("Skipping own message".to_string());
        }

        // Verify signature with sender's signing public key
        let unsigned = crypto::verify_signature(signed_message, sender_sign_pk)?;

        // Unsigned message format: [sender_encrypt_pk (32)] + [encrypted_content]
        if unsigned.len() < 32 {
            return Err("Message too short to contain sender encryption key".to_string());
        }

        let sender_encrypt_pk = &unsigned[..32];
        let ciphertext = &unsigned[32..];

        // Decrypt the message
        let plaintext = crypto::decrypt_message(ciphertext, sender_encrypt_pk, recipient_sk)?;

        // Parse JSON payload
        let payload: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| format!("Failed to parse message JSON: {}", e))?;

        // Extract fields
        let content = payload["content"]
            .as_str()
            .ok_or("Missing content field")?
            .to_string();

        let timestamp = payload["timestamp"]
            .as_i64()
            .ok_or("Missing timestamp field")?;

        let sender_id = payload["sender_id"]
            .as_str()
            .ok_or("Missing sender_id field")?
            .to_string();

        let msg_type = payload["type"]
            .as_str()
            .unwrap_or("text")
            .to_string();

        Ok(Message {
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

}
