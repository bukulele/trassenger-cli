use crate::ipc::{DaemonClient, DaemonEvent};
use crate::storage::{Config, Message, Peer};
use crate::event::AppEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Command/view state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuState {
    Closed,
    Contacts,
    ImportContact,
    ExportContact,
    Settings,
}

/// Input mode for text editing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Editing,
}

/// Main application state
pub struct App {
    /// Connection to daemon
    daemon: DaemonClient,

    /// Application configuration (cached locally for display)
    pub config: Config,
    /// List of contacts/peers
    pub peers: Vec<Peer>,
    /// Messages for current conversation
    pub messages: Vec<Message>,

    // Navigation state
    pub menu_state: MenuState,
    pub input_mode: InputMode,
    pub selected_peer_index: usize,

    // Message input
    pub message_input: String,
    pub input_cursor: usize,
    pub show_slash_menu: bool,
    pub slash_menu_index: usize,

    // Contact import/export
    pub contact_import_input: String,
    pub contact_export_name: String,
    pub contact_export_json: String,

    // Settings (cached for display)
    pub settings_selected_field: usize,
    pub settings_server_url: String,
    pub settings_polling_interval: String,
    pub settings_autostart_enabled: bool,

    // Status
    pub status_message: String,
    pub current_polling_interval: u64,

    pub chat_scroll_offset: usize,
    pub should_quit: bool,

    pub keyboard_enhancements_supported: bool,
}

impl App {
    /// Initialize the application by loading state from daemon
    pub async fn initialize(mut daemon: DaemonClient) -> Result<Self, String> {
        // Load peers
        daemon.load_peers();
        let peers = loop {
            match daemon.try_recv_all().into_iter().find_map(|ev| {
                if let DaemonEvent::Peers { peers } = ev { Some(peers) } else { None }
            }) {
                Some(p) => break p,
                None => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    daemon.load_peers();
                }
            }
        };

        // Load config (from disk directly - TUI still reads config for display)
        let config = crate::storage::load_config().unwrap_or_else(|_| Config {
            server_url: crate::config::DEFAULT_SERVER_URL.to_string(),
            polling_interval_secs: crate::config::DEFAULT_POLLING_INTERVAL,
        });

        let mut app = Self {
            daemon,
            config: config.clone(),
            peers,
            messages: Vec::new(),

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
            keyboard_enhancements_supported: false,
        };

        // Load messages for first peer
        if !app.peers.is_empty() {
            app.load_messages_for_selected_peer();
        }

