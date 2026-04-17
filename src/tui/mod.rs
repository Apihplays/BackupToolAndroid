pub mod browser;
pub mod progress;
pub mod summary;
pub mod widgets;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
use std::io::{self, stdout};
use std::time::Duration;

use crate::app::{App, AppView};

/// Initialize the terminal for TUI rendering.
pub fn init_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    Terminal::new(backend)
}

/// Restore the terminal to normal mode.
pub fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Main TUI event loop.
pub fn run_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        // Draw current view
        terminal.draw(|frame| {
            let area = frame.area();

            // Title bar
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),  // title bar
                    Constraint::Min(1),     // main content
                    Constraint::Length(3),  // status bar
                ])
                .split(area);

            // Render title bar
            widgets::render_title_bar(frame, chunks[0], app);

            // Render main content based on current view
            match app.current_view {
                AppView::DeviceSelect => browser::render_device_select(frame, chunks[1], app),
                AppView::FileBrowser => browser::render_file_browser(frame, chunks[1], app),
                AppView::Transferring => progress::render_progress(frame, chunks[1], app),
                AppView::Summary => summary::render_summary(frame, chunks[1], app),
            }

            // Render status bar
            widgets::render_status_bar(frame, chunks[2], app);
        })?;

        // Handle input events
        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.current_view {
                        AppView::DeviceSelect => handle_device_select_input(app, key.code),
                        AppView::FileBrowser => handle_browser_input(app, key.code),
                        AppView::Transferring => handle_transfer_input(app, key.code),
                        AppView::Summary => handle_summary_input(app, key.code),
                    }
                }
            }
        }

        // Check if we should update transfer progress
        if app.current_view == AppView::Transferring {
            app.update_transfer_progress();
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn handle_device_select_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.device_list_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.device_list_next(),
        KeyCode::Enter => app.select_device(),
        KeyCode::Char('r') => app.refresh_devices(),
        _ => {}
    }
}

fn handle_browser_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.browser_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.browser_next(),
        KeyCode::Enter => app.browser_toggle_expand(),
        KeyCode::Char(' ') => app.browser_toggle_select(),
        KeyCode::Char('a') => app.browser_select_all(),
        KeyCode::Char('n') => app.browser_select_none(),
        KeyCode::Char('s') => app.start_transfer(),
        KeyCode::Char('r') => app.resume_transfer(),
        KeyCode::Char('f') => app.toggle_media_filter(),
        KeyCode::Char('b') | KeyCode::Backspace => app.browser_go_back(),
        _ => {}
    }
}

fn handle_transfer_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.cancel_transfer();
            app.should_quit = true;
        }
        KeyCode::Char('c') => app.cancel_transfer(),
        _ => {}
    }
}

fn handle_summary_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('b') => app.go_to_browser(),
        KeyCode::Up | KeyCode::Char('k') => app.summary_scroll_up(),
        KeyCode::Down | KeyCode::Char('j') => app.summary_scroll_down(),
        _ => {}
    }
}
