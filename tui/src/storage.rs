use crate::crypto::Keypair;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server_url: String,
    pub polling_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub name: String,
    pub encrypt_pk: String,
    pub sign_pk: String,
    pub queue_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub queue_id: String,  // Which conversation this message belongs to
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    pub msg_type: String,  // 'text', 'file', 'file_chunk'
    pub status: String,    // 'sent', 'delivered', 'read'
    pub is_outbound: bool,
}

/// Get the app data directory
pub fn get_app_data_dir() -> Result<PathBuf, String> {
    // Check if custom data dir is set via environment variable
    if let Ok(custom_dir) = std::env::var("TRASSENGER_DATA_DIR") {
        return Ok(PathBuf::from(custom_dir));
    }

    // Default to system data directory
    dirs::data_dir()
        .map(|p| p.join("trassenger"))
        .ok_or_else(|| "Could not determine app data directory".to_string())
}

/// Initialize storage directories
pub fn init_storage() -> Result<(), String> {
    let app_dir = get_app_data_dir()?;
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Failed to create app directory: {}", e))?;

    let keys_dir = app_dir.join("keys");
    fs::create_dir_all(&keys_dir)
        .map_err(|e| format!("Failed to create keys directory: {}", e))?;

    let data_dir = app_dir.join("data");
    fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data directory: {}", e))?;

    Ok(())
}

/// Save keypair to disk (unencrypted in MVP)
pub fn save_keypair(keypair: &Keypair) -> Result<(), String> {
    let app_dir = get_app_data_dir()?;
    let keypair_path = app_dir.join("keys").join("keypair.json");

    let json = serde_json::to_string_pretty(keypair)
        .map_err(|e| format!("Failed to serialize keypair: {}", e))?;

    fs::write(keypair_path, json)
        .map_err(|e| format!("Failed to write keypair: {}", e))?;

    Ok(())
}

/// Load keypair from disk
pub fn load_keypair() -> Result<Keypair, String> {
    let app_dir = get_app_data_dir()?;
    let keypair_path = app_dir.join("keys").join("keypair.json");

    if !keypair_path.exists() {
        return Err("Keypair not found".to_string());
    }

    let json = fs::read_to_string(keypair_path)
        .map_err(|e| format!("Failed to read keypair: {}", e))?;

    serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse keypair: {}", e))
}

/// Save config to disk
pub fn save_config(config: &Config) -> Result<(), String> {
    let app_dir = get_app_data_dir()?;
    let config_path = app_dir.join("config.json");

    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(config_path, json)
        .map_err(|e| format!("Failed to write config: {}", e))?;

    Ok(())
}

/// Load config from disk
pub fn load_config() -> Result<Config, String> {
    let app_dir = get_app_data_dir()?;
    let config_path = app_dir.join("config.json");

    if !config_path.exists() {
        return Err("Config not found".to_string());
    }

    let json = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config: {}", e))?;

    serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse config: {}", e))
}

/// Save a peer to peers.json
pub fn save_peer(peer: &Peer) -> Result<(), String> {
    let mut peers = load_peers()?;

    // Remove existing peer with same name
    peers.retain(|p| p.name != peer.name);

    // Add new peer
    peers.push((*peer).clone());

    let app_dir = get_app_data_dir()?;
    let peers_path = app_dir.join("peers.json");

    let json = serde_json::to_string_pretty(&peers)
        .map_err(|e| format!("Failed to serialize peers: {}", e))?;

    fs::write(peers_path, json)
        .map_err(|e| format!("Failed to write peers: {}", e))?;

    Ok(())
}

/// Load all peers from disk
pub fn load_peers() -> Result<Vec<Peer>, String> {
    let app_dir = get_app_data_dir()?;
    let peers_path = app_dir.join("peers.json");

    if !peers_path.exists() {
        return Ok(Vec::new());
    }

    let json = fs::read_to_string(peers_path)
        .map_err(|e| format!("Failed to read peers: {}", e))?;

    serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse peers: {}", e))
}

/// Initialize SQLite database for messages
pub fn init_message_db() -> Result<Connection, String> {
    let app_dir = get_app_data_dir()?;
    let db_path = app_dir.join("data").join("messages.db");

    let conn = Connection::open(db_path)
        .map_err(|e| format!("Failed to open database: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            queue_id TEXT NOT NULL,
            sender TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            type TEXT NOT NULL,
            status TEXT NOT NULL,
            is_outbound INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("Failed to create messages table: {}", e))?;

    Ok(conn)
}

/// Save a message to the database
pub fn save_message(conn: &Connection, message: &Message) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO messages (id, queue_id, sender, content, timestamp, type, status, is_outbound)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            message.id,
            message.queue_id,
            message.sender,
            message.content,
            message.timestamp,
            message.msg_type,
            message.status,
            if message.is_outbound { 1 } else { 0 }
        ],
    )
    .map_err(|e| format!("Failed to save message: {}", e))?;

    Ok(())
}

/// Load messages for a specific conversation (queue_id)
pub fn load_messages_for_queue(conn: &Connection, queue_id: &str) -> Result<Vec<Message>, String> {
    let mut stmt = conn
        .prepare("SELECT id, queue_id, sender, content, timestamp, type, status, is_outbound FROM messages WHERE queue_id = ?1 ORDER BY timestamp ASC")
        .map_err(|e| format!("Failed to prepare statement: {}", e))?;

    let messages = stmt
        .query_map([queue_id], |row| {
            Ok(Message {
                id: row.get(0)?,
                queue_id: row.get(1)?,
                sender: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
                msg_type: row.get(5)?,
                status: row.get(6)?,
                is_outbound: row.get::<_, i32>(7)? != 0,
            })
        })
        .map_err(|e| format!("Failed to query messages: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect messages: {}", e))?;

    Ok(messages)
}
