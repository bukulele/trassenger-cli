use crate::app::{App, InputMode, MenuState};
use ratatui::{
    layout::Rect,
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

    // Work out how many rendered rows each message takes (accounting for \n and wrapping).
    // We need this to know which messages are visible in the current scroll position.
    let area_width = area.width.max(1) as usize;

    // prefix = "→ [HH:MM:SS] " -- compute per message below
    struct MsgMeta {
        rows: usize,
    }

    let meta: Vec<MsgMeta> = app.messages.iter().map(|msg| {
        let timestamp = format_smart_timestamp(msg.timestamp);
        let prefix_len = 2 + 1 + timestamp.len() + 2; // "→ " + "[" + ts + "] "
        let content_width = area_width.saturating_sub(prefix_len).max(1);
        let mut rows = 0usize;
        for segment in msg.content.split('\n') {
            let chars = segment.chars().count();
            rows += ((chars + content_width - 1) / content_width).max(1);
        }
        MsgMeta { rows }
    }).collect();

    let total_rows: usize = meta.iter().map(|m| m.rows).sum();

    // Clamp scroll offset so you can't scroll past the top.
    let max_offset = total_rows.saturating_sub(area.height as usize);
    let scroll_offset = app.chat_scroll_offset.min(max_offset);

    // Find which message and row-within-message to start rendering from.
    // We want to render starting at row (total_rows - area.height - scroll_offset) from the top.
    let start_row = total_rows
        .saturating_sub(area.height as usize)
        .saturating_sub(scroll_offset);

    // Walk messages, skipping until we reach start_row, then render.
    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    let mut row_cursor = 0usize;

    for (msg, m) in app.messages.iter().zip(meta.iter()) {
        let msg_end = row_cursor + m.rows;
        if msg_end <= start_row {
            row_cursor = msg_end;
            continue;
        }

        let timestamp = format_smart_timestamp(msg.timestamp);
        let color = if msg.is_outbound { Color::Cyan } else { Color::Green };
        let arrow = if msg.is_outbound { "→" } else { "←" };
        let prefix = format!("{} [{}] ", arrow, timestamp);
        let prefix_len = prefix.chars().count();
        let content_width = area_width.saturating_sub(prefix_len).max(1);

        // Expand message into individual rendered rows.
        let mut msg_rows: Vec<Line> = Vec::with_capacity(m.rows);
        let mut first = true;
        for segment in msg.content.split('\n') {
            let chars: Vec<char> = segment.chars().collect();
            if chars.is_empty() {
                msg_rows.push(if first {
                    first = false;
                    Line::from(vec![Span::styled(prefix.clone(), Style::default().fg(color))])
                } else {
                    Line::from("")
                });
                continue;
            }
            let mut offset = 0;
            while offset < chars.len() {
                let chunk: String = chars[offset..chars.len().min(offset + content_width)].iter().collect();
                msg_rows.push(if first {
                    first = false;
                    Line::from(vec![
                        Span::styled(prefix.clone(), Style::default().fg(color)),
                        Span::styled(chunk, Style::default().fg(Color::White)),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw(" ".repeat(prefix_len)),
                        Span::styled(chunk, Style::default().fg(Color::White)),
                    ])
                });
                offset += content_width;
            }
        }

        // Skip rows of this message that are above start_row.
        let skip = start_row.saturating_sub(row_cursor);
        lines.extend(msg_rows.into_iter().skip(skip));

        row_cursor = msg_end;
        if lines.len() >= area.height as usize {
            break;
        }
    }

    let paragraph = Paragraph::new(lines);
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

    let separator = "─".repeat(area.width as usize);
    let sep_style = Style::default().fg(Color::DarkGray);

    // Top separator
    let top_sep_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
    f.render_widget(Paragraph::new(separator.clone()).style(sep_style), top_sep_area);

    // Bottom separator (last row of area)
    let bot_sep_area = Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 };
    f.render_widget(Paragraph::new(separator).style(sep_style), bot_sep_area);

    // Text area between the two separators
    let input_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(2),
    };

    // Determine what to show based on current view
    if app.input_mode == InputMode::Editing {
        let lines = match app.menu_state {
            MenuState::ImportContact => {
                vec![Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}_", app.contact_import_input), Style::default().fg(Color::White)),
                ])]
            }
            MenuState::ExportContact => {
                vec![Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}_", app.contact_export_name), Style::default().fg(Color::White)),
                ])]
            }
            _ => {
                // Split at cursor to render cursor indicator
                let chars: Vec<char> = app.message_input.chars().collect();
                let cursor = app.input_cursor.min(chars.len());
                let before: String = chars[..cursor].iter().collect();
                let after: String = chars[cursor..].iter().collect();

                // Build lines: split on \n within before/after
                let full = format!("{}\x00{}", before, after); // \x00 marks cursor pos
                let mut lines: Vec<Line> = Vec::new();
                let mut first = true;
                for segment in full.split('\n') {
                    let prefix = if first { first = false; "> " } else { "" };
                    if let Some(cursor_pos) = segment.find('\x00') {
                        let seg_before = &segment[..cursor_pos];
                        let seg_after = &segment[cursor_pos + 1..];
                        lines.push(Line::from(vec![
                            Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                            Span::styled(seg_before.to_string(), Style::default().fg(Color::White)),
                            Span::styled("_", Style::default().fg(Color::Cyan)),
                            Span::styled(seg_after.to_string(), Style::default().fg(Color::White)),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                            Span::styled(segment.to_string(), Style::default().fg(Color::White)),
                        ]));
                    }
                }
                lines
            }
        };
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), input_area);
    } else {
        f.render_widget(
            Paragraph::new(Span::styled("> ", Style::default().fg(Color::DarkGray))),
            input_area,
        );
    }
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
                // Editing mode in chat - show appropriate newline key
                let newline_hint = if app.keyboard_enhancements_supported {
                    "Shift+Enter"
                } else {
                    "Ctrl+J"
                };
                vec![
                    Line::from(vec![
                        Span::styled("Enter", Style::default().fg(Color::DarkGray)),
                        Span::styled(" send  ", Style::default().fg(Color::DarkGray)),
                        Span::styled(newline_hint, Style::default().fg(Color::DarkGray)),
                        Span::styled(" newline  ", Style::default().fg(Color::DarkGray)),
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
                        Span::styled(" scroll  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Ctrl+P/N", Style::default().fg(Color::DarkGray)),
                        Span::styled(" contact  ", Style::default().fg(Color::DarkGray)),
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
pub fn render_import_view(f: &mut Frame, _app: &App, area: Rect) {
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
            Span::styled("Adaptive (live): ", Style::default().fg(Color::DarkGray)),
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
