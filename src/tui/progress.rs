use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::tui::widgets::{format_bytes, format_duration, format_speed, render_progress_bar};

/// Render the transfer progress view.
pub fn render_progress(frame: &mut Frame, area: Rect, app: &App) {
    let progress = app.transfer_progress_snapshot();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // overall progress bar
            Constraint::Length(8),  // stats
            Constraint::Min(5),    // current file + error log
        ])
        .split(area);

    // === Overall progress bar ===
    let percent = progress.percent();
    render_progress_bar(frame, chunks[0], percent, " Overall Progress ");

    // === Stats panel ===
    let elapsed = format_duration(progress.elapsed());
    let eta = progress
        .eta()
        .map(format_duration)
        .unwrap_or_else(|| "calculating...".into());

    let stats = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Files:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", progress.completed_files),
                Style::default().fg(Color::Green).bold(),
            ),
            Span::styled(
                format!(" / {}  ", progress.total_files),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("  Failed: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", progress.failed_files),
                Style::default().fg(if progress.failed_files > 0 {
                    Color::Red
                } else {
                    Color::Green
                }).bold(),
            ),
            Span::styled("  Skipped: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", progress.skipped_files),
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::styled("  Delta: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", progress.delta_skipped),
                Style::default().fg(Color::Rgb(100, 149, 237)).bold(), // cornflower blue
            ),
        ]),
        Line::from(vec![
            Span::styled("  Size:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_bytes(progress.transferred_bytes),
                Style::default().fg(Color::Cyan).bold(),
            ),
            Span::styled(
                format!(" / {}", format_bytes(progress.total_bytes)),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("   Workers: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", progress.active_workers),
                Style::default().fg(Color::Magenta).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Speed:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_speed(progress.speed_bytes_per_sec),
                Style::default().fg(Color::Cyan).bold(),
            ),
            Span::styled("   Elapsed: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &elapsed,
                Style::default().fg(Color::White),
            ),
            Span::styled("   ETA: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &eta,
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(""),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" 📊 Transfer Stats ")
            .title_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Rgb(15, 15, 25))),
    );

    frame.render_widget(stats, chunks[1]);

    // === Current file + error log ===
    let bottom_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // current file
            Constraint::Min(2),    // error log
        ])
        .split(chunks[2]);

    // Current file
    let current = Paragraph::new(Line::from(vec![
        Span::styled("  📄 ", Style::default()),
        Span::styled(
            if progress.current_file.is_empty() {
                "Preparing..."
            } else {
                &progress.current_file
            },
            Style::default().fg(Color::White),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Current File ")
            .title_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Rgb(15, 15, 25))),
    );

    frame.render_widget(current, bottom_chunks[0]);

    // Error log
    let error_lines: Vec<Line> = if progress.errors.is_empty() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No errors ✓",
                Style::default().fg(Color::Green),
            )),
        ]
    } else {
        progress
            .errors
            .iter()
            .rev()
            .take(20)
            .map(|(path, err)| {
                Line::from(vec![
                    Span::styled("  ✗ ", Style::default().fg(Color::Red)),
                    Span::styled(
                        path.rsplit('/').next().unwrap_or(path),
                        Style::default().fg(Color::Red),
                    ),
                    Span::styled(
                        format!(": {}", err),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            })
            .collect()
    };

    let errors = Paragraph::new(error_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" Errors ({}) ", progress.errors.len()))
                .title_style(Style::default().fg(if progress.errors.is_empty() {
                    Color::Green
                } else {
                    Color::Red
                }))
                .style(Style::default().bg(Color::Rgb(15, 15, 25))),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(errors, bottom_chunks[1]);
}
