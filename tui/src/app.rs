use crate::crypto::Keypair;
use crate::event::AppEvent;
use crate::storage::{Config, Message, Peer};
use crate::{config, crypto, storage};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rusqlite::Connection;

/// Command/view state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuState {
    Closed,          // Normal chat view
    Contacts,        // Viewing contacts list
    ImportContact,   // Importing a contact
    ExportContact,   // Exporting contact info
    Settings,        // Settings view
}

/// Input mode for text editing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Normal mode - navigation with keyboard
    Normal,
    /// Editing mode - typing text
    Editing,
}

/// Main application state
pub struct App {
    /// User's keypair (encryption + signing)
    pub keypair: Keypair,
    /// Application configuration
    pub config: Config,
    /// List of contacts/peers
    pub peers: Vec<Peer>,
    /// Messages for current conversation
    pub messages: Vec<Message>,
    /// Database connection
    pub db_conn: Connection,

    // Navigation state
    /// Menu overlay state
    pub menu_state: MenuState,
    /// Input mode
    pub input_mode: InputMode,
    /// Selected peer index
    pub selected_peer_index: usize,

    // Message input
    /// Current message being typed
    pub message_input: String,
    /// Cursor position in the active input (char index)
    pub input_cursor: usize,
    /// Slash command menu state
    pub show_slash_menu: bool,
    /// Selected command in slash menu
    pub slash_menu_index: usize,

    // Contact import/export
    /// Contact JSON input (for import)
    pub contact_import_input: String,
    /// Contact name input (for export)
    pub contact_export_name: String,
    /// Exported contact JSON
    pub contact_export_json: String,

    // Settings
    /// Currently editing settings field
    pub settings_selected_field: usize,
    /// Server URL input
    pub settings_server_url: String,
    /// Polling interval input
    pub settings_polling_interval: String,
    /// Daemon autostart enabled state (cached for display)
    pub settings_autostart_enabled: bool,

    // Status
    /// Status message to display
    pub status_message: String,
    /// Current polling interval (for adaptive polling)
    pub current_polling_interval: u64,

    /// Chat scroll offset (0 = at bottom, higher = scrolled up)
    pub chat_scroll_offset: usize,

    /// Should the app quit
    pub should_quit: bool,

    /// Sender for polling commands
    polling_sender: Option<tokio::sync::mpsc::UnboundedSender<crate::backend::PollingCommand>>,

    /// Whether keyboard enhancements are supported (for Shift+Enter)
    pub keyboard_enhancements_supported: bool,
}

impl App {
    /// Initialize the application
    pub fn initialize() -> Result<Self, String> {
        // Initialize crypto
        crypto::init()?;

        // Initialize storage directories
        storage::init_storage()?;

        // Load or generate keypair
        let keypair = match storage::load_keypair() {
            Ok(kp) => kp,
            Err(_) => {
                let kp = crypto::generate_keypair();
                storage::save_keypair(&kp)?;
                kp
            }
        };

        // Load or create config
        let config = match storage::load_config() {
            Ok(cfg) => cfg,
            Err(_) => {
                let cfg = Config {
                    server_url: config::DEFAULT_SERVER_URL.to_string(),
                    polling_interval_secs: config::DEFAULT_POLLING_INTERVAL,
                };
                storage::save_config(&cfg)?;
                cfg
            }
        };

        // Load peers
        let peers = storage::load_peers().unwrap_or_default();

        // Initialize database
        let db_conn = storage::init_message_db()?;

        let mut app = Self {
            keypair,
            config: config.clone(),
            peers,
            messages: Vec::new(),
            db_conn,

            menu_state: MenuState::Closed,
            input_mode: InputMode::Normal,
            selected_peer_index: 0,

            message_input: String::new(),
            input_cursor: 0,
            show_slash_menu: false,
            slash_menu_index: 0,

            contact_import_input: String::new(),
            contact_export_name: String::new(),
            contact_export_json: String::new(),

            settings_selected_field: 0,
            settings_server_url: config.server_url.clone(),
            settings_polling_interval: config.polling_interval_secs.to_string(),
            settings_autostart_enabled: check_autostart_enabled(),

            status_message: String::new(),
            current_polling_interval: config.polling_interval_secs,

            chat_scroll_offset: 0,
            should_quit: false,
            polling_sender: None,
            keyboard_enhancements_supported: false, // Will be set by main.rs
        };

        // Load messages for the first peer if available
        if !app.peers.is_empty() {
            app.load_messages_for_selected_peer();
        }

        Ok(app)
    }

