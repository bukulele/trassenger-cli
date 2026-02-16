// Trassenger TUI - Terminal-based encrypted messenger
mod crypto;
mod storage;
mod mailbox;
mod config;
mod event;
mod app;
mod backend;
mod ui;
mod logger;

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
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger (no console output)
    logger::init_logger()?;

    // Initialize application state
    let mut app = App::initialize().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();

    // Basic terminal setup (works everywhere)
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;

    // Try keyboard enhancements (modern terminals only - gracefully fail on old Windows)
    let keyboard_enhancements_supported = execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        )
    ).is_ok();

    if !keyboard_enhancements_supported {
        logger::log_to_file("Keyboard enhancements not supported, using fallback keys (Ctrl+J for newline)");
    }

    // Tell app about keyboard enhancement support
    app.keyboard_enhancements_supported = keyboard_enhancements_supported;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Setup event handler
    let mut event_handler = EventHandler::new();
    event_handler.spawn_keyboard_listener();
    event_handler.spawn_tick_timer(Duration::from_millis(250));

    // Start polling service
    let (polling_service, polling_cmd_sender) = backend::PollingService::new(
        app.config.server_url.clone(),
        app.keypair.encrypt_sk.clone(),
        app.keypair.sign_pk.clone(),
        event_handler.sender(),
    );
    polling_service.start();

    // Give app access to polling command sender
    app.set_polling_sender(polling_cmd_sender);

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
        // Draw UI
        terminal.draw(|f| {
            render_ui(f, app);
        })?;

        // Handle events
        if let Some(event) = event_handler.next().await {
            app.handle_event(event);
        }

        // Check if should quit
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

    // Calculate input area height dynamically for slash menu
    let input_height = if app.show_slash_menu && app.menu_state == app::MenuState::Closed {
        // Slash menu: 5 commands + separator + input line = 7 lines
        7
    } else {
        // Normal: separator + prompt line = 2 lines (+ 1 for padding)
        3
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),              // Main content (fills remaining space)
            Constraint::Length(input_height), // Input area (dynamic height)
            Constraint::Length(2),           // Hints (2 lines of text)
        ])
        .split(f.area());

    // Render different views based on state
    match app.menu_state {
        app::MenuState::Closed => {
            // Normal chat view
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

    // Hints (always at bottom)
    ui::render_hints(f, app, chunks[2]);
}
