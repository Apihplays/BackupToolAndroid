#![allow(dead_code)]

use glob::Pattern;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::adb::client::{AdbClient, DeviceInfo};
use crate::scanner::{FileNode, LocalScanner, Scanner};
use crate::state::StateManager;
use crate::transfer::engine::{TransferDirection, TransferEngine, TransferProgress};

#[derive(Debug, Clone, PartialEq)]
pub enum Pane {
    Left,  // Android
    Right, // PC
}

/// The current view state of the application.
#[derive(Debug, Clone, PartialEq)]
pub enum AppView {
    DeviceSelect,
    FileBrowser,
    FileBrowserSearch,
    DestinationBrowser,
    Transferring,
    Summary,
}

/// Flattened tree node for TUI rendering.
#[derive(Debug, Clone)]
pub struct FlatNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub total_size: u64,
    pub selected: bool,
    pub expanded: bool,
    pub depth: usize,
    pub file_count: u64,
    pub tree_index: Vec<usize>, // path of indices into tree for mutation
}

/// Main application state.
pub struct App {
    pub current_view: AppView,
    pub should_quit: bool,

    // ADB
    pub adb_client: AdbClient,

    // Device selection
    pub devices: Vec<DeviceInfo>,
    pub device_list_index: usize,

    pub active_pane: Pane,

    // File browser (Android)
    pub file_tree: Option<FileNode>,
    pub flat_tree: Vec<FlatNode>,
    pub browser_index: usize,

    // File browser (Local PC)
    pub local_tree: Option<FileNode>,
    pub local_flat_tree: Vec<FlatNode>,
    pub local_browser_index: usize,

    // Shared filters
    pub media_filter: bool,
    pub search_query: String,
    pub destination: String,

    // Transfer
    pub transfer_engine: Option<TransferEngine>,
    pub transfer_progress: Arc<Mutex<TransferProgress>>,
    pub transfer_thread: Option<thread::JoinHandle<()>>,
    pub last_transfer_direction: Option<TransferDirection>,

    // Summary
    pub summary_scroll: u16,

    // Status
    pub status_message: String,
    pub is_loading: bool,
}

impl App {
    pub fn new(destination: String) -> Self {
        Self {
            current_view: AppView::DeviceSelect,
            should_quit: false,
            adb_client: AdbClient::new(),
            devices: Vec::new(),
            device_list_index: 0,
            active_pane: Pane::Left,
            file_tree: None,
            flat_tree: Vec::new(),
            browser_index: 0,
            local_tree: None,
            local_flat_tree: Vec::new(),
            local_browser_index: 0,
            media_filter: false,
            search_query: String::new(),
            destination: destination.clone(),
            transfer_engine: None,
            transfer_progress: Arc::new(Mutex::new(TransferProgress::new(0, 0))),
            transfer_thread: None,
            last_transfer_direction: None,
            summary_scroll: 0,
            status_message: String::new(),
            is_loading: false,
        }
    }

    /// Initialize — load devices.
    pub fn init(&mut self) {
        self.refresh_devices();
    }

    // === Device Selection ===

    pub fn refresh_devices(&mut self) {
        match self.adb_client.list_devices() {
            Ok(devices) => {
                self.devices = devices;
                self.device_list_index = 0;
                if self.devices.len() == 1 {
                    // Auto-select if only one device
                    self.select_device();
                }
            }
            Err(e) => {
                self.status_message = format!("Error: {}", e);
                self.devices.clear();
            }
        }
    }

    pub fn device_list_next(&mut self) {
        if !self.devices.is_empty() {
            self.device_list_index = (self.device_list_index + 1) % self.devices.len();
        }
    }

    pub fn device_list_prev(&mut self) {
        if !self.devices.is_empty() {
            self.device_list_index = if self.device_list_index == 0 {
                self.devices.len() - 1
            } else {
                self.device_list_index - 1
            };
        }
    }

    pub fn select_device(&mut self) {
        if let Some(device) = self.devices.get(self.device_list_index).cloned() {
            if device.state != "device" {
                self.status_message = format!(
                    "Cannot connect: device is '{}'. Check authorization on phone.",
                    device.state
                );
                return;
            }
            self.adb_client.select_device(device);
            self.current_view = AppView::FileBrowser;
            self.load_file_tree();
            self.load_local_tree();
        }
    }

