pub mod browser;
pub mod profile;
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
pub fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        // Draw current view
        terminal.draw(|frame| {
            let area = frame.area();

            // Title bar
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // title bar
                    Constraint::Min(1),    // main content
                    Constraint::Length(3), // status bar
                ])
                .split(area);

            // Render title bar
            widgets::render_title_bar(frame, chunks[0], app);

            // Render main content based on current view
            match app.current_view {
                AppView::DeviceSelect => browser::render_device_select(frame, chunks[1], app),
                AppView::ProfileSelect => profile::render_profile_select(frame, chunks[1], app),
                AppView::FileBrowser => browser::render_file_browser(frame, chunks[1], app),
                AppView::FileBrowserSearch => browser::render_file_browser(frame, chunks[1], app),
                AppView::DestinationBrowser => {
                    browser::render_destination_browser(frame, chunks[1], app)
                }
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
                        AppView::ProfileSelect => handle_profile_select_input(app, key.code),
                        AppView::FileBrowser => handle_browser_input(app, key.code),
                        AppView::FileBrowserSearch => handle_browser_search_input(app, key.code),
                        AppView::DestinationBrowser => {
                            handle_destination_browser_input(app, key.code)
                        }
                        AppView::Transferring => handle_transfer_input(app, key.code),
                        AppView::Summary => handle_summary_input(app, key.code),
                    }
                }
            }
        }

        // Check if we should update transfer progress
        if app.current_view == AppView::Transferring {
            match app.transfer_mode {
                crate::app::TransferMode::Manual => app.update_transfer_progress(),
                _ => app.update_profile_batch(),
            }
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
        KeyCode::Char('p') => app.open_profile_select(),
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
        KeyCode::Char('/') => app.current_view = AppView::FileBrowserSearch,
        KeyCode::Tab => app.toggle_pane(),
        KeyCode::Delete | KeyCode::Char('x') => app.delete_selected(),
        _ => {}
    }
}

fn handle_browser_search_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Enter => app.current_view = AppView::FileBrowser,
        KeyCode::Backspace => {
            app.search_query.pop();
            app.apply_search_filter();
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
            app.apply_search_filter();
        }
        _ => {}
    }
}

fn handle_destination_browser_input(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Backspace => app.browser_go_back(),
        _ => {}
    }
}

fn handle_profile_select_input(app: &mut App, key: KeyCode) {
    // Restore directory text-input mode.
    if app.restore_input_active {
        match key {
            KeyCode::Esc => {
                app.restore_input_active = false;
                app.restore_dir_input.clear();
            }
            KeyCode::Enter => app.start_profile_restore(),
            KeyCode::Backspace => {
                app.restore_dir_input.pop();
            }
            KeyCode::Char(c) => app.restore_dir_input.push(c),
            _ => {}
        }
        return;
    }

    match key {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Esc => app.current_view = AppView::DeviceSelect,
        KeyCode::Up | KeyCode::Char('k') => app.profile_list_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.profile_list_next(),
        KeyCode::Char(' ') => app.profile_toggle(),
        KeyCode::Char('a') => app.profile_toggle_all(),
        KeyCode::Char('+') | KeyCode::Char('=') => app.cycle_workers_up(),
        KeyCode::Char('-') | KeyCode::Char('_') => app.cycle_workers_down(),
        KeyCode::Enter => app.start_profile_backup(),
        KeyCode::Char('r') => app.start_restore_input(),
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
