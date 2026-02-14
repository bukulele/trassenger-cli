use crate::app::{App, InputMode, MenuState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

/// Render the message list (chronological dialog)
pub fn render_message_list(f: &mut Frame, app: &App, area: Rect) {
    // If viewing a contact, show their name at top
    if !app.peers.is_empty() && app.selected_peer_index < app.peers.len() {
        let peer = &app.peers[app.selected_peer_index];

        // Render header with contact name - clear visual indicator
        let header = Line::from(vec![
            Span::styled("Chat: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&peer.name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]);

        let header_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        f.render_widget(Paragraph::new(header), header_area);

        // Render messages below header
        let message_area = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: area.height.saturating_sub(2)
        };

        render_messages_content(f, app, message_area);
    } else {
        // No contacts - show empty state
        render_empty_state(f, area);
    }
}

/// Render actual message content
fn render_messages_content(f: &mut Frame, app: &App, area: Rect) {
    if app.messages.is_empty() {
        let empty = Line::from(Span::styled(
            "No messages yet. Press Enter to start typing.",
            Style::default().fg(Color::DarkGray)
        ));
        f.render_widget(Paragraph::new(empty), area);
        return;
    }

    let messages: Vec<Line> = app.messages.iter().map(|msg| {
        let timestamp = format_smart_timestamp(msg.timestamp);
        let color = if msg.is_outbound { Color::Cyan } else { Color::Green };
        let arrow = if msg.is_outbound { "→" } else { "←" };

        Line::from(vec![
            Span::styled(format!("{} ", arrow), Style::default().fg(color)),
            Span::styled(format!("[{}] ", timestamp), Style::default().fg(Color::DarkGray)),
            Span::styled(&msg.content, Style::default().fg(Color::White)),
        ])
    }).collect();

    // Calculate scroll to show latest messages at bottom
    let num_messages = messages.len() as u16;
    let scroll_offset = if num_messages > area.height {
        num_messages.saturating_sub(area.height)
    } else {
        0
    };

    let paragraph = Paragraph::new(messages)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));

    f.render_widget(paragraph, area);
}

/// Render empty state when no contacts
fn render_empty_state(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("No contacts yet", Style::default().fg(Color::Yellow))),
        Line::from(""),
        Line::from(Span::styled("Press / to open menu, then i to import a contact", Style::default().fg(Color::DarkGray))),
    ];

    f.render_widget(Paragraph::new(lines), area);
}

/// Format timestamp smartly (today: HH:MM:SS, older: DD-MM-YYYY HH:MM:SS)
fn format_smart_timestamp(unix_ts: i64) -> String {
    let now = chrono::Local::now();
    let msg_time = match chrono::DateTime::from_timestamp(unix_ts, 0) {
        Some(t) => t,
        None => return "??:??:??".to_string(),
    };
    let msg_local = msg_time.with_timezone(&chrono::Local);

    if msg_local.date_naive() == now.date_naive() {
        // Today: just HH:MM:SS
        msg_local.format("%H:%M:%S").to_string()
    } else {
        // Older: DD-MM-YYYY HH:MM:SS
        msg_local.format("%d-%m-%Y %H:%M:%S").to_string()
    }
}

/// Render the input area (multi-line text input)
pub fn render_input_area(f: &mut Frame, app: &App, area: Rect) {
    // If slash menu is showing, render it above the input
    if app.show_slash_menu && app.menu_state == MenuState::Closed {
        render_slash_menu(f, app, area);
        return;
    }

    // Thin separator line
    let separator = "─".repeat(area.width as usize);
    let separator_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let separator_widget = Paragraph::new(separator)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(separator_widget, separator_area);

    // Render prompt and input below separator
    let input_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    // Determine what to show based on current view
    let (prompt_text, prompt_style) = match app.menu_state {
        MenuState::ImportContact => {
            if app.input_mode == InputMode::Editing {
                (
                    format!("> {}_", app.contact_import_input),
                    Style::default().fg(Color::White)
                )
            } else {
                (
                    "> ".to_string(),
                    Style::default().fg(Color::DarkGray)
                )
            }
        }
        MenuState::ExportContact => {
            if app.input_mode == InputMode::Editing {
                (
                    format!("> {}_", app.contact_export_name),
                    Style::default().fg(Color::White)
                )
            } else {
                (
                    "> ".to_string(),
                    Style::default().fg(Color::DarkGray)
                )
            }
        }
        _ => {
            // Normal chat input
            if app.input_mode == InputMode::Editing {
                (
                    format!("> {}_", app.message_input),
                    Style::default().fg(Color::White)
                )
            } else {
                (
                    "> ".to_string(),
                    Style::default().fg(Color::DarkGray)
                )
            }
        }
    };

    let paragraph = Paragraph::new(prompt_text)
        .style(prompt_style)
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, input_area);
}

