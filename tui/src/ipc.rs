// IPC client — connects TUI to the daemon socket, sends commands, receives events.

use tokio::sync::mpsc;
use crate::storage;
use crate::event::AppEvent;

// ── Socket path (must match daemon/src/ipc.rs) ────────────────────────────────

#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    storage::get_app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        .join("trassenger.sock")
}

#[cfg(windows)]
fn pipe_name() -> String {
    r"\\.\pipe\trassenger".to_string()
}

// ── Commands to daemon ────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, Clone)]
#[serde(tag = "type")]
pub enum DaemonCommand {
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

// ── Events from daemon ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, Clone)]
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

// ── DaemonClient ─────────────────────────────────────────────────────────────

/// Wraps a connection to the daemon socket.
/// Commands are sent via `send_command()`; incoming events are forwarded into the AppEvent channel.
pub struct DaemonClient {
    command_tx: mpsc::UnboundedSender<DaemonCommand>,
    /// Receiver for one-shot responses (LoadMessages, LoadPeers, etc.)
    response_rx: mpsc::UnboundedReceiver<DaemonEvent>,
}

impl DaemonClient {
    /// Connect to the daemon. Returns error string if daemon is not running.
    pub async fn connect(event_tx: mpsc::UnboundedSender<AppEvent>) -> Result<Self, String> {
        #[cfg(unix)]
        {
            Self::connect_unix(event_tx).await
        }
        #[cfg(windows)]
        {
            Self::connect_windows(event_tx).await
        }
    }

    #[cfg(unix)]
    async fn connect_unix(event_tx: mpsc::UnboundedSender<AppEvent>) -> Result<Self, String> {
        use tokio::net::UnixStream;

        let path = socket_path();
        let stream = UnixStream::connect(&path).await
            .map_err(|e| format!("Could not connect to daemon at {:?}: {}. Is the daemon running?", path, e))?;

        Self::from_stream(stream, event_tx)
    }

    #[cfg(windows)]
    async fn connect_windows(event_tx: mpsc::UnboundedSender<AppEvent>) -> Result<Self, String> {
        use tokio::net::windows::named_pipe::ClientOptions;

        let name = pipe_name();
        let stream = ClientOptions::new()
            .open(&name)
            .map_err(|e| format!("Could not connect to daemon pipe {}: {}. Is the daemon running?", name, e))?;

        Self::from_stream(stream, event_tx)
    }

    fn from_stream<S>(stream: S, event_tx: mpsc::UnboundedSender<AppEvent>) -> Result<Self, String>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
    {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<DaemonCommand>();
        let (response_tx, response_rx) = mpsc::unbounded_channel::<DaemonEvent>();

        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

            let (reader, mut writer) = tokio::io::split(stream);
            let mut lines = BufReader::new(reader).lines();

            loop {
                tokio::select! {
                    // Outgoing commands
                    cmd = command_rx.recv() => {
                        match cmd {
                            Some(command) => {
                                let json = match serde_json::to_string(&command) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        crate::logger::log_to_file(&format!("[ipc] Serialize command error: {}", e));
                                        continue;
                                    }
                                };
                                if let Err(e) = writer.write_all(format!("{}\n", json).as_bytes()).await {
                                    crate::logger::log_to_file(&format!("[ipc] Write error: {}", e));
                                    break;
                                }
                            }
                            None => break, // DaemonClient dropped
                        }
                    }

                    // Incoming events from daemon
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(json)) => {
                                match serde_json::from_str::<DaemonEvent>(&json) {
                                    Ok(event) => {
                                        // Route event: NewMessage → AppEvent, others → response_rx
                                        match &event {
                                            DaemonEvent::NewMessage { message } => {
                                                let _ = event_tx.send(AppEvent::NewMessage(message.clone()));
                                            }
                                            DaemonEvent::PollingInterval { secs } => {
                                                let _ = event_tx.send(AppEvent::PollingIntervalUpdate(*secs));
                                                // Also forward to response_rx for any waiter
                                                let _ = response_tx.send(event);
                                            }
                                            _ => {
                                                let _ = response_tx.send(event);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        crate::logger::log_to_file(&format!("[ipc] Parse daemon event error: {}: {}", e, json));
                                    }
                                }
                            }
                            Ok(None) => {
                                crate::logger::log_to_file("[ipc] Daemon disconnected");
                                break;
                            }
                            Err(e) => {
                                crate::logger::log_to_file(&format!("[ipc] Read error: {}", e));
                                break;
                            }
                        }
                    }
                }
            }

            crate::logger::log_to_file("[ipc] IPC reader/writer loop ended");
        });

        Ok(DaemonClient { command_tx, response_rx })
    }

    /// Send a command to daemon (fire-and-forget for most commands).
    pub fn send_command(&self, cmd: DaemonCommand) {
        let _ = self.command_tx.send(cmd);
    }

    // Convenience methods

    pub fn load_peers(&self) {
        self.send_command(DaemonCommand::LoadPeers);
    }

    pub fn load_messages(&self, queue_id: &str) {
        self.send_command(DaemonCommand::LoadMessages { queue_id: queue_id.to_string() });
    }

    pub fn send_message(&self, queue_id: &str, plaintext: &str, peer_encrypt_pk: &str) {
        self.send_command(DaemonCommand::SendMessage {
            queue_id: queue_id.to_string(),
            plaintext: plaintext.to_string(),
            peer_encrypt_pk: peer_encrypt_pk.to_string(),
        });
    }

    pub fn import_contact(&self, json: &str) {
        self.send_command(DaemonCommand::ImportContact { json: json.to_string() });
    }

    pub fn export_contact(&self, name: &str) {
        self.send_command(DaemonCommand::ExportContact { name: name.to_string() });
    }

    pub fn update_config(&self, server_url: &str, polling_interval_secs: u64) {
        self.send_command(DaemonCommand::UpdateConfig {
            server_url: server_url.to_string(),
            polling_interval_secs,
        });
    }

    pub fn reset_polling_interval(&self) {
        self.send_command(DaemonCommand::ResetPollingInterval);
    }

    /// Drain any pending response events without blocking.
    /// Returns all events currently in the buffer.
    pub fn try_recv_all(&mut self) -> Vec<DaemonEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.response_rx.try_recv() {
            events.push(ev);
        }
        events
    }
}
