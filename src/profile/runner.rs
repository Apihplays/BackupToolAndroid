// Parallel, priority-ordered execution of backup profiles.
//
// `plan_execution` computes the start schedule; `ProfileRunner` spawns one
// thread per profile, each holding a token from a shared `GlobalBudget` so
// total concurrent workers stay bounded across profiles.
#![allow(dead_code)]

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::adb::client::AdbClient;
use crate::error::AppResult;
use crate::profile::ProfileSpec;
use crate::scanner::tree::{FileNode, Scanner};
use crate::state::StateManager;
use crate::transfer::engine::{TransferDirection, TransferEngine};

/// Stagger between consecutive profile starts.
const START_DELAY_STEP: Duration = Duration::from_secs(2);

/// Default maximum number of concurrently running transfer threads.
pub const DEFAULT_MAX_WORKERS: usize = 6;

/// Compute the start schedule for a set of profiles.
///
/// Profiles are sorted by priority ascending; the first starts immediately,
/// every subsequent one is staggered by an additional 2s per position index.
pub fn plan_execution(profiles: &[ProfileSpec]) -> Vec<(String, Duration)> {
    let mut sorted: Vec<&ProfileSpec> = profiles.iter().collect();
    sorted.sort_by_key(|p| p.priority);

    sorted
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let delay = START_DELAY_STEP * i as u32;
            (p.name.clone(), delay)
        })
        .collect()
}

/// A counting semaphore built on `Mutex<usize>` + `Condvar` (no new deps).
///
/// Acquiring blocks while the budget is exhausted; the permit is released
/// automatically when the returned guard is dropped.
pub struct GlobalBudget {
    available: std::sync::Mutex<usize>,
    cond: std::sync::Condvar,
}

impl GlobalBudget {
    pub fn new(max_workers: usize) -> Self {
        Self {
            available: std::sync::Mutex::new(max_workers.max(1)),
            cond: std::sync::Condvar::new(),
        }
    }

    /// Block until a worker slot is free, then hold it until drop.
    pub fn acquire(&self) -> BudgetToken<'_> {
        let mut available = self.available.lock().unwrap();
        while *available == 0 {
            available = self.cond.wait(available).unwrap();
        }
        *available -= 1;
        BudgetToken { budget: self }
    }

    fn release(&self) {
        let mut available = self.available.lock().unwrap();
        *available += 1;
        self.cond.notify_one();
    }
}

/// RAII guard representing one held worker slot.
pub struct BudgetToken<'a> {
    budget: &'a GlobalBudget,
}

impl Drop for BudgetToken<'_> {
    fn drop(&mut self) {
        self.budget.release();
    }
}

/// Result of running a single profile.
#[derive(Debug, Clone)]
pub struct ProfileOutcome {
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
    pub files_transferred: u64,
}

/// Runs multiple profiles in parallel with priority-ordered staggering and a
/// shared global worker budget.
pub struct ProfileRunner {
    max_workers: usize,
}

impl Default for ProfileRunner {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_WORKERS)
    }
}

impl ProfileRunner {
    pub fn new(max_workers: usize) -> Self {
        Self { max_workers }
    }

    /// Run all profiles in parallel, honoring priority-based start order and
    /// sharing a global pool of `max_workers` slots. Blocks until every
    /// profile has finished.
    pub fn run_all(
        &self,
        client: Arc<AdbClient>,
        profiles: Vec<ProfileSpec>,
        destination: &str,
    ) -> Vec<ProfileOutcome> {
        let plan = plan_execution(&profiles);
        let budget = Arc::new(GlobalBudget::new(self.max_workers));

        let handles: Vec<_> = profiles
            .into_iter()
            .map(|profile| {
                let delay = plan
                    .iter()
                    .find(|(name, _)| *name == profile.name)
                    .map(|(_, d)| *d)
                    .unwrap_or(Duration::ZERO);
                let client = Arc::clone(&client);
                let budget = Arc::clone(&budget);
                let destination = destination.to_string();

                thread::spawn(move || {
                    run_single_profile(client, profile, &destination, delay, &budget)
                })
            })
            .collect();

        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    }
}

/// Execute one profile: sleep its scheduled delay, scan its sources into a
/// file tree, then pull everything under a namespaced state file. Holds one
/// global budget token for the whole duration.
fn run_single_profile(
    client: Arc<AdbClient>,
    profile: ProfileSpec,
    destination: &str,
    delay: Duration,
    budget: &GlobalBudget,
) -> ProfileOutcome {
    // Hold a global worker slot for the whole duration of this profile.
    let _token = budget.acquire();

    if !delay.is_zero() {
        thread::sleep(delay);
    }

    match transfer_profile(&client, &profile, destination) {
        Ok(files) => ProfileOutcome {
            name: profile.name,
            success: true,
            error: None,
            files_transferred: files,
        },
        Err(e) => ProfileOutcome {
            name: profile.name,
            success: false,
            error: Some(e.to_string()),
            files_transferred: 0,
        },
    }
}