/// Render slash command menu
fn render_slash_menu(f: &mut Frame, app: &App, area: Rect) {
    let commands = app.get_filtered_slash_commands();

    let mut lines = vec![];

    for (idx, (cmd, desc)) in commands.iter().enumerate() {
        let is_selected = idx == app.slash_menu_index;
        let (prefix, style) = if is_selected {
            ("→ ", Style::default().fg(Color::Cyan))
        } else {
            ("  ", Style::default().fg(Color::White))
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(*cmd, style.add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", desc), Style::default().fg(Color::DarkGray)),
        ]));
    }

    // Separator
    let separator = "─".repeat(area.width as usize);
    lines.push(Line::from(Span::styled(separator, Style::default().fg(Color::DarkGray))));

    // Input line
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}_", app.message_input), Style::default().fg(Color::White)),
    ]));

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Render hints bar (no borders, minimal)
pub fn render_hints(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.menu_state {
        MenuState::Closed => {
            if app.show_slash_menu {
                // Slash menu is open
                vec![
                    Line::from(vec![
                        Span::styled("↑↓", Style::default().fg(Color::DarkGray)),
                        Span::styled(" navigate  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Enter", Style::default().fg(Color::DarkGray)),
                        Span::styled(" select  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("type to filter  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Esc", Style::default().fg(Color::DarkGray)),
                        Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
                    ]),
                    Line::from(vec![
                        Span::styled(&app.status_message, Style::default().fg(Color::White)),
                    ]),
                ]
            } else if app.input_mode == InputMode::Editing {
                // Editing mode in chat
                vec![
                    Line::from(vec![
                        Span::styled("Enter", Style::default().fg(Color::DarkGray)),
                        Span::styled(" send  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Esc", Style::default().fg(Color::DarkGray)),
                        Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
                    ]),
                    Line::from(vec![
                        Span::styled(&app.status_message, Style::default().fg(Color::White)),
                    ]),
                ]
            } else {
                // Normal mode in chat
                vec![
                    Line::from(vec![
                        Span::styled("/", Style::default().fg(Color::DarkGray)),
                        Span::styled(" commands  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("↑↓", Style::default().fg(Color::DarkGray)),
                        Span::styled(" switch contact  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            format!("polling: {}s", app.current_polling_interval),
                            Style::default().fg(Color::DarkGray)
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled(&app.status_message, Style::default().fg(Color::White)),
                    ]),
                ]
            }
        }
        _ => {
            // Any other view
            vec![
                Line::from(vec![
                    Span::styled("Esc", Style::default().fg(Color::DarkGray)),
                    Span::styled(" back to chat", Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(vec![
                    Span::styled(&app.status_message, Style::default().fg(Color::White)),
                ]),
            ]
        }
    };

    let paragraph = Paragraph::new(hints);
    f.render_widget(paragraph, area);
}

/// Render full-screen contacts view
pub fn render_contacts_view(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("Contacts", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    if app.peers.is_empty() {
        lines.push(Line::from(Span::styled("No contacts yet", Style::default().fg(Color::DarkGray))));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Type /import to add a contact", Style::default().fg(Color::DarkGray))));
    } else {
        for (idx, peer) in app.peers.iter().enumerate() {
            let (prefix, style) = if idx == app.selected_peer_index {
                ("→ ", Style::default().fg(Color::Cyan))
            } else {
                ("  ", Style::default().fg(Color::White))
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(&peer.name, style),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Render full-screen import view
pub fn render_import_view(f: &mut Frame, app: &App, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("Import Contact", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("• Drag and drop a .json file here", Style::default().fg(Color::Cyan))),
        Line::from(Span::styled("• Paste contact JSON", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("• Type file path (e.g., contact-Bob.json)", Style::default().fg(Color::DarkGray))),
        Line::from(""),
        Line::from(Span::styled("Then press Enter to import", Style::default().fg(Color::DarkGray))),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Render full-screen export view
pub fn render_export_view(f: &mut Frame, app: &App, area: Rect) {
    let lines = if app.contact_export_json.is_empty() {
        // Step 1: Enter name
        vec![
            Line::from(""),
            Line::from(Span::styled("Export Contact", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("Enter your name and press Enter", Style::default().fg(Color::DarkGray))),
            Line::from(""),
            Line::from(Span::styled("File will be saved to ~/Downloads/contact-<name>.json", Style::default().fg(Color::DarkGray))),
            Line::from(""),
        ]
    } else {
        // Step 2: Show success and file location
        vec![
            Line::from(""),
            Line::from(Span::styled("Export Contact", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("✓ Contact file saved to Downloads!", Style::default().fg(Color::Green))),
            Line::from(""),
            Line::from(Span::styled("Check your Downloads folder", Style::default().fg(Color::Cyan))),
            Line::from(""),
            Line::from(Span::styled("Share this file with your contact", Style::default().fg(Color::DarkGray))),
            Line::from(""),
            Line::from(Span::styled("Press Esc to return", Style::default().fg(Color::DarkGray))),
        ]
    };

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Render full-screen settings view
pub fn render_settings_view(f: &mut Frame, app: &App, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("Settings", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("Server URL: ", Style::default().fg(Color::DarkGray)),
            Span::raw(&app.settings_server_url),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Polling Interval: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{}s", app.settings_polling_interval)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Current: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{}s", app.current_polling_interval)),
        ]),
    ];

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Render simple view hints
pub fn render_view_hints(f: &mut Frame, hint: &str, area: Rect) {
    let separator = "─".repeat(area.width as usize);
    let separator_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let separator_widget = Paragraph::new(separator)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(separator_widget, separator_area);

    let hint_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let paragraph = Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray)));
    f.render_widget(paragraph, hint_area);
}
