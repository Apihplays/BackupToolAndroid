use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, AppView};
use crate::tui::widgets::format_bytes;

/// Render the device selection view.
pub fn render_device_select(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" 📱 Connected Devices ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .style(Style::default().bg(Color::Rgb(15, 15, 25)));

    if app.devices.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No devices found!",
                Style::default().fg(Color::Red).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Make sure:",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "    • USB debugging is enabled on your device",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "    • Device is connected via USB or WiFi ADB",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "    • ADB server is running (adb start-server)",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press 'r' to refresh",
                Style::default().fg(Color::Green),
            )),
        ])
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = app
        .devices
        .iter()
        .map(|device| {
            let icon = if device.transport == "wifi" {
                "📶"
            } else {
                "🔌"
            };
            let content = Line::from(vec![
                Span::styled(format!(" {} ", icon), Style::default()),
                Span::styled(&device.model, Style::default().fg(Color::White).bold()),
                Span::styled(
                    format!("  [{}]", device.serial),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    if device.state != "device" {
                        format!("  (Status: {}) - Check phone!", device.state)
                    } else {
                        format!("  ({})", device.transport)
                    },
                    Style::default().fg(if device.state != "device" {
                        Color::Red
                    } else if device.transport == "wifi" {
                        Color::Yellow
                    } else {
                        Color::Green
                    }),
                ),
            ]);

            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 70))
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(app.device_list_index));

    frame.render_stateful_widget(list, area, &mut state);
}