    /// Handle incoming events
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::NewMessage(message) => self.handle_new_message(message),
            AppEvent::Tick => {
                self.load_messages_for_selected_peer();
            }
            AppEvent::PollingIntervalUpdate(interval) => {
                self.current_polling_interval = interval;
            }
            AppEvent::Paste(text) => self.handle_paste(text),
        }
    }

    /// Handle paste event (drag-and-drop or cmd+v)
    fn handle_paste(&mut self, text: String) {
        // If we're in import mode and text looks like a file path, import it
        if self.menu_state == MenuState::ImportContact {
            let trimmed = text.trim();

            // Check if it looks like a file path (has .json extension or starts with file://)
            if trimmed.ends_with(".json") || trimmed.starts_with("file://") {
                // Clean up file:// prefix if present
                let path = if trimmed.starts_with("file://") {
                    trimmed.trim_start_matches("file://")
                } else {
                    trimmed
                };

                self.contact_import_input = path.to_string();
                self.status_message = "File path pasted - press Enter to import".to_string();
            } else {
                // Assume it's JSON content
                self.contact_import_input = text;
                self.status_message = "JSON pasted - press Enter to import".to_string();
            }
        } else {
            // In other modes, just append to current input
            match self.menu_state {
                MenuState::ExportContact => {
                    self.contact_export_name.push_str(&text);
                }
                _ => {
                    self.message_input.push_str(&text);
                }
            }
        }
    }

    /// Handle keyboard input
    fn handle_key(&mut self, key: KeyEvent) {
        // Global shortcuts (work in any mode)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('p') => {
                    self.handle_up();
                    return;
                }
                KeyCode::Char('n') => {
                    self.handle_down();
                    return;
                }
                _ => {}
            }
        }

        // Mode-specific handling
        match self.input_mode {
            InputMode::Normal => self.handle_key_normal(key),
            InputMode::Editing => self.handle_key_editing(key),
        }
    }

    /// Handle keyboard input in Normal mode (navigation)
    fn handle_key_normal(&mut self, key: KeyEvent) {
        // Handle view/command state
        match key.code {
            // Escape - always go back to chat
            KeyCode::Esc => {
                self.menu_state = MenuState::Closed;
                self.status_message = "".to_string();
            }

            // Slash commands (like Claude Code)
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Editing;
                self.message_input.push('/');
                self.input_cursor = self.message_input.chars().count();
                self.show_slash_menu = true;
                self.slash_menu_index = 0;
            }

            // Navigation: contacts view = switch peer, chat view = scroll, settings = field select
            KeyCode::Up if self.menu_state == MenuState::Contacts => {
                self.handle_up();
            }
            KeyCode::Down if self.menu_state == MenuState::Contacts => {
                self.handle_down();
            }
            KeyCode::Up if self.menu_state == MenuState::Settings => {
                if self.settings_selected_field > 0 {
                    self.settings_selected_field -= 1;
                }
            }
            KeyCode::Down if self.menu_state == MenuState::Settings => {
                if self.settings_selected_field < 2 {
                    self.settings_selected_field += 1;
                }
            }
            KeyCode::Enter if self.menu_state == MenuState::Settings => {
                self.submit_settings();
            }
            KeyCode::Up if self.menu_state == MenuState::Closed => {
                self.chat_scroll_offset = self.chat_scroll_offset.saturating_add(1);
            }
            KeyCode::Down if self.menu_state == MenuState::Closed => {
                self.chat_scroll_offset = self.chat_scroll_offset.saturating_sub(1);
            }
            KeyCode::Enter if self.menu_state == MenuState::Contacts => {
                // Select contact and return to chat
                if !self.peers.is_empty() && self.selected_peer_index < self.peers.len() {
                    self.menu_state = MenuState::Closed;
                    self.load_messages_for_selected_peer();
                }
            }

            // Start typing (only in chat view with contacts)
            KeyCode::Char(c) if self.menu_state == MenuState::Closed => {
                if !self.peers.is_empty() {
                    self.input_mode = InputMode::Editing;
                    self.handle_char_input(c);
                } else {
                    self.status_message = "No contacts - type /import to add one".to_string();
                }
            }

            _ => {}
        }
    }

    /// Handle keyboard input in Editing mode
    fn handle_key_editing(&mut self, key: KeyEvent) {
        // Special handling when slash menu is open
        if self.show_slash_menu && self.menu_state == MenuState::Closed {
            match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.clear_message_input();
                    self.show_slash_menu = false;
                    self.status_message = "".to_string();
                }
                KeyCode::Up => {
                    let commands = self.get_filtered_slash_commands();
                    if !commands.is_empty() && self.slash_menu_index > 0 {
                        self.slash_menu_index -= 1;
                    }
                }
                KeyCode::Down => {
                    let commands = self.get_filtered_slash_commands();
                    if self.slash_menu_index < commands.len().saturating_sub(1) {
                        self.slash_menu_index += 1;
                    }
                }
                KeyCode::Enter => {
                    let commands = self.get_filtered_slash_commands();
                    if let Some((cmd, _)) = commands.get(self.slash_menu_index) {
                        self.message_input = cmd.to_string();
                        self.show_slash_menu = false;
                        self.handle_submit();
                    }
                }
                KeyCode::Backspace => {
                    self.handle_backspace();
                    if self.message_input.is_empty() || !self.message_input.starts_with('/') {
                        self.show_slash_menu = false;
                    }
                    self.slash_menu_index = 0;
                }
                KeyCode::Char(c) => {
                    self.handle_char_input(c);
                    self.slash_menu_index = 0;
                }
                _ => {}
            }
            return;
        }

        // Normal editing mode
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.message_input.clear();
                self.input_cursor = 0;
                self.show_slash_menu = false;
                self.status_message = "".to_string();
            }

            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_char_input('\n');
            }

            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.handle_char_input('\n');
                } else {
                    self.handle_submit();
                }
            }

            KeyCode::Backspace => {
                self.handle_backspace();
            }

            KeyCode::Delete => {
                self.handle_delete();
            }

            KeyCode::Left => {
                if self.menu_state == MenuState::Closed {
                    self.input_cursor = self.input_cursor.saturating_sub(1);
                }
            }

            KeyCode::Right => {
                if self.menu_state == MenuState::Closed {
                    let max = self.message_input.chars().count();
                    if self.input_cursor < max {
                        self.input_cursor += 1;
                    }
                }
            }

            KeyCode::Home => {
                if self.menu_state == MenuState::Closed {
                    self.input_cursor = 0;
                }
            }

            KeyCode::End => {
                if self.menu_state == MenuState::Closed {
                    self.input_cursor = self.message_input.chars().count();
                }
            }

            KeyCode::Char(c) => {
                self.handle_char_input(c);
            }

            _ => {}
        }
    }

    /// Handle Up arrow key
    fn handle_up(&mut self) {
        // Navigate peer list
        if self.selected_peer_index > 0 {
            self.selected_peer_index -= 1;
            self.load_messages_for_selected_peer();
        }
    }

    /// Handle Down arrow key
    fn handle_down(&mut self) {
        // Navigate peer list
        if !self.peers.is_empty() && self.selected_peer_index < self.peers.len() - 1 {
            self.selected_peer_index += 1;
            self.load_messages_for_selected_peer();
        }
    }

    /// Handle submit action (Enter in editing mode)
    fn handle_submit(&mut self) {
        match self.menu_state {
            MenuState::Closed => {
                // Check if it's a slash command
                let input = self.message_input.trim().to_string();
                if input.starts_with('/') {
                    self.handle_slash_command(&input);
                } else {
                    self.submit_message();
                }
            }
            MenuState::ImportContact => self.import_contact(),
            MenuState::ExportContact => self.export_contact(),
            MenuState::Settings => self.submit_settings(),
            _ => {
                self.input_mode = InputMode::Normal;
            }
        }
    }

    /// Handle slash commands (Claude Code style)
    fn handle_slash_command(&mut self, command: &str) {
        match command {
            "/contacts" | "/c" => {
                self.menu_state = MenuState::Contacts;
                self.clear_message_input();
                self.input_mode = InputMode::Normal;
            }
            "/import" | "/i" => {
                self.menu_state = MenuState::ImportContact;
                self.contact_import_input.clear();
                self.clear_message_input();
                self.input_mode = InputMode::Editing;
            }
            "/export" | "/e" => {
                self.menu_state = MenuState::ExportContact;
                self.contact_export_name.clear();
                self.contact_export_json.clear();
                self.clear_message_input();
                self.input_mode = InputMode::Editing;
            }
            "/settings" | "/s" => {
                self.menu_state = MenuState::Settings;
                self.clear_message_input();
                self.input_mode = InputMode::Normal;
            }
            "/quit" | "/q" => {
                self.should_quit = true;
            }
            _ => {
                self.status_message = format!("Unknown command: {}", command);
                self.clear_message_input();
                self.input_mode = InputMode::Normal;
            }
        }
    }

    /// Submit message in Messages view
    fn submit_message(&mut self) {
        if self.message_input.trim().is_empty() {
            self.input_mode = InputMode::Normal;
            self.status_message = "".to_string();
            return;
        }

        if self.peers.is_empty() {
            self.input_mode = InputMode::Normal;
            self.clear_message_input();
            self.status_message = "No contacts - import one first".to_string();
            return;
        }

        let peer = match self.peers.get(self.selected_peer_index) {
            Some(p) => p.clone(),
            None => {
                self.input_mode = InputMode::Normal;
                self.status_message = "Invalid peer selection".to_string();
                return;
            }
        };

        let message_content = self.message_input.clone();

        // Send the message
        match self.send_message_to_peer(&peer, &message_content) {
            Ok(message_id) => {
                self.status_message = "Sent".to_string();
                self.clear_message_input();

                // Reload messages to show the sent message
                self.load_messages_for_selected_peer();

                // Reset polling interval - user is active
                self.current_polling_interval = 5; // show immediately, backend will confirm
                self.reset_polling_interval();

                crate::logger::log_to_file(&format!("Message sent: {}", message_id));
            }
            Err(e) => {
                self.status_message = format!("Send failed: {}", e);
                crate::logger::log_to_file(&format!("Failed to send message: {}", e));
            }
        }

        self.input_mode = InputMode::Normal;
    }

    /// Send a message to a peer
    fn send_message_to_peer(&self, peer: &Peer, plaintext: &str) -> Result<String, String> {
        // Parse recipient's public keys
        let recipient_encrypt_pk = crypto::from_hex(&peer.encrypt_pk)?;
        let _recipient_sign_pk = crypto::from_hex(&peer.sign_pk)?;

        // Create message payload
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let payload = serde_json::json!({
            "type": "text",
            "content": plaintext,
            "timestamp": timestamp,
            "sender_id": crypto::to_hex(&self.keypair.encrypt_pk),
        });

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize payload: {}", e))?;

        // Encrypt the message (includes sender's encrypt PK prepended for decryption)
        let mut message_to_sign = self.keypair.encrypt_pk.clone();
        let encrypted = crypto::encrypt_message(&payload_bytes, &recipient_encrypt_pk, &self.keypair.encrypt_sk)?;
        message_to_sign.extend(encrypted);

        // Sign the message with sender's signing key
        let signed = crypto::sign_message(&message_to_sign, &self.keypair.sign_sk)?;

        // Final format: [sender_sign_pk (32)] + [signed_message]
        let mut final_message = self.keypair.sign_pk.clone();
        final_message.extend(signed);

        // Encode to base64
        use base64::{Engine as _, engine::general_purpose};
        let encoded = general_purpose::STANDARD.encode(&final_message);

        // Send to recipient's mailbox queue (synchronous - we'll spawn a task)
        let server_url = self.config.server_url.clone();
        let queue_id = peer.queue_id.clone();
        let message_id = uuid::Uuid::new_v4().to_string();

        // Save to local database FIRST (synchronously)
        let local_message = storage::Message {
            id: message_id.clone(),
            queue_id: queue_id.clone(),
            sender: "You".to_string(),
            content: plaintext.to_string(),
            timestamp,
            msg_type: "text".to_string(),
            status: "sending".to_string(),
            is_outbound: true,
        };

        storage::save_message(&self.db_conn, &local_message)
            .map_err(|e| format!("Failed to save message locally: {}", e))?;

        // Then spawn async task to send to server
        let message_id_clone = message_id.clone();
        let db_conn_path = storage::get_app_data_dir()
            .map(|p| p.join("data/messages.db"))
            .map_err(|e| format!("Failed to get DB path: {}", e))?;

        tokio::spawn(async move {
            use crate::mailbox::{MailboxClient, MessageMeta};

            let mailbox_client = MailboxClient::new(server_url);
            match mailbox_client.send_message(&queue_id, encoded, MessageMeta {
                filename: None,
                size: None,
            }).await {
                Ok(server_msg_id) => {
                    crate::logger::log_to_file(&format!("Message posted to server: {}", server_msg_id));

                    // Update status to "sent"
                    if let Ok(conn) = rusqlite::Connection::open(&db_conn_path) {
                        let _ = conn.execute(
                            "UPDATE messages SET status = 'sent' WHERE id = ?1",
                            [&message_id_clone],
                        );
                    }
                }
                Err(e) => {
                    crate::logger::log_to_file(&format!("Failed to post message to server: {}", e));

                    // Update status to "failed"
                    if let Ok(conn) = rusqlite::Connection::open(&db_conn_path) {
                        let _ = conn.execute(
                            "UPDATE messages SET status = 'failed' WHERE id = ?1",
                            [&message_id_clone],
                        );
                    }
                }
            }
        });

        Ok(message_id)
    }


    /// Import a contact from JSON (or file path)
    fn import_contact(&mut self) {
        let input = self.contact_import_input.trim();

        if input.is_empty() {
            self.status_message = "Empty input".to_string();
            self.input_mode = InputMode::Normal;
            return;
        }

        // Try to parse as JSON first, if that fails try as file path
        let json_str = if input.starts_with('{') {
            // Looks like JSON
            input.to_string()
        } else {
            // Assume it's a file path
            let file_path = if input.starts_with('/') || input.starts_with('~') {
                // Absolute path
                std::path::PathBuf::from(shellexpand::tilde(input).to_string())
            } else {
                // Relative to app data directory
                match storage::get_app_data_dir() {
                    Ok(dir) => dir.join(input),
                    Err(e) => {
                        self.status_message = format!("Failed to get data dir: {}", e);
                        self.input_mode = InputMode::Normal;
                        return;
                    }
                }
            };

            match std::fs::read_to_string(&file_path) {
                Ok(contents) => contents,
                Err(e) => {
                    self.status_message = format!("Failed to read file: {}", e);
                    self.input_mode = InputMode::Normal;
                    return;
                }
            }
        };

        // Parse JSON
        let contact_data: serde_json::Value = match serde_json::from_str(&json_str) {
            Ok(data) => data,
            Err(e) => {
                self.status_message = format!("Invalid JSON: {}", e);
                self.input_mode = InputMode::Normal;
                return;
            }
        };

        // Extract fields
        let name = match contact_data["name"].as_str() {
            Some(n) => n.to_string(),
            None => {
                self.status_message = "✗ Missing 'name' field".to_string();
                self.input_mode = InputMode::Normal;
                return;
            }
        };

        let encrypt_pk = match contact_data["encrypt_pk"].as_str() {
            Some(pk) => pk.to_string(),
            None => {
                self.status_message = "✗ Missing 'encrypt_pk' field".to_string();
                self.input_mode = InputMode::Normal;
                return;
            }
        };

        let sign_pk = match contact_data["sign_pk"].as_str() {
            Some(pk) => pk.to_string(),
            None => {
                self.status_message = "✗ Missing 'sign_pk' field".to_string();
                self.input_mode = InputMode::Normal;
                return;
            }
        };

        // Validate hex format
        if let Err(e) = crypto::from_hex(&encrypt_pk) {
            self.status_message = format!("✗ Invalid encrypt_pk: {}", e);
            self.input_mode = InputMode::Normal;
            return;
        }

        if let Err(e) = crypto::from_hex(&sign_pk) {
            self.status_message = format!("✗ Invalid sign_pk: {}", e);
            self.input_mode = InputMode::Normal;
            return;
        }

        // Check if trying to import own contact
        let my_encrypt_pk = crypto::to_hex(&self.keypair.encrypt_pk);
        if encrypt_pk == my_encrypt_pk {
            self.status_message = "Cannot import your own contact".to_string();
            self.input_mode = InputMode::Normal;
            self.contact_import_input.clear();
            return;
        }

        // Check for duplicates
        if self.peers.iter().any(|p| p.encrypt_pk == encrypt_pk) {
            self.status_message = "Contact already exists".to_string();
            self.input_mode = InputMode::Normal;
            self.contact_import_input.clear();
            return;
        }

        // Generate deterministic queue_id
        let my_encrypt_pk_hex = crypto::to_hex(&self.keypair.encrypt_pk);
        let queue_id = match crypto::generate_conversation_queue_id(&my_encrypt_pk_hex, &encrypt_pk) {
            Ok(qid) => qid,
            Err(e) => {
                self.status_message = format!("✗ Failed to generate queue_id: {}", e);
                self.input_mode = InputMode::Normal;
                return;
            }
        };

        // Create and save peer
        let peer = Peer {
            name: name.clone(),
            encrypt_pk,
            sign_pk,
            queue_id: queue_id.clone(),
        };

        match storage::save_peer(&peer) {
            Ok(_) => {
                self.peers.push(peer);
                self.status_message = format!("Contact '{}' imported", name);
                self.contact_import_input.clear();
                self.menu_state = MenuState::Closed;
                crate::logger::log_to_file(&format!("Contact imported: {} ({})", name, queue_id));
            }
            Err(e) => {
                self.status_message = format!("Import failed: {}", e);
            }
        }

        self.input_mode = InputMode::Normal;
    }

    /// Export contact info as JSON to file
    fn export_contact(&mut self) {
        let name = self.contact_export_name.trim();

        if name.is_empty() {
            self.status_message = "Name cannot be empty".to_string();
            return;
        }

        let contact_json = serde_json::json!({
            "name": name,
            "encrypt_pk": crypto::to_hex(&self.keypair.encrypt_pk),
            "sign_pk": crypto::to_hex(&self.keypair.sign_pk),
        });

        let json_string = serde_json::to_string_pretty(&contact_json)
            .unwrap_or_else(|_| "Error generating JSON".to_string());

        // Save to Downloads folder
        if let Some(home_dir) = dirs::home_dir() {
            let downloads_dir = home_dir.join("Downloads");
            let file_path = downloads_dir.join(format!("contact-{}.json", name.replace(" ", "-")));

            match std::fs::write(&file_path, &json_string) {
                Ok(_) => {
                    self.status_message = format!("✓ Saved to Downloads: contact-{}.json", name.replace(" ", "-"));
                    self.contact_export_json = json_string;
                    self.input_mode = InputMode::Normal;

                    crate::logger::log_to_file(&format!("Contact exported to: {}", file_path.display()));
                }
                Err(e) => {
                    self.status_message = format!("Failed to write file: {}", e);
                    crate::logger::log_to_file(&format!("Export failed: {}", e));
                }
            }
        } else {
            self.status_message = "Failed to find Downloads folder".to_string();
        }
    }

    /// Submit settings changes
    fn submit_settings(&mut self) {
        // Field 2 = "Start at Login" toggle (not a text field)
        if self.settings_selected_field == 2 {
            let now_enabled = toggle_autostart();
            self.settings_autostart_enabled = check_autostart_enabled();
            if now_enabled {
                self.status_message = "✓ Daemon will start at login".to_string();
            } else {
                self.status_message = "✓ Autostart disabled".to_string();
            }
            return;
        }

        // Validate and save settings
        let new_url = self.settings_server_url.trim();
        let new_interval_str = self.settings_polling_interval.trim();

        // Validate URL (basic check)
        if !new_url.starts_with("http://") && !new_url.starts_with("https://") {
            self.status_message = "✗ Invalid URL (must start with http:// or https://)".to_string();
            self.input_mode = InputMode::Normal;
            return;
        }

        // Validate interval
        let new_interval = match new_interval_str.parse::<u64>() {
            Ok(val) if val > 0 => val,
            _ => {
                self.status_message = "✗ Invalid interval (must be positive number)".to_string();
                self.input_mode = InputMode::Normal;
                return;
            }
        };

        // Update config
        self.config.server_url = new_url.to_string();
        self.config.polling_interval_secs = new_interval;

        // Save to file
        match storage::save_config(&self.config) {
            Ok(_) => {
                self.status_message = "Settings saved (restart to apply)".to_string();
                crate::logger::log_to_file(&format!("Settings saved: URL={}, Interval={}s", new_url, new_interval));
            }
            Err(e) => {
                self.status_message = format!("Save failed: {}", e);
                crate::logger::log_to_file(&format!("Failed to save config: {}", e));
            }
        }

        self.input_mode = InputMode::Normal;
    }

    /// Handle backspace in editing mode
    fn handle_backspace(&mut self) {
        match self.menu_state {
            MenuState::Closed => {
                if self.input_cursor > 0 {
                    let byte_pos = char_to_byte_index(&self.message_input, self.input_cursor - 1);
                    let next_byte = char_to_byte_index(&self.message_input, self.input_cursor);
                    self.message_input.drain(byte_pos..next_byte);
                    self.input_cursor -= 1;
                }
            }
            MenuState::ImportContact => { self.contact_import_input.pop(); }
            MenuState::ExportContact => { self.contact_export_name.pop(); }
            MenuState::Settings => {
                match self.settings_selected_field {
                    0 => { self.settings_server_url.pop(); }
                    1 => { self.settings_polling_interval.pop(); }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Delete character at cursor (Delete key)
    fn handle_delete(&mut self) {
        if self.menu_state == MenuState::Closed {
            let max = self.message_input.chars().count();
            if self.input_cursor < max {
                let byte_pos = char_to_byte_index(&self.message_input, self.input_cursor);
                let next_byte = char_to_byte_index(&self.message_input, self.input_cursor + 1);
                self.message_input.drain(byte_pos..next_byte);
            }
        }
    }

    /// Handle character input in editing mode
    fn handle_char_input(&mut self, c: char) {
        match self.menu_state {
            MenuState::Closed => {
                let byte_pos = char_to_byte_index(&self.message_input, self.input_cursor);
                self.message_input.insert(byte_pos, c);
                self.input_cursor += 1;
            }
            MenuState::ImportContact => { self.contact_import_input.push(c); }
            MenuState::ExportContact => { self.contact_export_name.push(c); }
            MenuState::Settings => {
                match self.settings_selected_field {
                    0 => { self.settings_server_url.push(c); }
                    1 => { self.settings_polling_interval.push(c); }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Handle new message received from polling service
    fn handle_new_message(&mut self, message: Message) {
        // Save to database (already done by polling service)
        // Reload messages if viewing this conversation
        if let Some(peer) = self.peers.get(self.selected_peer_index) {
            if peer.queue_id == message.queue_id {
                self.load_messages_for_selected_peer();
            }
        }

        self.status_message = format!("← {}", message.sender);
    }

    /// Load messages for the currently selected peer
    fn load_messages_for_selected_peer(&mut self) {
        if let Some(peer) = self.peers.get(self.selected_peer_index) {
            match storage::load_messages_for_queue(&self.db_conn, &peer.queue_id) {
                Ok(messages) => {
                    self.messages = messages;
                    self.chat_scroll_offset = 0;
                    self.status_message = "".to_string();
                }
                Err(e) => {
                    self.status_message = format!("Load error: {}", e);
                }
            }
        }
    }

    /// Clear message input and reset cursor
    fn clear_message_input(&mut self) {
        self.message_input.clear();
        self.input_cursor = 0;
    }

    /// Set the polling command sender
    pub fn set_polling_sender(&mut self, sender: tokio::sync::mpsc::UnboundedSender<crate::backend::PollingCommand>) {
        self.polling_sender = Some(sender);
    }

    /// Reset polling interval to minimum (user is active)
    fn reset_polling_interval(&self) {
        if let Some(sender) = &self.polling_sender {
            let _ = sender.send(crate::backend::PollingCommand::ResetInterval);
        }
    }

    /// Get available slash commands filtered by current input
    pub fn get_filtered_slash_commands(&self) -> Vec<(&'static str, &'static str)> {
        let all_commands = vec![
            ("/import", "Import a contact from JSON"),
            ("/export", "Export your contact info as JSON"),
            ("/contacts", "View all contacts"),
            ("/settings", "View settings"),
            ("/quit", "Quit application"),
        ];

        let filter = self.message_input.trim().to_lowercase();

        if filter == "/" {
            // Show all commands
            all_commands
        } else {
            // Filter by what's typed
            all_commands.into_iter()
                .filter(|(cmd, _)| cmd.starts_with(&filter))
                .collect()
        }
    }
}

/// Convert a char index to a byte index in a UTF-8 string.
pub fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Build the auto-launch handle for the daemon
fn make_auto_launch() -> Option<auto_launch::AutoLaunch> {
    // Find the daemon binary next to the current executable
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let daemon = dir.join("trassenger-daemon");
    let daemon_str = daemon.to_string_lossy().to_string();

    auto_launch::AutoLaunchBuilder::new()
        .set_app_name("Trassenger Daemon")
        .set_app_path(&daemon_str)
        .set_args(&["--daemon"])
        .build()
        .ok()
}

/// Check if the daemon is configured for autostart
pub fn check_autostart_enabled() -> bool {
    make_auto_launch()
        .and_then(|al| al.is_enabled().ok())
        .unwrap_or(false)
}

/// Toggle the daemon autostart setting
pub fn toggle_autostart() -> bool {
    if let Some(al) = make_auto_launch() {
        let enabled = al.is_enabled().unwrap_or(false);
        let result = if enabled { al.disable() } else { al.enable() };
        result.is_ok() && !enabled
    } else {
        false
    }
}
