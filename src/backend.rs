use crate::crypto;
use crate::event::AppEvent;
use crate::mailbox::{MailboxClient, ServerMessage};
use crate::storage::{self, Message};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

/// Adaptive polling interval management
#[derive(Debug, Clone)]
pub struct AdaptiveInterval {
    current_secs: u64,
    min_secs: u64,
    max_secs: u64,
}

impl AdaptiveInterval {
    /// Create new adaptive interval with default values (5s min, 60s max)
    pub fn new() -> Self {
        Self {
            current_secs: 5,
            min_secs: 5,
            max_secs: 60,
        }
    }

    /// Reset to minimum interval (when messages are received)
    pub fn reset(&mut self) {
        self.current_secs = self.min_secs;
    }

    /// Increase interval exponentially (when no messages)
    pub fn increase(&mut self) {
        self.current_secs = (self.current_secs * 2).min(self.max_secs);
    }

    /// Get current interval
    pub fn get(&self) -> u64 {
        self.current_secs
    }
}

impl Default for AdaptiveInterval {
    fn default() -> Self {
        Self::new()
    }
}

/// Polling service for TUI
pub struct PollingService {
    server_url: String,
    recipient_sk: Vec<u8>,
    recipient_sign_pk: Vec<u8>,
    event_sender: mpsc::UnboundedSender<AppEvent>,
    command_receiver: mpsc::UnboundedReceiver<PollingCommand>,
}

/// Commands that can be sent to the polling service
#[derive(Debug)]
pub enum PollingCommand {
    ResetInterval,
}

impl PollingService {
    /// Create a new polling service
    /// Returns the service and a sender for commands
    pub fn new(
        server_url: String,
        recipient_sk: Vec<u8>,
        recipient_sign_pk: Vec<u8>,
        event_sender: mpsc::UnboundedSender<AppEvent>,
    ) -> (Self, mpsc::UnboundedSender<PollingCommand>) {
        let (cmd_sender, cmd_receiver) = mpsc::unbounded_channel();

        let service = Self {
            server_url,
            recipient_sk,
            recipient_sign_pk,
            event_sender,
            command_receiver: cmd_receiver,
        };

        (service, cmd_sender)
    }

    /// Start the polling service (spawns background task)
    pub fn start(self) {
        tokio::spawn(async move {
            self.run().await;
        });
    }

    /// Main polling loop with adaptive interval
    async fn run(mut self) {
        let mailbox_client = MailboxClient::new(self.server_url.clone());
        let mut interval = AdaptiveInterval::new();

        loop {
            // Check for commands (non-blocking)
            while let Ok(cmd) = self.command_receiver.try_recv() {
                match cmd {
                    PollingCommand::ResetInterval => {
                        interval.reset();
                        crate::logger::log_to_file("User activity - polling interval reset to 5s");
                        let _ = self.event_sender.send(AppEvent::PollingIntervalUpdate(interval.get()));
                    }
                }
            }

            // Poll all conversation queues
            let has_messages = self.poll_all_queues(&mailbox_client).await;

            // Adjust interval based on activity
            if has_messages {
                // Active conversation detected - reset to minimum interval
                interval.reset();
                crate::logger::log_to_file(&format!("Messages received - polling interval reset to {}s", interval.get()));
            } else {
                // No activity - increase interval (exponential backoff)
                let old_interval = interval.get();
                interval.increase();
                if interval.get() != old_interval {
                    crate::logger::log_to_file(&format!("No messages - polling interval increased to {}s", interval.get()));
                }
            }

            // Notify UI of current polling interval
            let _ = self.event_sender.send(AppEvent::PollingIntervalUpdate(interval.get()));

            // Wait for next poll
            sleep(Duration::from_secs(interval.get())).await;
        }
    }

