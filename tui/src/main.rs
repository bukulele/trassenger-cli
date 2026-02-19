// Trassenger TUI - Terminal-based encrypted messenger
mod event;
mod app;
mod ui;
mod ipc;

// Re-export shared modules from lib so crate:: references in submodules resolve
pub(crate) use trassenger_lib::logger;
pub(crate) use trassenger_lib::storage;
pub(crate) use trassenger_lib::config;

use app::App;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, DisableBracketedPaste, EnableBracketedPaste,
        KeyboardEnhancementFlags, PushKeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::EventHandler;
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger (no console output)
    logger::init_logger()?;

    // Initialize storage directories (needed for socket path resolution)
    if let Err(e) = storage::init_storage() {
        eprintln!("Failed to initialize storage: {}", e);
        std::process::exit(1);
    }

    // Setup event handler first (we need the sender for IPC)
    let mut event_handler = EventHandler::new();
    event_handler.spawn_keyboard_listener();

    // Connect to daemon
    let daemon_client = match ipc::DaemonClient::connect(event_handler.sender()).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("Please start the Trassenger daemon first.");
            std::process::exit(1);
        }
    };

    logger::log_to_file("Connected to daemon");

    // Initialize application state (loads from daemon)
    let mut app = match App::initialize(daemon_client).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to initialize app: {}", e);
            std::process::exit(1);
        }
    };

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();

    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;

    // Try keyboard enhancements (modern terminals only)
    let keyboard_enhancements_supported = execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        )
    ).is_ok();

    if !keyboard_enhancements_supported {
        logger::log_to_file("Keyboard enhancements not supported, using fallback keys (Ctrl+J for newline)");
    }

    app.keyboard_enhancements_supported = keyboard_enhancements_supported;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Main event loop
    let result = run_app(&mut terminal, &mut app, &mut event_handler).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
        PopKeyboardEnhancementFlags
    )?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        logger::log_to_file(&format!("Error: {:?}", err));
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    event_handler: &mut EventHandler,
) -> io::Result<()> {
    loop {
        // Drain any pending daemon responses (LoadMessages, LoadPeers, etc.)
        for ev in app.drain_daemon_events() {
            app.handle_daemon_event(ev);
        }

        // Draw UI
        terminal.draw(|f| {
            render_ui(f, app);
        })?;

        // Handle events
        if let Some(event) = event_handler.next().await {
            app.handle_event(event);
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn render_ui(f: &mut ratatui::Frame, app: &App) {
    use ratatui::{
        layout::{Constraint, Direction, Layout},
    };

    let terminal_width = f.area().width as usize;
    let input_height = if app.show_slash_menu && app.menu_state == app::MenuState::Closed {
        let commands = app.get_filtered_slash_commands();
        (commands.len() + 2) as u16
    } else {
        let content_width = terminal_width.saturating_sub(2).max(1);
        let input_text = match app.menu_state {
            app::MenuState::ImportContact => &app.contact_import_input,
            app::MenuState::ExportContact => &app.contact_export_name,
            _ => &app.message_input,
        };
        let text_lines: u16 = input_text.split('\n').map(|seg| {
            let chars = seg.chars().count();
            ((chars + content_width - 1) / content_width).max(1) as u16
        }).sum();
        let text_lines = text_lines.max(2);
        let max_input = f.area().height / 3;
        (text_lines + 2).min(max_input)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(input_height),
            Constraint::Length(2),
        ])
        .split(f.area());

    match app.menu_state {
        app::MenuState::Closed => {
            ui::render_message_list(f, app, chunks[0]);
            ui::render_input_area(f, app, chunks[1]);
        }
        app::MenuState::Contacts => {
            ui::render_contacts_view(f, app, chunks[0]);
            ui::render_view_hints(f, "Esc to return to chat", chunks[1]);
        }
        app::MenuState::ImportContact => {
            ui::render_import_view(f, app, chunks[0]);
            ui::render_input_area(f, app, chunks[1]);
        }
        app::MenuState::ExportContact => {
            ui::render_export_view(f, app, chunks[0]);
            ui::render_input_area(f, app, chunks[1]);
        }
        app::MenuState::Settings => {
            ui::render_settings_view(f, app, chunks[0]);
            ui::render_view_hints(f, "Esc to return to chat", chunks[1]);
        }
    }

    ui::render_hints(f, app, chunks[2]);
}
