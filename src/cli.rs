// Non-interactive CLI subcommands: `backup` and `restore`.
//
// Pure argument parsing lives here (with unit tests); execution wiring stays
// in `main.rs`. No external arg-parsing crates — std::env only.

/// Parsed command-line invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Interactive TUI mode (default when no subcommand is given).
    Tui { destination: String },
    /// Non-interactive backup run.
    Backup {
        profiles: Option<Vec<String>>,
        with_appdata: bool,
        destination: String,
    },
    /// Non-interactive restore run.
    Restore {
        with_appdata: Option<String>,
        backup_dir: String,
    },
}

/// Result of parsing: either a `Command`, help requested, or an error.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseOutcome {
    Run(Command),
    Help,
    Error(String),
}

const USAGE: &str = "\
⚡ andpull — Android backup tool

USAGE:
    andpull                                  Launch the interactive TUI
    andpull backup [OPTIONS] [DEST]          Non-interactive backup
    andpull restore [OPTIONS] [BACKUP_DIR]   Non-interactive restore
    andpull --help                           Show this help

BACKUP OPTIONS:
    --profiles <a,b,c>   Comma-separated profile names to back up
                         (default: all builtin profiles)
    --with-appdata       Also back up app data (requires root) to
                         DEST/<profile>_appdata.tar
    DEST                 Output directory (default: ./andpull_backup_<timestamp>)

RESTORE OPTIONS:
    --with-appdata <tar> Also restore app data from the given tar file
                         (requires root)
    BACKUP_DIR           Backup directory to push from
                         (default: newest ./andpull_backup_*)

EXAMPLES:
    andpull backup
    andpull backup --profiles whatsapp --with-appdata /mnt/flash/backup
    andpull restore /mnt/flash/backup
";

pub fn usage() -> &'static str {
    USAGE
}

fn default_backup_destination() -> String {
    let now = chrono_now();
    format!("andpull_backup_{now}")
}

/// Local timestamp formatted as YYYYmmdd_HHMMSS (no chrono dependency).
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil_from_unix(secs);
    format!("{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}")
}

/// Convert a unix timestamp into (year, month, day, hour, min, sec) in UTC.
/// Days-from-epoch civil algorithm (Howard Hinnant's `civil_from_days`).
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// Parse raw argv (without the binary name) into a [`ParseOutcome`].
pub fn parse_args(args: &[String]) -> ParseOutcome {
    match args.first().map(String::as_str) {
        None => ParseOutcome::Run(Command::Tui {
            destination: "andpull_output".to_string(),
        }),
        Some("-h") | Some("--help") | Some("help") => ParseOutcome::Help,
        Some("backup") => parse_backup(&args[1..]),
        Some("restore") => parse_restore(&args[1..]),
        // Back-compat: a bare positional destination launches the TUI as before.
        Some(dest) if !dest.starts_with('-') => ParseOutcome::Run(Command::Tui {
            destination: dest.to_string(),
        }),
        Some(flag) => ParseOutcome::Error(format!("unknown option or subcommand: {flag}")),
    }
}

fn parse_backup(args: &[String]) -> ParseOutcome {
    let mut profiles: Option<Vec<String>> = None;
    let mut with_appdata = false;
    let mut dest: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--profiles" => {
                if i + 1 >= args.len() {
                    return ParseOutcome::Error("--profiles requires a value".to_string());
                }
                i += 1;
                let list: Vec<String> = args[i]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if list.is_empty() {
                    return ParseOutcome::Error(
                        "--profiles requires at least one name".to_string(),
                    );
                }
                profiles = Some(list);
            }
            "--with-appdata" => with_appdata = true,
            s if s.starts_with('-') && s.len() > 1 => {
                return ParseOutcome::Error(format!("unknown flag for backup: {s}"));
            }
            s => {
                if dest.is_some() {
                    return ParseOutcome::Error(format!(
                        "unexpected extra argument for backup: {s}"
                    ));
                }
                dest = Some(s.to_string());
            }
        }
        i += 1;
    }

    ParseOutcome::Run(Command::Backup {
        profiles,
        with_appdata,
        destination: dest.unwrap_or_else(default_backup_destination),
    })
}

