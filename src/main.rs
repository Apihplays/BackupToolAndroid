mod adb;
mod app;
mod error;
mod scanner;
mod state;
mod transfer;
mod tui;

use std::env;

fn main() {
    // Parse destination from args, default to ./andpull_output
    let args: Vec<String> = env::args().collect();
    let destination = if args.len() > 1 {
        args[1].clone()
    } else {
        "andpull_output".to_string()
    };
    // Ensure the destination directory exists
    if let Err(e) = std::fs::create_dir_all(&destination) {
        eprintln!("Failed to create destination directory: {}", e);
        std::process::exit(1);
    }

    // Initialize TUI
    let mut terminal = match tui::init_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to initialize terminal: {}", e);
            std::process::exit(1);
        }
    };

    // Create app
    let mut app = app::App::new(destination);
    app.init();

    // Run TUI event loop
    let result = tui::run_tui(&mut terminal, &mut app);

    // Restore terminal
    if let Err(e) = tui::restore_terminal() {
        eprintln!("Failed to restore terminal: {}", e);
    }

    // Report any TUI errors
    if let Err(e) = result {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }

    println!("\n⚡ andpull — Transfer complete. Thanks for using andpull!");
}
