//! SQL 库面板的纯状态：根目录 + 已展开目录的子项，展平成可渲染行。
//!
//! 这一层不碰 GPUI，只负责「哪些行可见」，方便直接单测。

use one_core::storage::{SqlLibraryEntry, list_sql_library_directory};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 一行可见的树节点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlLibraryRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
    /// 是否由用户挂载（可以取消挂载，默认根目录不行）。
    pub is_mounted: bool,
}

/// 面板展示的库树。
#[derive(Default)]
pub struct SqlLibraryTree {
    roots: Vec<PathBuf>,
    mounted: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<SqlLibraryEntry>>,
    expanded: HashSet<PathBuf>,
}

impl SqlLibraryTree {
    pub fn set_roots(&mut self, roots: Vec<PathBuf>, mounted: HashSet<PathBuf>) {
        self.roots = roots;
        self.mounted = mounted;
        self.children
            .retain(|directory, _| self.roots.iter().any(|root| root == directory));
        self.expanded
            .retain(|directory| self.roots.iter().any(|root| root == directory));
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn set_children(&mut self, directory: &Path, entries: Vec<SqlLibraryEntry>) {
        self.children.insert(directory.to_path_buf(), entries);
    }

    pub fn is_expanded(&self, directory: &Path) -> bool {
        self.expanded.contains(directory)
    }

    /// 展开 / 收起一个目录，返回展开后是否需要加载它的子项。
    pub fn toggle(&mut self, directory: &Path) -> bool {
        if self.expanded.remove(directory) {
            return false;
        }
        self.expanded.insert(directory.to_path_buf());
        !self.children.contains_key(directory)
    }

    /// 确保目录处于展开状态；子项由调用方随后加载。
    pub fn expand(&mut self, directory: &Path) {
        self.expanded.insert(directory.to_path_buf());
    }

    /// 根目录 + 已展开目录：刷新时需要重新读取它们的子项。
    pub fn visible_directories(&self) -> Vec<PathBuf> {
        let mut directories: Vec<PathBuf> = self.roots.clone();
        for directory in &self.expanded {
            if !directories.contains(directory) {
                directories.push(directory.clone());
            }
        }
        directories
    }

    /// 展平成可见行：根目录始终可见，子项仅在父目录展开时可见。
    pub fn rows(&self) -> Vec<SqlLibraryRow> {
        let mut rows = Vec::new();
        for root in &self.roots {
            rows.push(SqlLibraryRow {
                path: root.clone(),
                name: root_name(root),
                is_dir: true,
                depth: 0,
                expanded: self.is_expanded(root),
                is_mounted: self.mounted.contains(root),
            });
            self.push_children(root, 1, &mut rows);
        }
        rows
    }

    fn push_children(&self, directory: &Path, depth: usize, rows: &mut Vec<SqlLibraryRow>) {
        if !self.is_expanded(directory) {
            return;
        }
        let Some(entries) = self.children.get(directory) else {
            return;
        };
        for entry in entries {
            rows.push(SqlLibraryRow {
                path: entry.path.clone(),
                name: entry.name.clone(),
                is_dir: entry.is_dir(),
                depth,
                expanded: entry.is_dir() && self.is_expanded(&entry.path),
                is_mounted: false,
            });
            if entry.is_dir() {
                self.push_children(&entry.path, depth + 1, rows);
            }
        }
    }
}

/// 读取目录内容，供后台任务调用。
pub fn read_directory(directory: &Path) -> Result<Vec<SqlLibraryEntry>, String> {
    list_sql_library_directory(directory).map_err(|error| error.to_string())
}

/// 库根目录行显示目录名，完整路径留给 tooltip。
fn root_name(root: &Path) -> String {
    root.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}
