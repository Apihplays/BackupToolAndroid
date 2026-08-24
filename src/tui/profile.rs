use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, TransferMode};
use crate::profile::runner::{ProfileRunState, ProfileSlot};
use crate::tui::widgets::format_duration;

/// Render the profile selection view (checkbox list + optional restore input).
pub fn render_profile_select(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" 🧩 Backup Profiles ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .style(Style::default().bg(Color::Rgb(15, 15, 25)));

    let items: Vec<ListItem> = app
        .profiles
        .iter()
        .enumerate()
        .map(|(i, profile)| {
            let checked = app.selected_profiles.get(i).copied().unwrap_or(false);
            let checkbox = if checked { "[x]" } else { "[ ]" };
            let is_cursor = i == app.profile_index;

            let root_tag = if profile.requires_root {
                Span::styled(" 🔒root", Style::default().fg(Color::Red))
            } else {
                Span::styled("", Style::default())
            };

            let content = Line::from(vec![
                Span::styled(
                    format!(" {} ", checkbox),
                    Style::default().fg(if checked {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    &profile.name,
                    if is_cursor {
                        Style::default().fg(Color::Cyan).bold()
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                root_tag,
                Span::styled(
                    format!("   (priority {})", profile.priority),
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
    state.select(Some(app.profile_index));

    // Restore input row (search-input style) when active.
    let constraints = if app.restore_input_active {
        vec![Constraint::Min(3), Constraint::Length(3)]
    } else {
        vec![Constraint::Min(0)]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    frame.render_stateful_widget(list, chunks[0], &mut state);

    if app.restore_input_active {
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Restore from backup dir (Enter to start) ");
        let input_text = format!("{}█", app.restore_dir_input);
        let input = Paragraph::new(input_text)
            .block(input_block)
            .style(Style::default().fg(Color::White));
        frame.render_widget(input, chunks[1]);
    }
}

/// Render the per-profile progress board during a profile-mode transfer.
/// Returns true if this was a profile-mode render.
pub fn render_profile_progress(frame: &mut Frame, area: Rect, app: &App) -> bool {
    if matches!(
        app.transfer_mode,
        TransferMode::ProfileBackup | TransferMode::ProfileRestore
    ) {
    } else {
        return false;
    }

    let slots = app.profile_slots_snapshot().unwrap_or_default();
    let elapsed = app
        .profile_batch
        .as_ref()
        .map(|b| format_duration(b.elapsed()))
        .unwrap_or_else(|| "0s".into());

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Elapsed: ", Style::default().fg(Color::DarkGray)),
            Span::styled(elapsed, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
    ];

    for slot in &slots {
        lines.push(profile_slot_line(slot));
    }
    lines.push(Line::from(""));

    let title = match app.transfer_mode {
        TransferMode::ProfileRestore => " 🔄 Restoring Profiles ",
        _ => " 🚀 Running Profiles ",
    };

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(title)
            .title_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Rgb(15, 15, 25))),
    );

    frame.render_widget(para, area);
    true
}

fn profile_slot_line(slot: &ProfileSlot) -> Line<'_> {
    let icon = "⠿"; // spinner-ish indeterminate marker
    match slot.state {
        ProfileRunState::Pending => Line::from(vec![
            Span::styled(format!("  {} ", icon), Style::default().fg(Color::DarkGray)),
            Span::styled(&slot.name, Style::default().fg(Color::DarkGray)),
            Span::styled("  waiting…", Style::default().fg(Color::DarkGray)),
        ]),
        ProfileRunState::Running => Line::from(vec![
            Span::styled(format!("  {} ", icon), Style::default().fg(Color::Yellow)),
            Span::styled(&slot.name, Style::default().fg(Color::White).bold()),
            Span::styled("  running…", Style::default().fg(Color::Yellow)),
        ]),
        ProfileRunState::Done => Line::from(vec![
            Span::styled("  ✓ ", Style::default().fg(Color::Green)),
            Span::styled(&slot.name, Style::default().fg(Color::White)),
            Span::styled(
                format!("  done ({} files)", slot.files_transferred),
                Style::default().fg(Color::Green),
            ),
        ]),
    }
}
