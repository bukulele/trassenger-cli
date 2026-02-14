use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize)]
struct PostMessageRequest {
    data: String,
    meta: MessageMeta,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct PostMessageResponse {
    pub id: String,
    pub timestamp: i64,
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct GetMessagesResponse {
    pub messages: Vec<ServerMessage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerMessage {
    pub id: String,
    pub timestamp: i64,
    pub data: String,
    pub meta: MessageMeta,
}

#[derive(Debug, Deserialize)]
struct DeleteMessageResponse {
    pub success: bool,
    pub deleted: String,
}

pub struct MailboxClient {
    base_url: String,
    client: reqwest::Client,
}

impl MailboxClient {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { base_url, client }
    }

    /// Send a message to the mailbox server
    pub async fn send_message(
        &self,
        queue_id: &str,
        encrypted_data: String,
        meta: MessageMeta,
    ) -> Result<String, String> {
        let url = format!("{}/mailbox/{}", self.base_url, queue_id);

        let request = PostMessageRequest {
            data: encrypted_data,
            meta,
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Failed to send message: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("HTTP {}: {}", status, error_text));
        }

        let result: PostMessageResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if !result.success {
            return Err("Server reported failure".to_string());
        }

        crate::logger::log_to_file(&format!("Message sent at timestamp: {}", result.timestamp));
        Ok(result.id)
    }

    /// Fetch all messages from the mailbox server
    pub async fn fetch_messages(&self, queue_id: &str) -> Result<Vec<ServerMessage>, String> {
        let url = format!("{}/mailbox/{}", self.base_url, queue_id);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch messages: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("HTTP {}: {}", status, error_text));
        }

        let result: GetMessagesResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(result.messages)
    }

    /// Delete a message from the mailbox server
    pub async fn delete_message(&self, queue_id: &str, message_id: &str) -> Result<(), String> {
        let url = format!("{}/mailbox/{}/{}", self.base_url, queue_id, message_id);

        let response = self
            .client
            .delete(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to delete message: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("HTTP {}: {}", status, error_text));
        }

        let result: DeleteMessageResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if !result.success {
            return Err("Delete operation reported failure".to_string());
        }

        crate::logger::log_to_file(&format!("Successfully deleted message: {}", result.deleted));
        Ok(())
    }
}
