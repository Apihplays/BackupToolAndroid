use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, TransferMode};
use crate::profile::runner::ProfileOutcome;
use crate::tui::widgets::{format_bytes, format_duration, format_speed};

/// Render the transfer summary view.
pub fn render_summary(frame: &mut Frame, area: Rect, app: &App) {
    // Profile-mode summaries render grouped per-profile outcomes.
    if matches!(
        app.transfer_mode,
        TransferMode::ProfileBackup | TransferMode::ProfileRestore
    ) {
        if let Some(outcomes) = &app.profile_outcomes {
            render_profile_summary(frame, area, app, outcomes);
            return;
        }
    }

    let progress = app.transfer_progress_snapshot();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12), // summary stats
            Constraint::Min(5),     // error details
        ])
        .split(area);

    // === Summary stats ===
    let status_icon = if progress.failed_files == 0 {
        "✅"
    } else if progress.completed_files > 0 {
        "⚠️"
    } else {
        "❌"
    };

    let status_text = if progress.is_cancelled {
        "Transfer Cancelled"
    } else if progress.failed_files == 0 {
        "Transfer Complete!"
    } else {
        "Transfer Complete (with errors)"
    };

    let elapsed = format_duration(progress.elapsed());

    let summary = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {} {}", status_icon, status_text),
            Style::default()
                .fg(if progress.failed_files == 0 {
                    Color::Green
                } else {
                    Color::Yellow
                })
                .bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ✓ Completed:  ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{} files", progress.completed_files),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ⏭ Skipped:    ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{} files", progress.skipped_files),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ✗ Failed:     ", Style::default().fg(Color::Red)),
            Span::styled(
                format!("{} files", progress.failed_files),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  📦 Total:      ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format_bytes(progress.transferred_bytes),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ⏱ Time:       ", Style::default().fg(Color::DarkGray)),
            Span::styled(&elapsed, Style::default().fg(Color::White)),
            Span::styled("   Avg: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_speed(progress.speed_bytes_per_sec),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  🔒 Verified:   ",
                Style::default().fg(Color::Rgb(50, 205, 50)),
            ),
            Span::styled(
                format!("{} files", progress.integrity_verified),
                Style::default().fg(Color::White).bold(),
            ),
            if progress.integrity_failed > 0 {
                Span::styled(
                    format!("   ⚠ {} integrity failures", progress.integrity_failed),
                    Style::default().fg(Color::Red).bold(),
                )
            } else {
                Span::styled(
                    "   ✓ all checksums passed",
                    Style::default().fg(Color::Green),
                )
            },
        ]),
        Line::from(""),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" 📊 Transfer Summary ")
            .title_style(Style::default().fg(Color::Cyan).bold())
            .style(Style::default().bg(Color::Rgb(15, 15, 25))),
    );

    frame.render_widget(summary, chunks[0]);

    // === Error details ===
    let error_lines: Vec<Line> = if progress.errors.is_empty() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  All files transferred successfully! 🎉",
                Style::default().fg(Color::Green),
            )),
        ]
    } else {
        let mut lines = vec![Line::from("")];
        for (i, (path, err)) in progress.errors.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}. ", i + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(path, Style::default().fg(Color::Red)),
            ]));
            lines.push(Line::from(Span::styled(
                format!("     └─ {}", err),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines
    };

    let error_list = Paragraph::new(error_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(
                    " {} Failed Files ({}) ",
                    if progress.errors.is_empty() {
                        "✓"
                    } else {
                        "✗"
                    },
                    progress.errors.len()
                ))
                .title_style(Style::default().fg(if progress.errors.is_empty() {
                    Color::Green
                } else {
                    Color::Red
                }))
                .style(Style::default().bg(Color::Rgb(15, 15, 25))),
        )
        .scroll((app.summary_scroll, 0))
        .wrap(Wrap { trim: true });

    frame.render_widget(error_list, chunks[1]);
}

/// Render the grouped per-profile outcome summary (profile backup/restore).
fn render_profile_summary(frame: &mut Frame, area: Rect, app: &App, outcomes: &[ProfileOutcome]) {
    let total_files: u64 = outcomes.iter().map(|o| o.files_transferred).sum();
    let failed = outcomes.iter().filter(|o| !o.success).count();

    let (status_icon, status_text, status_color) = if failed == 0 {
        ("✅", "Profile Run Complete!", Color::Green)
    } else if failed < outcomes.len() {
        ("⚠️", "Profile Run Complete (with errors)", Color::Yellow)
    } else {
        ("❌", "Profile Run Failed", Color::Red)
    };

    let mode_label = match app.transfer_mode {
        TransferMode::ProfileRestore => "Restore",
        _ => "Backup",
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {} {}", status_icon, status_text),
            Style::default().fg(status_color).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Profiles: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", outcomes.len()),
                Style::default().fg(Color::White).bold(),
            ),
            Span::styled("   Files: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", total_files),
                Style::default().fg(Color::Cyan).bold(),
            ),
            Span::styled("   Failed profiles: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", failed),
                Style::default()
                    .fg(if failed > 0 { Color::Red } else { Color::Green })
                    .bold(),
            ),
        ]),
        Line::from(""),
    ];

    for outcome in outcomes {
        let (mark, mark_color) = if outcome.success {
            ("✓", Color::Green)
        } else {
            ("✗", Color::Red)
        };

        let detail = if outcome.new_files > 0 || outcome.changed_files > 0 || outcome.skipped_files > 0 {
            format!(
                "  — {} new, {} changed, {} skipped",
                outcome.new_files, outcome.changed_files, outcome.skipped_files
            )
        } else {
            format!("  — {} files", outcome.files_transferred)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", mark),
                Style::default().fg(mark_color).bold(),
            ),
            Span::styled(&outcome.name, Style::default().fg(Color::White).bold()),
            Span::styled(detail, Style::default().fg(Color::DarkGray)),
        ]));

        if let Some(err) = &outcome.error {
            lines.push(Line::from(Span::styled(
                format!("      └─ {}", err),
                Style::default().fg(Color::Red),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Mode: {}", mode_label),
        Style::default().fg(Color::DarkGray),
    )));

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" 📊 Profile Summary ")
                .title_style(Style::default().fg(Color::Cyan).bold())
                .style(Style::default().bg(Color::Rgb(15, 15, 25))),
        )
        .scroll((app.summary_scroll, 0))
        .wrap(Wrap { trim: true });

    frame.render_widget(para, area);
}
