use crate::adb::client::{AdbClient, RemoteEntry};
use crate::error::AppResult;

/// Media file extensions to filter for.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "raw", "cr2", "nef", "arw", "dng",
    "svg", "tiff", "tif", // Video
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "3gp", "ts", // Audio
    "mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "opus",
    // Documents (sometimes considered media)
    "pdf",
];

/// A node in the device file tree.
#[derive(Debug, Clone)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub children: Vec<FileNode>,
    pub selected: bool,
    pub expanded: bool,
    pub loaded: bool, // whether children have been fetched
    pub depth: usize,
    pub total_size: u64, // aggregate size including children
    pub file_count: u64, // number of files (recursive)
}

impl FileNode {
    pub fn from_entry(entry: &RemoteEntry, depth: usize) -> Self {
        Self {
            name: entry.name.clone(),
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            size: entry.size,
            mtime: entry.mtime,
            children: Vec::new(),
            selected: false,
            expanded: false,
            loaded: false,
            depth,
            total_size: entry.size,
            file_count: if entry.is_dir { 0 } else { 1 },
        }
    }

    /// Create a root node for a given path.
    pub fn root(path: &str) -> Self {
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        Self {
            name,
            path: path.to_string(),
            is_dir: true,
            size: 0,
            mtime: 0,
            children: Vec::new(),
            selected: false,
            expanded: true,
            loaded: false,
            depth: 0,
            total_size: 0,
            file_count: 0,
        }
    }

    /// Check if a file has a media extension.
    pub fn is_media(&self) -> bool {
        if self.is_dir {
            return true; // always show directories
        }
        let ext = self.name.rsplit('.').next().unwrap_or("").to_lowercase();
        MEDIA_EXTENSIONS.contains(&ext.as_str())
    }

    /// Recursively select/deselect this node and all children.
    pub fn set_selected_recursive(&mut self, selected: bool) {
        self.selected = selected;
        for child in &mut self.children {
            child.set_selected_recursive(selected);
        }
    }

    /// Get all selected file paths (not directories) recursively.
    pub fn selected_files(&self) -> Vec<&FileNode> {
        let mut result = Vec::new();
        if self.selected && !self.is_dir {
            result.push(self);
        }
        for child in &self.children {
            result.extend(child.selected_files());
        }
        result
    }

    /// Get all selected directory paths recursively.
    pub fn selected_dirs(&self) -> Vec<&FileNode> {
        let mut result = Vec::new();
        if self.selected && self.is_dir {
            result.push(self);
        }
        for child in &self.children {
            result.extend(child.selected_dirs());
        }
        result
    }

    /// Compute total size recursively.
    pub fn compute_totals(&mut self) {
        if self.is_dir {
            let mut total = 0u64;
            let mut count = 0u64;
            for child in &mut self.children {
                child.compute_totals();
                total += child.total_size;
                count += child.file_count;
            }
            self.total_size = total;
            self.file_count = count;
        } else {
            self.total_size = self.size;
            self.file_count = 1;
        }
    }

    /// Flatten the tree into a visible list for TUI rendering.
    /// Only includes expanded nodes' children.
    pub fn flatten_visible(&self, media_filter: bool) -> Vec<&FileNode> {
        let mut result = Vec::new();
        self.flatten_into(&mut result, media_filter);
        result
    }

    fn flatten_into<'a>(&'a self, result: &mut Vec<&'a FileNode>, media_filter: bool) {
        if media_filter && !self.is_media() {
            return;
        }
        result.push(self);
        if self.is_dir && self.expanded {
            for child in &self.children {
                child.flatten_into(result, media_filter);
            }
        }
    }

    /// Get total selected size.
    pub fn selected_total_size(&self) -> u64 {
        let mut total = 0u64;
        if self.selected && !self.is_dir {
            total += self.size;
        }
        for child in &self.children {
            total += child.selected_total_size();
        }
        total
    }

    /// Get total selected file count.
    pub fn selected_file_count(&self) -> u64 {
        let mut count = 0u64;
        if self.selected && !self.is_dir {
            count += 1;
        }
        for child in &self.children {
            count += child.selected_file_count();
        }
        count
    }
}

/// Scanner that walks the device file system and builds the tree.
pub struct Scanner;

impl Scanner {
    /// Load children for a directory node from the device.
    ///
    /// Tries a normal `ls -la` first; when that returns no entries (or fails
    /// with a permission error) and root is available, retries via
    /// `su -c 'ls -1ap'` so that protected directories (e.g. Android 16
    /// scoped-storage paths) are still accessible.
    pub fn load_children(client: &AdbClient, node: &mut FileNode) -> AppResult<()> {
        if !node.is_dir || node.loaded {
            return Ok(());
        }

        let entries = match client.list_dir(&node.path) {
            Ok(e) if !e.is_empty() => e,
            _ => client.list_dir_rooted(&node.path)?,
        };
        let depth = node.depth + 1;

        node.children = entries
            .iter()
            .map(|e| FileNode::from_entry(e, depth))
            .collect();

        // Sort: directories first, then alphabetical
        node.children.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        node.loaded = true;
        node.compute_totals();

        Ok(())
    }

    /// Recursively load all children up to a max depth.
    pub fn load_recursive(
        client: &AdbClient,
        node: &mut FileNode,
        max_depth: usize,
    ) -> AppResult<()> {
        if node.depth >= max_depth {
            return Ok(());
        }

        Self::load_children(client, node)?;

        let paths: Vec<usize> = node
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_dir)
            .map(|(i, _)| i)
            .collect();

        for idx in paths {
            Self::load_recursive(client, &mut node.children[idx], max_depth)?;
        }

        node.compute_totals();
        Ok(())
    }

    /// Build the initial tree with common media directories.
    pub fn build_media_tree(client: &AdbClient) -> AppResult<FileNode> {
        let mut root = FileNode::root("/");
        root.name = "Device Root".to_string();
        root.depth = 0;

        // 1. Internal Storage
        let mut internal = FileNode::root("/sdcard");
        internal.name = "Internal Storage".to_string();
        internal.depth = 1;
        internal.expanded = false;

        // Prepare children list
        let mut children = vec![internal];

        // 2. Discover physical SD cards in /storage
        let cmd = "ls -1 /storage 2>/dev/null";
        if let Ok(output) = client.shell_command(cmd) {
            for line in output.lines() {
                let name = line.trim();
                // SD cards typically have a format like XXXX-XXXX (hexadecimal)
                if name.len() == 9 && name.chars().nth(4) == Some('-') {
                    let mut sdcard = FileNode::root(&format!("/storage/{}", name));
                    sdcard.name = format!("SD Card ({})", name);
                    sdcard.depth = 1;
                    sdcard.expanded = false;
                    children.push(sdcard);
                }
            }
        }

        // Pre-load the first level for better UX
        for child in &mut children {
            let _ = Self::load_children(client, child);
        }

        root.children = children;
        root.loaded = true;
        root.compute_totals();

        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_node(name: &str, size: u64) -> FileNode {
        let mut n = FileNode::root(name);
        n.is_dir = false;
        n.size = size;
        n
    }

    #[test]
    fn compute_totals_sums_children() {
        let mut root = FileNode::root("/sdcard");
        root.children.push(file_node("a.jpg", 100));
        root.children.push(file_node("b.mp4", 250));

        root.compute_totals();

        assert_eq!(root.total_size, 350);
        assert_eq!(root.file_count, 2);
    }
}
