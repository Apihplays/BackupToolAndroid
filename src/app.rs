#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::thread;
use std::path::PathBuf;
use std::fs;
use glob::Pattern;

use crate::adb::client::{AdbClient, DeviceInfo};
use crate::scanner::{FileNode, Scanner};
use crate::state::StateManager;
use crate::transfer::engine::{TransferEngine, TransferProgress};

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

    // File browser
    pub file_tree: Option<FileNode>,
    pub flat_tree: Vec<FlatNode>,
    pub browser_index: usize,
    pub media_filter: bool,
    pub search_query: String,
    pub destination: String,

    // Local Destination Browser
    pub local_browser_path: PathBuf,
    pub local_browser_items: Vec<String>,
    pub local_browser_index: usize,

    // Transfer
    pub transfer_engine: Option<TransferEngine>,
    pub transfer_progress: Arc<Mutex<TransferProgress>>,
    pub transfer_thread: Option<thread::JoinHandle<()>>,

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
            file_tree: None,
            flat_tree: Vec::new(),
            browser_index: 0,
            media_filter: false,
            search_query: String::new(),
            destination: destination.clone(),
            local_browser_path: PathBuf::from(destination),
            local_browser_items: Vec::new(),
            local_browser_index: 0,
            transfer_engine: None,
            transfer_progress: Arc::new(Mutex::new(TransferProgress::new(0, 0))),
            transfer_thread: None,
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
                self.status_message = format!("Cannot connect: device is '{}'. Check authorization on phone.", device.state);
                return;
            }
            self.adb_client.select_device(device);
            self.current_view = AppView::FileBrowser;
            self.load_file_tree();
        }
    }

    // === File Browser ===

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
                
                visible.into_iter().filter(|node| {
                    if let Ok(ref pat) = pattern {
                        pat.matches(&node.name)
                    } else {
                        node.name.to_lowercase().contains(&query_lower)
                    }
                }).collect()
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

    pub fn browser_next(&mut self) {
        if !self.flat_tree.is_empty() {
            self.browser_index = (self.browser_index + 1).min(self.flat_tree.len() - 1);
        }
    }

    pub fn browser_prev(&mut self) {
        self.browser_index = self.browser_index.saturating_sub(1);
    }

    pub fn browser_toggle_expand(&mut self) {
        if let Some(flat_node) = self.flat_tree.get(self.browser_index) {
            if flat_node.is_dir {
                let path = flat_node.path.clone();
                if let Some(ref mut tree) = self.file_tree {
                    if let Some(node) = find_node_mut(tree, &path) {
                        if !node.loaded {
                            // Lazy-load children
                            let _ = Scanner::load_children(&self.adb_client, node);
                        }
                        node.expanded = !node.expanded;
                    }
                }
                self.rebuild_flat_tree();
            }
        }
    }

    pub fn browser_toggle_select(&mut self) {
        if let Some(flat_node) = self.flat_tree.get(self.browser_index) {
            let path = flat_node.path.clone();
            if let Some(ref mut tree) = self.file_tree {
                if let Some(node) = find_node_mut(tree, &path) {
                    let new_selected = !node.selected;
                    node.set_selected_recursive(new_selected);
                }
            }
            self.rebuild_flat_tree();
        }
    }

    pub fn browser_select_all(&mut self) {
        if let Some(ref mut tree) = self.file_tree {
            tree.set_selected_recursive(true);
        }
        self.rebuild_flat_tree();
    }

    pub fn browser_select_none(&mut self) {
        if let Some(ref mut tree) = self.file_tree {
            tree.set_selected_recursive(false);
        }
        self.rebuild_flat_tree();
    }

    pub fn browser_go_back(&mut self) {
        self.current_view = AppView::DeviceSelect;
    }

    pub fn toggle_media_filter(&mut self) {
        self.media_filter = !self.media_filter;
        self.rebuild_flat_tree();
    }

    // === Local Destination Browser ===

    pub fn load_local_browser(&mut self) {
        self.local_browser_items.clear();
        self.local_browser_index = 0;
        
        self.local_browser_items.push("[Select Current Directory]".to_string());
        self.local_browser_items.push("..".to_string());

        if let Ok(entries) = fs::read_dir(&self.local_browser_path) {
            let mut dirs = Vec::new();
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        if let Ok(name) = entry.file_name().into_string() {
                            dirs.push(name);
                        }
                    }
                }
            }
            dirs.sort_unstable();
            self.local_browser_items.extend(dirs);
        }
    }

    pub fn local_browser_next(&mut self) {
        if !self.local_browser_items.is_empty() {
            self.local_browser_index = (self.local_browser_index + 1) % self.local_browser_items.len();
        }
    }

    pub fn local_browser_prev(&mut self) {
        if !self.local_browser_items.is_empty() {
            self.local_browser_index = if self.local_browser_index == 0 {
                self.local_browser_items.len() - 1
            } else {
                self.local_browser_index - 1
            };
        }
    }

    pub fn local_browser_enter(&mut self) {
        if self.local_browser_items.is_empty() {
            return;
        }
        let selected = &self.local_browser_items[self.local_browser_index];
        if selected == "[Select Current Directory]" {
            self.destination = self.local_browser_path.to_string_lossy().into_owned();
            self.current_view = AppView::FileBrowser;
        } else if selected == ".." {
            if let Some(parent) = self.local_browser_path.parent() {
                self.local_browser_path = parent.to_path_buf();
                self.load_local_browser();
            }
        } else {
            self.local_browser_path.push(selected);
            self.load_local_browser();
        }
    }

    // === Transfer ===

    pub fn start_transfer(&mut self) {
        let selected_count = self.file_tree.as_ref()
            .map(|t| t.selected_file_count())
            .unwrap_or(0);

        if selected_count == 0 {
            self.status_message = "No files selected!".into();
            return;
        }

        self.current_view = AppView::Transferring;

        let engine = TransferEngine::new();
        self.transfer_progress = engine.progress.clone();

        // Clone what we need for the transfer thread
        let tree = self.file_tree.clone().unwrap();
        let destination = self.destination.clone();
        let progress = engine.progress.clone();
        let adb_client = AdbClient::new();

        // Copy device selection to new client
        if let Some(ref device) = self.adb_client.selected_device {
            let mut client = adb_client;
            client.select_device(device.clone());

            let handle = thread::spawn(move || {
                let mut state_manager = StateManager::new(&tree.path, &destination);
                let engine = TransferEngine { progress };

                if let Err(e) = engine.execute(&client, &tree, &destination, &mut state_manager) {
                    let mut p = engine.progress.lock().unwrap();
                    p.errors.push(("FATAL".into(), e.to_string()));
                    p.is_complete = true;
                }
            });

            self.transfer_thread = Some(handle);
        }

        self.transfer_engine = Some(engine);
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
    }

    pub fn summary_scroll_up(&mut self) {
        self.summary_scroll = self.summary_scroll.saturating_sub(1);
    }

    pub fn summary_scroll_down(&mut self) {
        self.summary_scroll += 1;
    }
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
