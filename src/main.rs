mod adb;
mod app;
mod cli;
mod error;
mod profile;
mod scanner;
mod state;
mod transfer;
mod tui;

use std::path::Path;
use std::process::exit;
use std::sync::Arc;

use adb::client::AdbClient;
use cli::{Command, ParseOutcome};
use profile::appdata::backup_appdata_best_effort;
use profile::restore::{preflight_restore, RestoreRunner};
use profile::runner::ProfileRunner;

const APPDATA_PACKAGE: &str = "com.whatsapp";

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse_args(&raw) {
        ParseOutcome::Help => {
            print!("{}", cli::usage());
            exit(0);
        }
        ParseOutcome::Error(msg) => {
            eprintln!("error: {msg}\n");
            eprint!("{}", cli::usage());
            exit(2);
        }
        ParseOutcome::Run(cmd) => match cmd {
            Command::Tui { destination } => run_tui(destination),
            Command::Backup {
                profiles,
                with_appdata,
                destination,
            } => run_backup(profiles, with_appdata, &destination),
            Command::Restore {
                with_appdata,
                backup_dir,
            } => run_restore(with_appdata, &backup_dir),
        },
    }
}

/// Ensure a directory exists or abort with a message.
fn ensure_dir(path: &str) {
    if let Err(e) = std::fs::create_dir_all(path) {
        eprintln!("Failed to create destination directory {path}: {e}");
        exit(1);
    }
}

fn connect_device() -> AdbClient {
    let mut client = AdbClient::new();
    let devices = match client.list_devices() {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => {
            eprintln!("No Android devices connected (adb devices shows none usable).");
            exit(1);
        }
        Err(e) => {
            eprintln!("Failed to list devices: {e}");
            exit(1);
        }
    };
    if devices.len() > 1 {
        eprintln!(
            "Multiple devices connected ({}); non-interactive mode auto-selects only a single device.",
            devices.len()
        );
        for d in &devices {
            eprintln!("  - {d}");
        }
        exit(1);
    }
    let device = devices.into_iter().next().unwrap();
    println!("Using device: {device}");
    client.select_device(device);
    client
}

fn resolve_profiles(names: Option<Vec<String>>) -> Vec<profile::ProfileSpec> {
    match names {
        None => profile::builtin_profiles(),
        Some(list) => {
            let mut specs = Vec::new();
            for name in &list {
                match profile::find_profile(name) {
                    Some(p) => specs.push(p),
                    None => {
                        eprintln!("Unknown profile: {name}");
                        exit(2);
                    }
                }
            }
            specs
        }
    }
}

fn report_outcomes(outcomes: &[profile::runner::ProfileOutcome]) -> bool {
    let mut all_ok = true;
    for o in outcomes {
        if o.success {
            println!("[ok] {}: {} files", o.name, o.files_transferred);
        } else {
            all_ok = false;
            println!(
                "[FAIL] {}: {}",
                o.name,
                o.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
    all_ok
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

fn run_backup(profiles: Option<Vec<String>>, with_appdata: bool, destination: &str) -> ! {
    ensure_dir(destination);
    let specs = resolve_profiles(profiles);
    let client = Arc::new(connect_device());

    println!("Backing up {} profile(s) to {destination}...", specs.len());
    let outcomes =
        ProfileRunner::default().run_all(Arc::clone(&client), specs.clone(), destination);
    let all_ok = report_outcomes(&outcomes);

    // App-data backup runs after the media profiles finish (whatsapp only).
    if with_appdata {
        let out_tar = Path::new(destination).join(format!("{APPDATA_PACKAGE}_appdata.tar"));
        print!("[appdata] {}: ", out_tar.display());
        match backup_appdata_best_effort(&client, APPDATA_PACKAGE, &out_tar) {
            Ok(n) => println!("[ok] {} ({})", out_tar.display(), human_size(n)),
            Err(e) => {
                println!("[FAIL] {e}");
                exit(1);
            }
        }
    }

    exit(if all_ok { 0 } else { 1 });
}

fn run_restore(with_appdata: Option<String>, backup_dir: &str) -> ! {
    if !Path::new(backup_dir).is_dir() {
        eprintln!("Backup directory does not exist: {backup_dir}");
        exit(2);
    }
    if let Some(tar) = &with_appdata {
        if !Path::new(tar).is_file() {
            eprintln!("App-data tar not found: {tar}");
            exit(2);
        }
    }
    let client = Arc::new(connect_device());

    // Preflight warnings before touching anything.
    let report = preflight_restore(&client, APPDATA_PACKAGE);
    for w in &report.warnings {
        println!("[warn] {w}");
    }
    if with_appdata.is_some() && !client.su_available() {
        eprintln!("--with-appdata requires root (su) on the device.");
        exit(1);
    }
    println!(
        "Preflight: package installed={}, free space on /sdcard={}",
        report.package_installed,
        human_size(report.free_bytes)
    );

    println!("Restoring from {backup_dir}...");
    let outcomes = RestoreRunner::new().run_all(
        client,
        profile::builtin_profiles(),
        backup_dir,
        false,
        with_appdata.map(std::path::PathBuf::from),
    );

    exit(if report_outcomes(&outcomes) { 0 } else { 1 });
}

fn run_tui(destination: String) -> ! {
    // Ensure the destination directory exists
    ensure_dir(&destination);

    // Initialize TUI
    let mut terminal = match tui::init_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to initialize terminal: {e}");
            exit(1);
        }
    };

    // Create app
    let mut app = app::App::new(destination);
    app.init();

    // Run TUI event loop
    let result = tui::run_tui(&mut terminal, &mut app);

    // Restore terminal
    if let Err(e) = tui::restore_terminal() {
        eprintln!("Failed to restore terminal: {e}");
    }

    // Report any TUI errors
    if let Err(e) = result {
        eprintln!("Application error: {e}");
        exit(1);
    }

    println!("\n⚡ andpull — Transfer complete. Thanks for using andpull!");
    exit(0);
}