/// Render the file browser view.
pub fn render_file_browser(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(35),
            Constraint::Percentage(30),
        ])
        .split(area);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(
                if app.current_view == AppView::FileBrowserSearch || !app.search_query.is_empty() {
                    3
                } else {
                    0
                },
            ),
        ])
        .split(chunks[0]);

    render_file_tree(frame, left_chunks[0], app);

    if app.current_view == AppView::FileBrowserSearch || !app.search_query.is_empty() {
        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_style(
                Style::default().fg(if app.current_view == AppView::FileBrowserSearch {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            )
            .title(" Search (Glob / Regex) ");

        let input_text = format!("{}█", app.search_query);
        let search_para = Paragraph::new(input_text)
            .block(search_block)
            .style(Style::default().fg(Color::White));
        frame.render_widget(search_para, left_chunks[1]);
    }

    render_local_tree(frame, chunks[1], app);
    render_info_panel(frame, chunks[2], app);
}

/// Render the file tree.
fn render_file_tree(frame: &mut Frame, area: Rect, app: &App) {
    let is_active = app.active_pane == crate::app::Pane::Left;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_active {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
        .title(if is_active {
            " 📱 Android Device (Active) "
        } else {
            " 📱 Android Device "
        })
        .title_style(
            Style::default()
                .fg(if is_active { Color::Cyan } else { Color::Gray })
                .bold(),
        )
        .style(Style::default().bg(Color::Rgb(15, 15, 25)));

    if app.flat_tree.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Loading file tree...",
                Style::default().fg(Color::Yellow),
            )),
        ])
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = app
        .flat_tree
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let is_selected = i == app.browser_index;

            let style = if is_selected && is_active {
                Style::default().bg(Color::Cyan).fg(Color::Black).bold()
            } else if is_selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else if node.selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            let indent = "  ".repeat(node.depth);
            let checkbox = if node.selected { "☑" } else { "☐" };
            let icon = if node.is_dir {
                if node.expanded {
                    "📂"
                } else {
                    "📁"
                }
            } else {
                file_icon(&node.name)
            };

            let size_str = if node.is_dir {
                format!("({})", format_bytes(node.total_size))
            } else {
                format_bytes(node.size)
            };

            let content = Line::from(vec![
                Span::styled(
                    format!(" {}{} {} ", indent, checkbox, icon),
                    Style::default().fg(if node.selected {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    &node.name,
                    style.add_modifier(if node.is_dir {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                ),
                Span::styled(
                    format!("  {}", size_str),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            ListItem::new(content)
        })
        .collect();

    let list = List::new(items).block(block).highlight_style(if is_active {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else {
        Style::default().bg(Color::DarkGray)
    });

    let mut state = ListState::default();
    state.select(Some(app.browser_index));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_local_tree(frame: &mut Frame, area: Rect, app: &App) {
    let is_active = app.active_pane == crate::app::Pane::Right;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_active {
            Color::Green
        } else {
            Color::DarkGray
        }))
        .title(if is_active {
            " 💻 Local PC (Active) "
        } else {
            " 💻 Local PC "
        })
        .title_style(
            Style::default()
                .fg(if is_active { Color::Green } else { Color::Gray })
                .bold(),
        )
        .style(Style::default().bg(Color::Rgb(15, 20, 15)));

    if app.local_flat_tree.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Folder is empty",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = app
        .local_flat_tree
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let is_selected = i == app.local_browser_index;

            let style = if is_selected && is_active {
                Style::default().bg(Color::Green).fg(Color::Black).bold()
            } else if is_selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else if node.selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = "  ".repeat(node.depth);
            let icon = if node.is_dir {
                if node.expanded {
                    "📂 "
                } else {
                    "📁 "
                }
            } else {
                "📄 "
            };
            let checkbox = if node.selected { "[x] " } else { "[ ] " };

            let name_str = format!("{}{}{}{}", prefix, icon, checkbox, node.name);
            ListItem::new(Line::from(vec![Span::styled(name_str, style)]))
        })
        .collect();

    let list = List::new(items).block(block).highlight_style(if is_active {
        Style::default().bg(Color::Green).fg(Color::Black)
    } else {
        Style::default().bg(Color::DarkGray)
    });

    let mut state = ListState::default();
    state.select(Some(app.local_browser_index));

    frame.render_stateful_widget(list, area, &mut state);
}

/// Render the right-side info panel.
fn render_info_panel(frame: &mut Frame, area: Rect, app: &App) {
    let (node_name, node_path, node_size, node_is_dir) = match app.active_pane {
        crate::app::Pane::Left => {
            if let Some(flat_node) = app.flat_tree.get(app.browser_index) {
                (
                    flat_node.name.clone(),
                    flat_node.path.clone(),
                    flat_node.size,
                    flat_node.is_dir,
                )
            } else {
                return;
            }
        }
        crate::app::Pane::Right => {
            if let Some(flat_node) = app.local_flat_tree.get(app.local_browser_index) {
                (
                    flat_node.name.clone(),
                    flat_node.path.clone(),
                    flat_node.size,
                    flat_node.is_dir,
                )
            } else {
                return;
            }
        }
    };

    let text = vec![
        Line::from(vec![
            Span::styled("Name: ", Style::default().fg(Color::Gray)),
            Span::styled(node_name.clone(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::Gray)),
            Span::styled(node_path.clone(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Type: ", Style::default().fg(Color::Gray)),
            Span::styled(
                if node_is_dir { "Directory" } else { "File" },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("Size: ", Style::default().fg(Color::Gray)),
            Span::styled(format_bytes(node_size), Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Shortcuts:",
            Style::default().fg(Color::Yellow).bold(),
        )),
        Line::from("  [Tab]    Switch Panes"),
        Line::from("  [Space]  Select file/folder"),
        Line::from("  [Enter]  Expand/Collapse folder"),
        Line::from("  [s]      Sync (Pull/Push)"),
        Line::from("  [Del/x]  Delete selected"),
        Line::from("  [f]      Toggle media filter"),
        Line::from("  [/]      Search / Filter"),
    ];

    let info = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" ℹ Info ")
            .title_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Rgb(15, 15, 25))),
    );

    frame.render_widget(info, area);
}

/// Get a file icon based on extension.
fn file_icon(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "heic" | "heif" | "svg" | "tiff" => "🖼️",
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "3gp" => "🎬",
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "wma" | "m4a" | "opus" => "🎵",
        "pdf" => "📄",
        "apk" => "📦",
        "zip" | "tar" | "gz" | "rar" | "7z" => "🗜️",
        _ => "📄",
    }
}

/// Render the destination selection view.
pub fn render_destination_browser(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(format!(" 📁 Local Destination: {} ", app.destination))
        .title_style(Style::default().fg(Color::Yellow).bold())
        .style(Style::default().bg(Color::Rgb(15, 15, 25)));

    if app.local_flat_tree.is_empty() {
        let msg = Paragraph::new("No local files found or permission denied.")
            .style(Style::default().fg(Color::Red))
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = app
        .local_flat_tree
        .iter()
        .map(|node| {
            let content = if node.name == ".." {
                Line::from(Span::styled(
                    " ⬆️  Up (..)",
                    Style::default().fg(Color::Cyan),
                ))
            } else {
                Line::from(Span::styled(
                    format!(" 📁 {}", node.name),
                    Style::default().fg(Color::White),
                ))
            };
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 70))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(app.local_browser_index));

    frame.render_stateful_widget(list, area, &mut state);
}