    /// Poll all conversation queues
    /// Returns true if any messages were received
    async fn poll_all_queues(&self, mailbox_client: &MailboxClient) -> bool {
        let mut total_messages = 0;

        // Load all peers/contacts
        match storage::load_peers() {
            Ok(peers) => {
                for peer in peers {
                    match self.poll_once(mailbox_client, &peer.queue_id).await {
                        Ok(count) => {
                            total_messages += count;
                        }
                        Err(e) => {
                            crate::logger::log_to_file(&format!("Error polling queue {}: {}", peer.queue_id, e));
                        }
                    }
                }
            }
            Err(e) => {
                crate::logger::log_to_file(&format!("Failed to load peers: {}", e));
            }
        }

        total_messages > 0
    }

    /// Poll a single queue and process messages
    /// Returns the number of messages processed
    async fn poll_once(
        &self,
        mailbox_client: &MailboxClient,
        queue_id: &str,
    ) -> Result<usize, String> {
        // Fetch messages from server
        let server_messages = mailbox_client.fetch_messages(queue_id).await?;

        if server_messages.is_empty() {
            return Ok(0);
        }

        crate::logger::log_to_file(&format!("Fetched {} messages from queue {}", server_messages.len(), queue_id));

        let mut processed_count = 0;

        // Process each message
        for server_msg in server_messages {
            crate::logger::log_to_file(&format!(
                "Processing message {} from server (server timestamp: {})",
                server_msg.id, server_msg.timestamp
            ));
            match self.process_message(&server_msg, queue_id).await {
                Ok(message) => {
                    // Save to database
                    if let Ok(conn) = storage::init_message_db() {
                        if let Err(e) = storage::save_message(&conn, &message) {
                            crate::logger::log_to_file(&format!("Failed to save message: {}", e));
                            continue;
                        }
                    }

                    // Emit event to UI
                    if let Err(e) = self.event_sender.send(AppEvent::NewMessage(message.clone())) {
                        crate::logger::log_to_file(&format!("Failed to send NewMessage event: {}", e));
                    }

                    // Delete message from server after successful processing
                    if let Err(e) = mailbox_client.delete_message(queue_id, &server_msg.id).await {
                        crate::logger::log_to_file(&format!("Failed to delete message {}: {}", server_msg.id, e));
                    } else {
                        crate::logger::log_to_file(&format!("Processed and deleted message {}", server_msg.id));
                    }

                    processed_count += 1;
                }
                Err(e) => {
                    // Skip own messages silently (this is normal)
                    // DO NOT delete them - the recipient needs to fetch them!
                    if e.contains("Skipping own message") {
                        continue;
                    }

                    crate::logger::log_to_file(&format!("Failed to process message {}: {}", server_msg.id, e));

                    // If decryption failed, delete the invalid message
                    if e.contains("Decryption failed") || e.contains("Signature verification failed") {
                        crate::logger::log_to_file("Deleting invalid message from server");
                        let _ = mailbox_client.delete_message(queue_id, &server_msg.id).await;
                    }
                }
            }
        }

        Ok(processed_count)
    }

    /// Process a single message (decrypt and verify)
    async fn process_message(
        &self,
        server_msg: &ServerMessage,
        queue_id: &str,
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
        if sender_sign_pk == &self.recipient_sign_pk[..] {
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
        let plaintext = crypto::decrypt_message(ciphertext, sender_encrypt_pk, &self.recipient_sk)?;

        // Parse JSON payload
        let payload: serde_json::Value = serde_json::from_slice(&plaintext)
            .map_err(|e| format!("Failed to parse message JSON: {}", e))?;

        // Extract fields
        let content = payload["content"]
            .as_str()
            .ok_or("Missing content field")?
            .to_string();

        let mut timestamp = payload["timestamp"]
            .as_i64()
            .ok_or("Missing timestamp field")?;

        // Normalize timestamp: if it's in milliseconds (13+ digits), convert to seconds
        if timestamp > 9999999999 {
            timestamp = timestamp / 1000;
        }

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