/// Build the device-side tree for a profile and pull all of it.
fn transfer_profile(
    client: &AdbClient,
    profile: &ProfileSpec,
    destination: &str,
) -> AppResult<u64> {
    let base_path = profile
        .sources
        .first()
        .map(|s| s.device_path.clone())
        .unwrap_or_else(|| "/sdcard".to_string());

    let mut tree = FileNode::root(&base_path);

    // Scan each source into the tree, rooted listing when required.
    for source in &profile.sources {
        let mut source_root = FileNode::root(&source.device_path);

        if profile.requires_root {
            scan_source_rooted(client, &source.device_path, &mut source_root)?;
        } else if source.recursive {
            Scanner::load_recursive(client, &mut source_root, MAX_SCAN_DEPTH)?;
        } else {
            Scanner::load_children(client, &mut source_root)?;
        }

        if let Some(exts) = &source.extensions {
            keep_extensions(&mut source_root, exts);
        }
        source_root.set_selected_recursive(true);
        source_root.compute_totals();

        tree.children.push(source_root);
    }
    tree.compute_totals();

    // Nothing matched — treat as a successful no-op.
    if tree.selected_file_count() == 0 {
        return Ok(0);
    }

    let engine = TransferEngine::new();
    let mut state_manager = StateManager::new_named(&base_path, destination, &profile.name);

    engine.execute(
        client,
        &tree,
        destination,
        &base_path,
        &mut state_manager,
        TransferDirection::Pull,
    )?;

    let progress = engine.progress.lock().unwrap();
    Ok(progress.completed_files)
}

/// Max recursion depth when scanning profile sources.
const MAX_SCAN_DEPTH: usize = 4;

/// Scan a source directory using rooted listing (su cat fallback).
fn scan_source_rooted(client: &AdbClient, path: &str, node: &mut FileNode) -> AppResult<()> {
    use crate::adb::client::RemoteEntry;

    // Rooted listing of the top level; deeper levels fall back to plain listing.
    let entries: Vec<RemoteEntry> = match client.list_dir_rooted(path) {
        Ok(entries) => entries,
        Err(_) => return Scanner::load_recursive(client, node, MAX_SCAN_DEPTH),
    };

    let depth = node.depth + 1;
    node.children = entries
        .iter()
        .map(|e| FileNode::from_entry(e, depth))
        .collect();
    node.loaded = true;

    for idx in 0..node.children.len() {
        if node.children[idx].is_dir {
            Scanner::load_recursive(client, &mut node.children[idx], MAX_SCAN_DEPTH)?;
        }
    }
    node.compute_totals();
    Ok(())
}

/// Recursively prune non-matching files from a scanned subtree.
fn keep_extensions(node: &mut FileNode, extensions: &[String]) {
    if node.is_dir {
        for child in &mut node.children {
            keep_extensions(child, extensions);
        }
        node.children.retain(|c| c.is_dir || c.selected);
    } else {
        let matches = node
            .name
            .rsplit('.')
            .next()
            .is_some_and(|ext| extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)));
        node.selected = matches;
    }
    node.compute_totals();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::SourceSpec;

    fn spec(name: &str, priority: u8) -> ProfileSpec {
        ProfileSpec {
            name: name.to_string(),
            priority,
            requires_root: false,
            sources: vec![SourceSpec {
                device_path: "/sdcard/x".to_string(),
                alt_paths: vec![],
                recursive: false,
                extensions: None,
            }],
        }
    }

    #[test]
    fn plan_empty_slice_yields_empty_plan() {
        assert!(plan_execution(&[]).is_empty());
    }

    #[test]
    fn plan_single_profile_starts_immediately() {
        let profiles = vec![spec("only", 7)];
        let plan = plan_execution(&profiles);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0, "only");
        assert_eq!(plan[0].1, Duration::ZERO);
    }

    #[test]
    fn plan_sorts_by_priority_and_staggers_two_seconds() {
        let profiles = vec![spec("mid", 5), spec("low", 0), spec("high", 9)];
        let plan = plan_execution(&profiles);
        assert_eq!(
            plan.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            vec!["low", "mid", "high"]
        );
        assert_eq!(plan[0].1, Duration::from_secs(0));
        assert_eq!(plan[1].1, Duration::from_secs(2));
        assert_eq!(plan[2].1, Duration::from_secs(4));
    }

    #[test]
    fn plan_is_stable_for_equal_priorities() {
        let profiles = vec![spec("a", 3), spec("b", 3)];
        let plan = plan_execution(&profiles);
        assert_eq!(plan[0].0, "a");
        assert_eq!(plan[1].0, "b");
    }

    #[test]
    fn budget_allows_up_to_max_concurrent_holders() {
        let budget = GlobalBudget::new(3);
        let t1 = budget.acquire();
        let t2 = budget.acquire();
        let t3 = budget.acquire();
        drop(t2);
        let t4 = budget.acquire(); // freed slot reused after drop
        drop(t1);
        drop(t3);
        drop(t4);
    }

    #[test]
    fn budget_blocks_at_max_then_releases_on_drop() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let budget = Arc::new(GlobalBudget::new(2));
        let inside = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let budget = Arc::clone(&budget);
            let inside = Arc::clone(&inside);
            let peak = Arc::clone(&peak);
            handles.push(thread::spawn(move || {
                let _token = budget.acquire();
                let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(20));
                inside.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        assert!(peak.load(Ordering::SeqCst) <= 2, "budget was exceeded");
    }

    #[test]
    fn default_runner_uses_suggested_worker_budget() {
        assert_eq!(DEFAULT_MAX_WORKERS, 6);
        assert_eq!(ProfileRunner::default().max_workers, 6);
    }
}
