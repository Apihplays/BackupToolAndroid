// Backup profile specifications.
// Scaffolding: consumed by upcoming transfer/scanner integration.
#![allow(dead_code)]

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
}
