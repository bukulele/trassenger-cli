use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc;

/// Application events
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Keyboard input event
    Key(KeyEvent),
    /// New message received from polling service
    NewMessage(crate::storage::Message),
    /// Polling interval updated (adaptive polling)
    PollingIntervalUpdate(u64),
    /// Paste event (for drag-and-drop file paths)
    Paste(String),
}

/// Event handler for the TUI application
pub struct EventHandler {
    sender: mpsc::UnboundedSender<AppEvent>,
    receiver: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventHandler {
    /// Create a new event handler
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self { sender, receiver }
    }

    /// Get a clone of the sender for other components
    pub fn sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.sender.clone()
    }

    /// Receive the next event (blocking)
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.receiver.recv().await
    }

    /// Spawn the keyboard event listener task
    pub fn spawn_keyboard_listener(&self) {
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let mut reader = event::EventStream::new();
            loop {
                let event = reader.next().fuse();
                tokio::select! {
                    maybe_event = event => {
                        match maybe_event {
                            Some(Ok(CrosstermEvent::Key(key))) => {
                                // Filter out key release events (Windows sends both press and release)
                                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                                    // Ctrl+V or Ctrl+Shift+V: read clipboard and emit as Paste
                                    // (Windows Terminal doesn't support bracketed paste)
                                    let is_ctrl_v = key.modifiers.contains(KeyModifiers::CONTROL)
                                        && key.code == KeyCode::Char('v');
                                    if is_ctrl_v {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            if let Ok(text) = clipboard.get_text() {
                                                let _ = sender.send(AppEvent::Paste(text));
                                                continue;
                                            }
                                        }
                                        // Clipboard unavailable — fall through and let key pass
                                    }
                                    if sender.send(AppEvent::Key(key)).is_err() {
                                        break; // Channel closed, stop listener
                                    }
                                }
                            }
                            Some(Ok(CrosstermEvent::Paste(text))) => {
                                if sender.send(AppEvent::Paste(text)).is_err() {
                                    break;
                                }
                            }
                            Some(Ok(_)) => {
                                // Ignore other events (mouse, resize, etc.)
                            }
                            Some(Err(e)) => {
                                crate::logger::log_to_file(&format!("Keyboard event error: {}", e));
                            }
                            None => break,
                        }
                    }
                }
            }
        });
    }

}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}
