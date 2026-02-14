use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use futures::{FutureExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;

/// Application events
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Keyboard input event
    Key(KeyEvent),
    /// New message received from polling service
    NewMessage(crate::storage::Message),
    /// Periodic tick for UI refresh
    Tick,
    /// Polling interval updated (adaptive polling)
    PollingIntervalUpdate(u64),
    /// Reset polling interval to minimum (user is active)
    ResetPollingInterval,
    /// Paste event (for drag-and-drop file paths)
    Paste(String),
    /// Application should quit
    Quit,
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
                                if sender.send(AppEvent::Key(key)).is_err() {
                                    break; // Channel closed, stop listener
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

    /// Spawn the tick timer task for periodic UI refresh
    pub fn spawn_tick_timer(&self, tick_rate: Duration) {
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_rate);
            loop {
                interval.tick().await;
                if sender.send(AppEvent::Tick).is_err() {
                    break; // Channel closed, stop timer
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