    // === File Browser (Dual Pane) ===

    fn load_local_tree(&mut self) {
        match LocalScanner::build_tree(&self.destination) {
            Ok(tree) => {
                self.local_tree = Some(tree);
                self.rebuild_local_flat_tree();
            }
            Err(e) => {
                self.status_message = format!("Error loading local tree: {}", e);
            }
        }
    }

    fn load_file_tree(&mut self) {
        self.is_loading = true;
        self.status_message = "Loading file tree...".into();

        match Scanner::build_media_tree(&self.adb_client) {
            Ok(tree) => {
                self.file_tree = Some(tree);
                self.rebuild_flat_tree();
                self.is_loading = false;
                self.status_message.clear();
            }
            Err(e) => {
                self.status_message = format!("Error loading tree: {}", e);
                self.is_loading = false;
            }
        }
    }

    /// Rebuild the flat tree from the current file tree state.
    fn rebuild_flat_tree(&mut self) {
        self.flat_tree.clear();
        if let Some(ref tree) = self.file_tree {
            let visible = tree.flatten_visible(self.media_filter);

            let filtered: Vec<_> = if !self.search_query.is_empty() {
                let pattern = Pattern::new(&self.search_query);
                let query_lower = self.search_query.to_lowercase();

                visible
                    .into_iter()
                    .filter(|node| {
                        if let Ok(ref pat) = pattern {
                            pat.matches(&node.name)
                        } else {
                            node.name.to_lowercase().contains(&query_lower)
                        }
                    })
                    .collect()
            } else {
                visible
            };

            self.flat_tree = filtered
                .into_iter()
                .map(|node| FlatNode {
                    name: node.name.clone(),
                    path: node.path.clone(),
                    is_dir: node.is_dir,
                    size: node.size,
                    total_size: node.total_size,
                    selected: node.selected,
                    expanded: node.expanded,
                    depth: node.depth,
                    file_count: node.file_count,
                    tree_index: Vec::new(), // simplified — we use path-based lookups
                })
                .collect();
        }

        // Ensure browser index is in bounds
        if self.browser_index >= self.flat_tree.len() {
            self.browser_index = self.flat_tree.len().saturating_sub(1);
        }
    }

    fn rebuild_local_flat_tree(&mut self) {
        self.local_flat_tree.clear();
        if let Some(ref tree) = self.local_tree {
            let visible = tree.flatten_visible(self.media_filter);

            let filtered: Vec<_> = if !self.search_query.is_empty() {
                let pattern = Pattern::new(&self.search_query);
                let query_lower = self.search_query.to_lowercase();

                visible
                    .into_iter()
                    .filter(|node| {
                        if let Ok(ref pat) = pattern {
                            pat.matches(&node.name)
                        } else {
                            node.name.to_lowercase().contains(&query_lower)
                        }
                    })
                    .collect()
            } else {
                visible
            };

            self.local_flat_tree = filtered
                .into_iter()
                .map(|node| FlatNode {
                    name: node.name.clone(),
                    path: node.path.clone(),
                    is_dir: node.is_dir,
                    size: node.size,
                    total_size: node.total_size,
                    selected: node.selected,
                    expanded: node.expanded,
                    depth: node.depth,
                    file_count: node.file_count,
                    tree_index: Vec::new(),
                })
                .collect();
        }

        if self.local_browser_index >= self.local_flat_tree.len() {
            self.local_browser_index = self.local_flat_tree.len().saturating_sub(1);
        }
    }

    pub fn browser_next(&mut self) {
        match self.active_pane {
            Pane::Left => {
                if !self.flat_tree.is_empty() {
                    self.browser_index = (self.browser_index + 1).min(self.flat_tree.len() - 1);
                }
            }
            Pane::Right => {
                if !self.local_flat_tree.is_empty() {
                    self.local_browser_index =
                        (self.local_browser_index + 1).min(self.local_flat_tree.len() - 1);
                }
            }
        }
    }

    pub fn browser_prev(&mut self) {
        match self.active_pane {
            Pane::Left => {
                self.browser_index = self.browser_index.saturating_sub(1);
            }
            Pane::Right => {
                self.local_browser_index = self.local_browser_index.saturating_sub(1);
            }
        }
    }

