// Backup profile specifications.
// Scaffolding: consumed by upcoming transfer/scanner integration.

pub mod appdata;
pub mod restore;
pub mod runner;

/// A single source location on the device to back up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpec {
    pub device_path: String,
    pub alt_paths: Vec<String>,
    pub recursive: bool,
    pub extensions: Option<Vec<String>>,
}

/// A named backup profile: a prioritized collection of sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSpec {
    pub name: String,
    pub priority: u8,
    pub requires_root: bool,
    pub sources: Vec<SourceSpec>,
}

/// Returns the builtin profiles (whatsapp, dcim), sorted by priority.
pub fn builtin_profiles() -> Vec<ProfileSpec> {
    vec![
        ProfileSpec {
            name: "whatsapp".to_string(),
            priority: 0,
            requires_root: true,
            sources: vec![SourceSpec {
                device_path: "/sdcard/Android/media/com.whatsapp".to_string(),
                alt_paths: vec!["/sdcard/WhatsApp".to_string()],
                recursive: true,
                extensions: None,
            }],
        },
        ProfileSpec {
            name: "dcim".to_string(),
            priority: 1,
            requires_root: false,
            sources: vec![SourceSpec {
                device_path: "/sdcard/DCIM".to_string(),
                alt_paths: vec![],
                recursive: true,
                extensions: Some(vec![
                    "jpg".to_string(),
                    "jpeg".to_string(),
                    "png".to_string(),
                    "heic".to_string(),
                    "mp4".to_string(),
                    "mov".to_string(),
                    "webm".to_string(),
                    "dng".to_string(),
                ]),
            }],
        },
    ]
}

/// Clone-based lookup among builtin profiles by name.
///
/// Returns an owned `ProfileSpec` (cloned) rather than a reference so
/// callers never borrow from a temporary; builtins are tiny and cheap to clone.
pub fn find_profile(name: &str) -> Option<ProfileSpec> {
    builtin_profiles().into_iter().find(|p| p.name == name)
}

// ---------------------------------------------------------------------------
// Build-artifact / junk detection
// ---------------------------------------------------------------------------

/// Filename suffixes and exact names that cargo/build systems leave behind.
pub const JUNK_EXTENSIONS: &[&str] = &["o", "rlib", "rmeta"];

pub const JUNK_NAMES: &[&str] = &[
    ".rustc_info.json",
    ".cargo-lock",
    ".cargo-build-lock",
    ".cargo-artifact-lock",
    "target",
];

/// Returns `true` when `name` is almost certainly a build artifact and should
/// never be pulled as "backup data".
///
/// Detection is deliberately conservative: it only catches known cargo/build
/// patterns plus 40-char lowercase hex fingerprints.
pub fn is_build_artifact(name: &str) -> bool {
    // Exact name matches (e.g. "target" directory, ".cargo-lock").
    if JUNK_NAMES.contains(&name) {
        return true;
    }

    // Extension matches (e.g. "foo.o", "bar.rlib").
    if let Some(ext) = name.rsplit('.').next() {
        if JUNK_EXTENSIONS.contains(&ext) {
            return true;
        }
    }

    // Cargo fingerprint files: exactly 40 lowercase hex chars (no extension).
    if !name.contains('.') && name.len() == 40 && name.bytes().all(|b| b.is_ascii_hexdigit()) && !name.bytes().any(|b| b.is_ascii_uppercase()) {
        return true;
    }

    // Known cargo build-script output filenames.
    if name.starts_with("run-build-script-") || name == "rustix_test_can_compile" {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_returns_two() {
        assert_eq!(builtin_profiles().len(), 2);
    }

    #[test]
    fn whatsapp_priority_lower_than_dcim() {
        let profiles = builtin_profiles();
        let wa = profiles.iter().find(|p| p.name == "whatsapp").unwrap();
        let dcim = profiles.iter().find(|p| p.name == "dcim").unwrap();
        assert!(wa.priority < dcim.priority);
    }

    #[test]
    fn whatsapp_requires_root() {
        assert!(find_profile("whatsapp").unwrap().requires_root);
    }

    #[test]
    fn dcim_does_not_require_root() {
        assert!(!find_profile("dcim").unwrap().requires_root);
    }

    #[test]
    fn whatsapp_source_matches_exactly() {
        let p = find_profile("whatsapp").unwrap();
        assert_eq!(p.sources.len(), 1);
        let s = &p.sources[0];
        assert_eq!(s.device_path, "/sdcard/Android/media/com.whatsapp");
        assert_eq!(s.alt_paths, vec!["/sdcard/WhatsApp"]);
    }

    #[test]
    fn dcim_extensions_non_empty() {
        let p = find_profile("dcim").unwrap();
        assert!(!p.sources[0].extensions.as_ref().unwrap().is_empty());
    }

    #[test]
    fn find_profile_lookup() {
        assert!(find_profile("whatsapp").is_some());
        assert!(find_profile("nope").is_none());
    }

    // --- is_build_artifact tests ---

    #[test]
    fn artifact_object_file() {
        assert!(is_build_artifact("foo.o"));
    }

    #[test]
    fn artifact_rlib() {
        assert!(is_build_artifact("libfoo.rlib"));
    }

    #[test]
    fn artifact_rmeta() {
        assert!(is_build_artifact("foo.rmeta"));
    }

    #[test]
    fn artifact_cargo_lock_files() {
        assert!(is_build_artifact(".rustc_info.json"));
        assert!(is_build_artifact(".cargo-lock"));
        assert!(is_build_artifact(".cargo-build-lock"));
        assert!(is_build_artifact(".cargo-artifact-lock"));
    }

    #[test]
    fn artifact_hex_fingerprint() {
        // 40-char lowercase hex = cargo fingerprint
        assert!(is_build_artifact("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0"));
    }

    #[test]
    fn artifact_hex_not_uppercase() {
        // Uppercase hex should NOT be flagged (could be legit)
        assert!(!is_build_artifact("A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6E7F8A9B0"));
    }

    #[test]
    fn artifact_target_dir() {
        assert!(is_build_artifact("target"));
    }

    #[test]
    fn artifact_build_script_outputs() {
        assert!(is_build_artifact("run-build-script-build-script-build"));
        assert!(is_build_artifact("rustix_test_can_compile"));
    }

    #[test]
    fn no_false_positive_photos() {
        assert!(!is_build_artifact("IMG_20260727_111036.jpg"));
        assert!(!is_build_artifact("msgstore.db.crypt15"));
        assert!(!is_build_artifact(".nomedia"));
        assert!(!is_build_artifact("DCIM"));
        assert!(!is_build_artifact("WhatsApp"));
    }

    #[test]
    fn no_false_positive_hex_not_exactly_40() {
        // 39 chars — too short
        assert!(!is_build_artifact("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b"));
        // 41 chars — too long
        assert!(!is_build_artifact("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b01"));
    }

    #[test]
    fn no_false_positive_hex_with_nonhex() {
        assert!(!is_build_artifact("g1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0"));
    }
}