        Ok(app)
    }

    /// Drain pending daemon response events (called every frame before rendering)
    pub fn drain_daemon_events(&mut self) -> Vec<DaemonEvent> {
        self.daemon.try_recv_all()
    }

    /// Handle a daemon event received between frames
    pub fn handle_daemon_event(&mut self, ev: DaemonEvent) {
        match ev {
            DaemonEvent::Messages { queue_id, messages } => {
                if let Some(peer) = self.peers.get(self.selected_peer_index) {
                    if peer.queue_id == queue_id {
                        self.messages = messages;
                    }
                }
            }
            DaemonEvent::Peers { peers } => {
                self.peers = peers;
            }
            DaemonEvent::ContactImported { peer } => {
                if !self.peers.iter().any(|p| p.encrypt_pk == peer.encrypt_pk) {
                    self.peers.push(peer.clone());
                }
                self.status_message = format!("Contact '{}' imported", peer.name);
                self.contact_import_input.clear();
                self.menu_state = MenuState::Closed;
                self.input_mode = InputMode::Normal;
            }
            DaemonEvent::ContactExported { json } => {
                self.contact_export_json = json;
                let name = self.contact_export_name.trim().replace(' ', "-");
                self.status_message = format!("Saved to Downloads: contact-{}.json", name);
                self.input_mode = InputMode::Normal;
            }
            DaemonEvent::MessageSent => {
                self.load_messages_for_selected_peer();
            }
            DaemonEvent::PollingInterval { secs } => {
                self.current_polling_interval = secs;
            }
            DaemonEvent::Error { message } => {
                self.status_message = format!("Error: {}", message);
                self.input_mode = InputMode::Normal;
            }
            DaemonEvent::NewMessage { message } => {
                // Reload if we're viewing this conversation
                if let Some(peer) = self.peers.get(self.selected_peer_index) {
                    if peer.queue_id == message.queue_id {
                        self.load_messages_for_selected_peer();
                    }
                }
                self.status_message = format!("← {}", message.sender);
            }
        }
    }

    /// Handle incoming AppEvents
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::NewMessage(message) => {
                if let Some(peer) = self.peers.get(self.selected_peer_index) {
                    if peer.queue_id == message.queue_id {
                        self.load_messages_for_selected_peer();
                    }
                }
                self.status_message = format!("← {}", message.sender);
            }
            AppEvent::PollingIntervalUpdate(interval) => {
                self.current_polling_interval = interval;
            }
            AppEvent::Paste(text) => self.handle_paste(text),
        }
    }

    fn handle_paste(&mut self, text: String) {
        if self.menu_state == MenuState::ImportContact {
            let trimmed = text.trim();
            if trimmed.ends_with(".json") || trimmed.starts_with("file://") {
                let path = trimmed.trim_start_matches("file://");
                self.contact_import_input = path.to_string();
                self.status_message = "File path pasted - press Enter to import".to_string();
            } else {
                self.contact_import_input = text;
                self.status_message = "JSON pasted - press Enter to import".to_string();
            }
        } else {
            match self.menu_state {
                MenuState::ExportContact => { self.contact_export_name.push_str(&text); }
                _ => { self.message_input.push_str(&text); }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('p') => { self.handle_up(); return; }
                KeyCode::Char('n') => { self.handle_down(); return; }
                _ => {}
            }
        }

        match self.input_mode {
            InputMode::Normal => self.handle_key_normal(key),
            InputMode::Editing => self.handle_key_editing(key),
        }
    }

    fn handle_key_normal(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.menu_state = MenuState::Closed;
                self.status_message.clear();
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Editing;
                self.message_input.push('/');
                self.input_cursor = self.message_input.chars().count();
                self.show_slash_menu = true;
                self.slash_menu_index = 0;
            }
            KeyCode::Up if self.menu_state == MenuState::Contacts => self.handle_up(),
            KeyCode::Down if self.menu_state == MenuState::Contacts => self.handle_down(),
            KeyCode::Up if self.menu_state == MenuState::Settings => {
                if self.settings_selected_field > 0 { self.settings_selected_field -= 1; }
            }
            KeyCode::Down if self.menu_state == MenuState::Settings => {
                if self.settings_selected_field < 2 { self.settings_selected_field += 1; }
            }
            KeyCode::Enter if self.menu_state == MenuState::Settings => self.submit_settings(),
            KeyCode::Up if self.menu_state == MenuState::Closed => {
                self.chat_scroll_offset = self.chat_scroll_offset.saturating_add(1);
            }
            KeyCode::Down if self.menu_state == MenuState::Closed => {
                self.chat_scroll_offset = self.chat_scroll_offset.saturating_sub(1);
            }
            KeyCode::Enter if self.menu_state == MenuState::Contacts => {
                if !self.peers.is_empty() && self.selected_peer_index < self.peers.len() {
                    self.menu_state = MenuState::Closed;
                    self.load_messages_for_selected_peer();
                }
            }
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

    fn handle_key_editing(&mut self, key: KeyEvent) {
        if self.show_slash_menu && self.menu_state == MenuState::Closed {
            match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.clear_message_input();
                    self.show_slash_menu = false;
                    self.status_message.clear();
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

        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.message_input.clear();
                self.input_cursor = 0;
                self.show_slash_menu = false;
                self.status_message.clear();
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
            KeyCode::Backspace => self.handle_backspace(),
            KeyCode::Delete => self.handle_delete(),
            KeyCode::Left if self.menu_state == MenuState::Closed => {
                self.input_cursor = self.input_cursor.saturating_sub(1);
            }
            KeyCode::Right if self.menu_state == MenuState::Closed => {
                let max = self.message_input.chars().count();
                if self.input_cursor < max { self.input_cursor += 1; }
            }
            KeyCode::Home if self.menu_state == MenuState::Closed => {
                self.input_cursor = 0;
            }
            KeyCode::End if self.menu_state == MenuState::Closed => {
                self.input_cursor = self.message_input.chars().count();
            }
            KeyCode::Char(c) => self.handle_char_input(c),
            _ => {}
        }
    }

    fn handle_up(&mut self) {
        if self.selected_peer_index > 0 {
            self.selected_peer_index -= 1;
            self.load_messages_for_selected_peer();
        }
    }

    fn handle_down(&mut self) {
        if !self.peers.is_empty() && self.selected_peer_index < self.peers.len() - 1 {
            self.selected_peer_index += 1;
            self.load_messages_for_selected_peer();
        }
    }

    fn handle_submit(&mut self) {
        match self.menu_state {
            MenuState::Closed => {
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
            _ => { self.input_mode = InputMode::Normal; }
        }
    }

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
            "/quit" | "/q" => { self.should_quit = true; }
            _ => {
                self.status_message = format!("Unknown command: {}", command);
                self.clear_message_input();
                self.input_mode = InputMode::Normal;
            }
        }
    }

    fn submit_message(&mut self) {
        if self.message_input.trim().is_empty() {
            self.input_mode = InputMode::Normal;
            self.status_message.clear();
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

        let plaintext = self.message_input.clone();

        // Send command to daemon (async, non-blocking)
        self.daemon.send_message(&peer.queue_id, &plaintext, &peer.encrypt_pk);
        self.status_message = "Sending...".to_string();
        self.clear_message_input();
        self.input_mode = InputMode::Normal;

        // Tell daemon to reset polling interval (user is active)
        self.daemon.reset_polling_interval();
    }

    fn import_contact(&mut self) {
        let input = self.contact_import_input.trim().to_string();

        if input.is_empty() {
            self.status_message = "Empty input".to_string();
            self.input_mode = InputMode::Normal;
            return;
        }

        // Try to parse as JSON or file path
        let json_str = if input.starts_with('{') {
            input.clone()
        } else {
            let file_path = if input.starts_with('/') || input.starts_with('~') {
                std::path::PathBuf::from(shellexpand::tilde(&input).to_string())
            } else {
                match crate::storage::get_app_data_dir() {
                    Ok(dir) => dir.join(&input),
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

        // Send to daemon — response handled in handle_daemon_event
        self.daemon.import_contact(&json_str);
        self.status_message = "Importing...".to_string();
        self.input_mode = InputMode::Normal;
    }

    fn export_contact(&mut self) {
        let name = self.contact_export_name.trim().to_string();

        if name.is_empty() {
            self.status_message = "Name cannot be empty".to_string();
            return;
        }

        self.daemon.export_contact(&name);
        self.status_message = "Exporting...".to_string();
        self.input_mode = InputMode::Normal;
    }

    fn submit_settings(&mut self) {
        if self.settings_selected_field == 2 {
            let now_enabled = toggle_autostart();
            self.settings_autostart_enabled = check_autostart_enabled();
            if now_enabled {
                self.status_message = "Daemon will start at login".to_string();
            } else {
                self.status_message = "Autostart disabled".to_string();
            }
            return;
        }

        let new_url = self.settings_server_url.trim().to_string();
        let new_interval_str = self.settings_polling_interval.trim().to_string();

        if !new_url.starts_with("http://") && !new_url.starts_with("https://") {
            self.status_message = "Invalid URL (must start with http:// or https://)".to_string();
            self.input_mode = InputMode::Normal;
            return;
        }

        let new_interval = match new_interval_str.parse::<u64>() {
            Ok(val) if val > 0 => val,
            _ => {
                self.status_message = "Invalid interval (must be positive number)".to_string();
                self.input_mode = InputMode::Normal;
                return;
            }
        };

        self.config.server_url = new_url.clone();
        self.config.polling_interval_secs = new_interval;

        // Send to daemon
        self.daemon.update_config(&new_url, new_interval);
        self.status_message = "Settings saved (restart daemon to apply)".to_string();
        self.input_mode = InputMode::Normal;
    }

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

    /// Request messages reload from daemon
    fn load_messages_for_selected_peer(&mut self) {
        if let Some(peer) = self.peers.get(self.selected_peer_index) {
            self.daemon.load_messages(&peer.queue_id);
        }
    }

    fn clear_message_input(&mut self) {
        self.message_input.clear();
        self.input_cursor = 0;
    }

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
            all_commands
        } else {
            all_commands.into_iter()
                .filter(|(cmd, _)| cmd.starts_with(&filter))
                .collect()
        }
    }
}

pub fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn make_auto_launch() -> Option<auto_launch::AutoLaunch> {
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

pub fn check_autostart_enabled() -> bool {
    make_auto_launch()
        .and_then(|al| al.is_enabled().ok())
        .unwrap_or(false)
}

pub fn toggle_autostart() -> bool {
    if let Some(al) = make_auto_launch() {
        let enabled = al.is_enabled().unwrap_or(false);
        let result = if enabled { al.disable() } else { al.enable() };
        result.is_ok() && !enabled
    } else {
        false
    }
}