    pub fn browser_toggle_expand(&mut self) {
        match self.active_pane {
            Pane::Left => {
                if let Some(flat_node) = self.flat_tree.get(self.browser_index) {
                    if flat_node.is_dir {
                        let path = flat_node.path.clone();
                        if let Some(ref mut tree) = self.file_tree {
                            if let Some(node) = find_node_mut(tree, &path) {
                                if !node.loaded {
                                    let _ = Scanner::load_children(&self.adb_client, node);
                                }
                                node.expanded = !node.expanded;
                            }
                        }
                        self.rebuild_flat_tree();
                    }
                }
            }
            Pane::Right => {
                if let Some(flat_node) = self.local_flat_tree.get(self.local_browser_index) {
                    if flat_node.is_dir {
                        let path = flat_node.path.clone();
                        if let Some(ref mut tree) = self.local_tree {
                            if path == ".." && flat_node.name == ".." {
                                // Navigate up
                                let current = std::path::PathBuf::from(&self.destination);
                                if let Some(parent) = current.parent() {
                                    self.destination = parent.to_string_lossy().to_string();
                                    self.load_local_tree();
                                }
                            } else if let Some(node) = find_node_mut(tree, &path) {
                                if !node.loaded {
                                    let _ = LocalScanner::load_children(node);
                                }
                                node.expanded = !node.expanded;
                            }
                        }
                        self.rebuild_local_flat_tree();
                    }
                }
            }
        }
    }

    pub fn browser_toggle_select(&mut self) {
        match self.active_pane {
            Pane::Left => {
                if let Some(flat_node) = self.flat_tree.get(self.browser_index) {
                    let path = flat_node.path.clone();
                    if let Some(ref mut tree) = self.file_tree {
                        if let Some(node) = find_node_mut(tree, &path) {
                            if node.is_dir && !node.loaded {
                                let _ = crate::scanner::tree::Scanner::load_recursive(
                                    &self.adb_client,
                                    node,
                                    node.depth + 10,
                                );
                            }
                            let new_selected = !node.selected;
                            node.set_selected_recursive(new_selected);
                        }
                    }
                    self.rebuild_flat_tree();
                }
            }
            Pane::Right => {
                if let Some(flat_node) = self.local_flat_tree.get(self.local_browser_index) {
                    let path = flat_node.path.clone();
                    if let Some(ref mut tree) = self.local_tree {
                        if let Some(node) = find_node_mut(tree, &path) {
                            if node.is_dir && !node.loaded {
                                let _ = crate::scanner::local::LocalScanner::load_recursive(
                                    node,
                                    node.depth + 10,
                                );
                            }
                            let new_selected = !node.selected;
                            node.set_selected_recursive(new_selected);
                        }
                    }
                    self.rebuild_local_flat_tree();
                }
            }
        }
    }

    pub fn browser_select_all(&mut self) {
        match self.active_pane {
            Pane::Left => {
                if let Some(ref mut tree) = self.file_tree {
                    // Deselect everything first to ensure clean state
                    tree.set_selected_recursive(false);
                    // Select only nodes that are visible in the currently filtered flat_tree
                    for flat_node in &self.flat_tree {
                        if let Some(node) = find_node_mut(tree, &flat_node.path) {
                            if node.is_dir && !node.loaded {
                                let _ = crate::scanner::tree::Scanner::load_recursive(
                                    &self.adb_client,
                                    node,
                                    node.depth + 10,
                                );
                            }
                            node.set_selected_recursive(true);
                        }
                    }
                }
                self.rebuild_flat_tree();
            }
            Pane::Right => {
                if let Some(ref mut tree) = self.local_tree {
                    // Deselect everything first to ensure clean state
                    tree.set_selected_recursive(false);
                    // Select only nodes that are visible in the currently filtered local_flat_tree
                    for flat_node in &self.local_flat_tree {
                        if let Some(node) = find_node_mut(tree, &flat_node.path) {
                            if node.is_dir && !node.loaded {
                                let _ = crate::scanner::local::LocalScanner::load_recursive(
                                    node,
                                    node.depth + 10,
                                );
                            }
                            node.set_selected_recursive(true);
                        }
                    }
                }
                self.rebuild_local_flat_tree();
            }
        }
    }

    pub fn browser_select_none(&mut self) {
        match self.active_pane {
            Pane::Left => {
                if let Some(ref mut tree) = self.file_tree {
                    tree.set_selected_recursive(false);
                }
                self.rebuild_flat_tree();
            }
            Pane::Right => {
                if let Some(ref mut tree) = self.local_tree {
                    tree.set_selected_recursive(false);
                }
                self.rebuild_local_flat_tree();
            }
        }
    }

