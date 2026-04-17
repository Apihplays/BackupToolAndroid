use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
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
        .enumerate()
        .map(|(_i, device)| {
            let icon = if device.transport == "wifi" { "📶" } else { "🔌" };
            let content = Line::from(vec![
                Span::styled(
                    format!(" {} ", icon),
                    Style::default(),
                ),
                Span::styled(
                    &device.model,
                    Style::default().fg(Color::White).bold(),
                ),
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
    // Split into file tree (left) and info panel (right)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(70),
            Constraint::Percentage(30),
        ])
        .split(area);

    render_file_tree(frame, chunks[0], app);
    render_info_panel(frame, chunks[1], app);
}

/// Render the file tree.
fn render_file_tree(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" 📂 Device Files ")
        .title_style(Style::default().fg(Color::Cyan).bold())
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
        .map(|node| {
            let indent = "  ".repeat(node.depth);
            let checkbox = if node.selected { "☑" } else { "☐" };
            let icon = if node.is_dir {
                if node.expanded { "📂" } else { "📁" }
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
                    Style::default().fg(if node.is_dir {
                        Color::Cyan
                    } else {
                        Color::White
                    }).add_modifier(if node.is_dir {
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

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 70))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(app.browser_index));

    frame.render_stateful_widget(list, area, &mut state);
}

/// Render info panel showing selection stats.
fn render_info_panel(frame: &mut Frame, area: Rect, app: &App) {
    let selected_size = app.file_tree.as_ref()
        .map(|t| t.selected_total_size())
        .unwrap_or(0);
    let selected_count = app.file_tree.as_ref()
        .map(|t| t.selected_file_count())
        .unwrap_or(0);

    let total_size = app.file_tree.as_ref()
        .map(|t| t.total_size)
        .unwrap_or(0);
    let total_count = app.file_tree.as_ref()
        .map(|t| t.file_count)
        .unwrap_or(0);

    let info = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            " Selection",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("   Files: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", selected_count),
                Style::default().fg(Color::Green).bold(),
            ),
            Span::styled(
                format!(" / {}", total_count),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("   Size:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_bytes(selected_size),
                Style::default().fg(Color::Green).bold(),
            ),
            Span::styled(
                format!(" / {}", format_bytes(total_size)),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Destination",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("   {}", app.destination),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        if app.media_filter {
            Line::from(Span::styled(
                " 🔍 Media filter: ON",
                Style::default().fg(Color::Yellow),
            ))
        } else {
            Line::from(Span::styled(
                " 🔍 Media filter: OFF",
                Style::default().fg(Color::DarkGray),
            ))
        },
    ])
    .block(
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
