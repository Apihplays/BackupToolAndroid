use std::fs;
use crate::error::{AppError, AppResult};
use crate::scanner::tree::FileNode;

pub struct LocalScanner;

impl LocalScanner {
    pub fn load_children(node: &mut FileNode) -> AppResult<()> {
        if !node.is_dir || node.loaded {
            return Ok(());
        }

        let entries = match fs::read_dir(&node.path) {
            Ok(e) => e,
            Err(e) => return Err(AppError::Io(e)),
        };

        let depth = node.depth + 1;
        
        // Don't clear children if we already manually added ".."
        let mut new_children = Vec::new();
        for child in &node.children {
            if child.name == ".." {
                new_children.push(child.clone());
            }
        }
        node.children = new_children;

        for entry in entries.flatten() {
            let metadata = entry.metadata().ok();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = metadata.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path().to_string_lossy().to_string();

            let child = FileNode {
                name,
                path,
                is_dir,
                size,
                mtime,
                children: Vec::new(),
                selected: false,
                expanded: false,
                loaded: false,
                depth,
                total_size: size,
                file_count: if is_dir { 0 } else { 1 },
            };
            
            node.children.push(child);
        }

        // Sort: directories first, then alphabetical (ignoring "..")
        node.children.sort_by(|a, b| {
            if a.name == ".." {
                std::cmp::Ordering::Less
            } else if b.name == ".." {
                std::cmp::Ordering::Greater
            } else {
                b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }
        });

        node.loaded = true;
        node.compute_totals();

        Ok(())
    }

    pub fn load_recursive(node: &mut FileNode, max_depth: usize) -> AppResult<()> {
        if node.depth >= max_depth {
            return Ok(());
        }

        Self::load_children(node)?;

        let paths: Vec<usize> = node
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_dir && c.name != "..")
            .map(|(i, _)| i)
            .collect();

        for idx in paths {
            Self::load_recursive(&mut node.children[idx], max_depth)?;
        }

        node.compute_totals();
        Ok(())
    }

    pub fn build_tree(path: &str) -> AppResult<FileNode> {
        let abs_path = std::path::PathBuf::from(path).canonicalize().unwrap_or_else(|_| std::path::PathBuf::from(path));
        let path_str = abs_path.to_string_lossy().to_string();

        let mut root = FileNode::root(&path_str);
        root.name = format!("PC ({})", abs_path.file_name().unwrap_or(std::ffi::OsStr::new(&path_str)).to_string_lossy());
        root.expanded = true;
        
        // Add parent directory ".." if there is a parent
        if abs_path.parent().is_some() {
            let parent_path = abs_path.parent().unwrap().to_string_lossy().to_string();
            let mut parent_node = FileNode::root(&parent_path);
            parent_node.name = "..".to_string();
            parent_node.is_dir = true;
            parent_node.depth = 1;
            root.children.push(parent_node);
        }
        
        Self::load_children(&mut root)?;
        Ok(root)
    }
}