    pub fn toggle_pane(&mut self) {
        self.active_pane = match self.active_pane {
            Pane::Left => Pane::Right,
            Pane::Right => Pane::Left,
        };
    }

    pub fn browser_go_back(&mut self) {
        self.current_view = AppView::DeviceSelect;
    }

    pub fn toggle_media_filter(&mut self) {
        self.media_filter = !self.media_filter;
        self.rebuild_flat_tree();
        self.rebuild_local_flat_tree();
    }

    /// Re-filter the flat trees based on the current search query.
    pub fn apply_search_filter(&mut self) {
        self.rebuild_flat_tree();
        self.rebuild_local_flat_tree();
    }

    // === Transfer & File Operations ===

    pub fn start_transfer(&mut self) {
        // Safety net: ensure any selected but unloaded directories are fully loaded before transfer
        match self.active_pane {
            Pane::Left => {
                let mut paths_to_load = Vec::new();
                if let Some(ref tree) = self.file_tree {
                    for dir in tree.selected_dirs() {
                        if !dir.loaded {
                            paths_to_load.push(dir.path.clone());
                        }
                    }
                }
                if let Some(ref mut tree) = self.file_tree {
                    for path in paths_to_load {
                        if let Some(node) = find_node_mut(tree, &path) {
                            let _ = crate::scanner::tree::Scanner::load_recursive(
                                &self.adb_client,
                                node,
                                usize::MAX,
                            );
                            node.set_selected_recursive(true);
                        }
                    }
                }
            }
            Pane::Right => {
                let mut paths_to_load = Vec::new();
                if let Some(ref tree) = self.local_tree {
                    for dir in tree.selected_dirs() {
                        if !dir.loaded {
                            paths_to_load.push(dir.path.clone());
                        }
                    }
                }
                if let Some(ref mut tree) = self.local_tree {
                    for path in paths_to_load {
                        if let Some(node) = find_node_mut(tree, &path) {
                            let _ = crate::scanner::local::LocalScanner::load_recursive(
                                node,
                                usize::MAX,
                            );
                            node.set_selected_recursive(true);
                        }
                    }
                }
            }
        }

        let (tree, direction) = match self.active_pane {
            Pane::Left => {
                let selected_count = self
                    .file_tree
                    .as_ref()
                    .map(|t| t.selected_file_count())
                    .unwrap_or(0);
                if selected_count == 0 {
                    self.status_message = "No remote files selected to pull!".into();
                    return;
                }
                (self.file_tree.clone().unwrap(), TransferDirection::Pull)
            }
            Pane::Right => {
                let selected_count = self
                    .local_tree
                    .as_ref()
                    .map(|t| t.selected_file_count())
                    .unwrap_or(0);
                if selected_count == 0 {
                    self.status_message = "No local files selected to push!".into();
                    return;
                }
                (self.local_tree.clone().unwrap(), TransferDirection::Push)
            }
        };

        self.current_view = AppView::Transferring;
        self.last_transfer_direction = Some(direction);

        let engine = TransferEngine::new();
        self.transfer_progress = engine.progress.clone();

        let destination = match direction {
            TransferDirection::Pull => self.destination.clone(),
            TransferDirection::Push => {
                let path = if let Some(flat_node) = self.flat_tree.get(self.browser_index) {
                    if flat_node.is_dir {
                        flat_node.path.clone()
                    } else {
                        if let Some(pos) = flat_node.path.rfind('/') {
                            if pos == 0 {
                                "/".to_string()
                            } else {
                                flat_node.path[..pos].to_string()
                            }
                        } else {
                            "/sdcard".to_string()
                        }
                    }
                } else {
                    "/sdcard".to_string()
                };

                if path == "/" {
                    "/sdcard".to_string()
                } else {
                    path
                }
            }
        };
        let base_path = tree.path.clone();
        let progress = engine.progress.clone();
        let adb_client = AdbClient::new();

        if let Some(ref device) = self.adb_client.selected_device {
            let mut client = adb_client;
            client.select_device(device.clone());

            let handle = thread::spawn(move || {
                let mut state_manager = StateManager::new(&base_path, &destination);
                let engine = TransferEngine { progress };

                if let Err(e) = engine.execute(
                    &client,
                    &tree,
                    &destination,
                    &base_path,
                    &mut state_manager,
                    direction,
                ) {
                    let mut p = engine.progress.lock().unwrap();
                    p.errors.push(("FATAL".into(), e.to_string()));
                    p.is_complete = true;
                    p.end_time = Some(std::time::Instant::now());
                }
            });

            self.transfer_thread = Some(handle);
        }

        self.transfer_engine = Some(engine);
    }

