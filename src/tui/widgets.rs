use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, AppView};

/// Render the title bar at the top of the screen.
pub fn render_title_bar(frame: &mut Frame, area: Rect, app: &App) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " ⚡ andpull ",
            Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(
            match app.current_view {
                AppView::DeviceSelect => " 📱 Select Device ",
                AppView::ProfileSelect => " 🧩 Select Profiles ",
                AppView::FileBrowser => " 📂 Browse Files ",
                AppView::FileBrowserSearch => " 🔍 Search Files ",
                AppView::DestinationBrowser => " 📁 Select Destination ",
                AppView::Transferring => " 🚀 Transferring ",
                AppView::Summary => " 📊 Summary ",
            },
            Style::default().fg(Color::Yellow).bold(),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Rgb(20, 20, 30))),
    );

    frame.render_widget(title, area);
}

/// Render the status bar at the bottom of the screen.
pub fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let help_text = match app.current_view {
        AppView::DeviceSelect => "↑↓ Navigate │ Enter Select │ p Profiles │ r Refresh │ q Quit",
        AppView::ProfileSelect => {
            if app.restore_input_active {
                "Type backup dir │ Enter Start Restore │ Esc Cancel"
            } else {
                "↑↓ Navigate │ Space Toggle │ a All │ Enter Backup │ r Restore │ Esc Back │ q Quit"
            }
        }
        AppView::FileBrowser => "↑↓ Navigate │ Space Select │ Enter Expand │ a All │ n None │ f Filter │ s Start │ r Resume │ q Quit",
        AppView::FileBrowserSearch => "Type to search │ Enter/Esc Cancel",
        AppView::DestinationBrowser => "↑↓ Navigate │ Enter Select │ Backspace Up │ q Quit",
        AppView::Transferring => "c Cancel │ q Quit",
        AppView::Summary => "↑↓ Scroll │ b Back │ q Quit",
    };

    let status = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(help_text, Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Rgb(20, 20, 30))),
    );

    frame.render_widget(status, area);
}

/// Format bytes into human-readable string.
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format duration into human-readable string.
pub fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 3600 {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

/// Format transfer speed.
pub fn format_speed(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    if bytes_per_sec >= MB {
        format!("{:.1} MB/s", bytes_per_sec / MB)
    } else if bytes_per_sec >= KB {
        format!("{:.0} KB/s", bytes_per_sec / KB)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

/// Render a styled progress bar.
pub fn render_progress_bar(frame: &mut Frame, area: Rect, percent: f64, label: &str) {
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(label))
        .gauge_style(
            Style::default()
                .fg(Color::Cyan)
                .bg(Color::Rgb(30, 30, 40))
                .add_modifier(Modifier::BOLD),
        )
        .percent(percent.min(100.0) as u16)
        .label(format!("{:.1}%", percent));

    frame.render_widget(gauge, area);
}