fn parse_restore(args: &[String]) -> ParseOutcome {
    let mut appdata_tar: Option<String> = None;
    let mut dir: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--with-appdata" => {
                if i + 1 >= args.len() {
                    return ParseOutcome::Error("--with-appdata requires a tar path".to_string());
                }
                i += 1;
                appdata_tar = Some(args[i].clone());
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return ParseOutcome::Error(format!("unknown flag for restore: {s}"));
            }
            s => {
                if dir.is_some() {
                    return ParseOutcome::Error(format!(
                        "unexpected extra argument for restore: {s}"
                    ));
                }
                dir = Some(s.to_string());
            }
        }
        i += 1;
    }

    ParseOutcome::Run(Command::Restore {
        with_appdata: appdata_tar,
        backup_dir: dir.unwrap_or_else(|| ".".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_args_is_tui_default_dest() {
        assert_eq!(
            parse_args(&[]),
            ParseOutcome::Run(Command::Tui {
                destination: "andpull_output".to_string()
            })
        );
    }

    #[test]
    fn bare_positional_is_tui_with_dest() {
        assert_eq!(
            parse_args(&v(&["mydest"])),
            ParseOutcome::Run(Command::Tui {
                destination: "mydest".to_string()
            })
        );
    }

    #[test]
    fn backup_defaults_all_profiles_and_timestamped_dest() {
        let out = parse_args(&v(&["backup"]));
        match out {
            ParseOutcome::Run(Command::Backup {
                profiles,
                with_appdata,
                destination,
            }) => {
                assert!(profiles.is_none());
                assert!(!with_appdata);
                assert!(destination.starts_with("andpull_backup_"));
                assert_eq!(destination.len(), "andpull_backup_YYYYmmdd_HHMMSS".len());
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn backup_full_flag_set_parses() {
        let out = parse_args(&v(&[
            "backup",
            "--profiles",
            "whatsapp,dcim",
            "--with-appdata",
            "/tmp/x",
        ]));
        assert_eq!(
            out,
            ParseOutcome::Run(Command::Backup {
                profiles: Some(vec!["whatsapp".to_string(), "dcim".to_string()]),
                with_appdata: true,
                destination: "/tmp/x".to_string(),
            })
        );
    }

    #[test]
    fn backup_single_profile_no_space_in_value() {
        let out = parse_args(&v(&["backup", "--profiles", " dcim "]));
        match out {
            ParseOutcome::Run(Command::Backup { profiles, .. }) => {
                assert_eq!(profiles, Some(vec!["dcim".to_string()]));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn backup_missing_profiles_value_errors() {
        assert!(matches!(
            parse_args(&v(&["backup", "--profiles"])),
            ParseOutcome::Error(_)
        ));
    }

    #[test]
    fn backup_unknown_flag_errors() {
        match parse_args(&v(&["backup", "--bogus"])) {
            ParseOutcome::Error(msg) => assert!(msg.contains("--bogus")),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn backup_extra_positional_errors() {
        assert!(matches!(
            parse_args(&v(&["backup", "/a", "/b"])),
            ParseOutcome::Error(_)
        ));
    }

    #[test]
    fn restore_with_dir_and_tar_parses() {
        assert_eq!(
            parse_args(&v(&["restore", "--with-appdata", "wa.tar", "/mnt/bk"])),
            ParseOutcome::Run(Command::Restore {
                with_appdata: Some("wa.tar".to_string()),
                backup_dir: "/mnt/bk".to_string(),
            })
        );
    }

    #[test]
    fn restore_defaults_to_cwd_without_appdata() {
        assert_eq!(
            parse_args(&v(&["restore"])),
            ParseOutcome::Run(Command::Restore {
                with_appdata: None,
                backup_dir: ".".to_string(),
            })
        );
    }

    #[test]
    fn restore_unknown_flag_errors() {
        match parse_args(&v(&["restore", "-x"])) {
            ParseOutcome::Error(msg) => assert!(msg.contains("-x")),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn help_flags_are_recognized() {
        for f in ["--help", "-h", "help"] {
            assert_eq!(parse_args(&v(&[f])), ParseOutcome::Help);
        }
    }

    #[test]
    fn unknown_subcommand_errors() {
        match parse_args(&v(&["--frobnicate"])) {
            ParseOutcome::Error(msg) => assert!(msg.contains("--frobnicate")),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn timestamp_format_is_sane() {
        let ts = chrono_now();
        assert_eq!(ts.len(), 15); // 8 digits + '_' + 6 digits
        assert!(ts.as_bytes()[8] == b'_');
        assert!(ts.chars().take(8).all(|c| c.is_ascii_digit()));
        assert!(ts.chars().skip(9).all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn civil_from_unix_known_values() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        // 2026-08-25T00:00:00Z == 1787587200
        assert_eq!(civil_from_unix(1_787_616_000), (2026, 8, 25, 0, 0, 0));
        assert_eq!(civil_from_unix(951_782_400), (2000, 2, 29, 0, 0, 0));
    }
}