    pub fn delete_selected(&mut self) {
        match self.active_pane {
            Pane::Left => {
                if let Some(ref tree) = self.file_tree {
                    let files = tree.selected_files();
                    let dirs = tree.selected_dirs();
                    if files.is_empty() && dirs.is_empty() {
                        self.status_message = "No remote files selected to delete!".into();
                        return;
                    }

                    self.status_message =
                        format!("Deleting {} items from device...", files.len() + dirs.len());

                    for f in files.iter().chain(dirs.iter()) {
                        let _ = self.adb_client.rm_remote(&f.path);
                    }

                    self.status_message = "Remote deletion complete.".into();
                }
                self.load_file_tree();
            }
            Pane::Right => {
                if let Some(ref tree) = self.local_tree {
                    let files = tree.selected_files();
                    let dirs = tree.selected_dirs();
                    if files.is_empty() && dirs.is_empty() {
                        self.status_message = "No local files selected to delete!".into();
                        return;
                    }

                    self.status_message =
                        format!("Deleting {} items locally...", files.len() + dirs.len());

                    for f in files {
                        let _ = std::fs::remove_file(&f.path);
                    }
                    // Sort dirs by depth descending to delete children before parents
                    let mut sorted_dirs: Vec<_> = dirs.into_iter().collect();
                    sorted_dirs.sort_by_key(|b| std::cmp::Reverse(b.depth));
                    for d in sorted_dirs {
                        let _ = std::fs::remove_dir_all(&d.path);
                    }

                    self.status_message = "Local deletion complete.".into();
                }
                self.load_local_tree();
            }
        }
    }

    pub fn resume_transfer(&mut self) {
        if let Some(state_manager) = StateManager::load_existing(&self.destination) {
            self.status_message = format!(
                "Resume: {} files already completed",
                state_manager.stats().0
            );

            // Start transfer with existing state (will skip completed files)
            self.start_transfer();
        } else {
            self.status_message = "No previous transfer state found to resume.".into();
        }
    }

    pub fn cancel_transfer(&mut self) {
        if let Ok(mut progress) = self.transfer_progress.lock() {
            progress.is_cancelled = true;
            if progress.end_time.is_none() {
                progress.end_time = Some(std::time::Instant::now());
            }
        }
    }

    pub fn update_transfer_progress(&mut self) {
        let is_complete = self
            .transfer_progress
            .lock()
            .map(|p| p.is_complete)
            .unwrap_or(false);

        if is_complete {
            // Wait for transfer thread to finish
            if let Some(handle) = self.transfer_thread.take() {
                let _ = handle.join();
            }
            self.current_view = AppView::Summary;
        }
    }

    pub fn transfer_progress_snapshot(&self) -> TransferProgress {
        self.transfer_progress
            .lock()
            .map(|p| p.clone())
            .unwrap_or_else(|_| TransferProgress::new(0, 0))
    }

    // === Summary ===

    pub fn go_to_browser(&mut self) {
        self.current_view = AppView::FileBrowser;
        self.summary_scroll = 0;
        match self.last_transfer_direction {
            Some(TransferDirection::Pull) => {
                self.load_local_tree();
            }
            Some(TransferDirection::Push) => {
                self.load_file_tree();
            }
            None => {
                self.load_local_tree();
            }
        }
    }

    pub fn summary_scroll_up(&mut self) {
        self.summary_scroll = self.summary_scroll.saturating_sub(1);
    }

    pub fn summary_scroll_down(&mut self) {
        self.summary_scroll += 1;
    }

    // Preview functionality removed
}

/// Find a node in the tree by path (mutable).
fn find_node_mut<'a>(node: &'a mut FileNode, path: &str) -> Option<&'a mut FileNode> {
    if node.path == path {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_node_mut(child, path) {
            return Some(found);
        }
    }
    None
}
