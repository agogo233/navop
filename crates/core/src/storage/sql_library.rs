//! SQL 库：跨连接的 SQL 文档库根目录与文件操作。
//!
//! 与 [`crate::storage::query_directory`]（按「连接 + 数据库」隔离的查询目录）不同，
//! SQL 库是全局文档库：默认根目录位于应用数据目录下，用户还可以挂载任意已有 SQL
//! 目录。库内 `.sql` 文件用 SQL 编辑器打开，可自由切换连接执行。

use crate::app_dirs;
use crate::storage::manager::get_config_dir;
use crate::storage::query_directory::{
    QueryDirectoryEntryKind, create_query_subdirectory, is_sql_file,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const SQL_LIBRARY_DIRECTORIES_FILE: &str = "sql-library-directories.json";
const DEFAULT_SQL_LIBRARY_DIR_NAME: &str = "sql-library";
const SQL_FILE_EXTENSION: &str = "sql";

/// SQL 库条目：子目录或 `.sql` 文件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlLibraryEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: QueryDirectoryEntryKind,
}

impl SqlLibraryEntry {
    pub fn is_dir(&self) -> bool {
        self.kind == QueryDirectoryEntryKind::Directory
    }
}

/// SQL 文件 / 目录名称不可用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SqlFileNameError {
    Empty,
    Invalid,
    AlreadyExists,
    ReadDirectory(String),
}

impl std::fmt::Display for SqlFileNameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("name cannot be empty"),
            Self::Invalid => formatter.write_str("name is not a valid file name"),
            Self::AlreadyExists => formatter.write_str("a file with this name already exists"),
            Self::ReadDirectory(message) => {
                write!(formatter, "failed to read directory: {message}")
            }
        }
    }
}

impl std::error::Error for SqlFileNameError {}

/// 默认 SQL 库根目录：应用数据目录下的 `sql-library`。
pub fn default_sql_library_root() -> Result<PathBuf> {
    let base = app_dirs::data_dir().context("application data directory is unavailable")?;
    Ok(base.join(DEFAULT_SQL_LIBRARY_DIR_NAME))
}

/// 库根目录列表：默认根目录在前，其后是用户挂载的目录（排序、去重）。
pub fn sql_library_roots() -> Result<Vec<PathBuf>> {
    let default_root = default_sql_library_root()?;
    ensure_directory(&default_root)?;
    roots_from(&settings_path()?, &default_root)
}

pub fn is_default_sql_library_root(directory: &Path) -> bool {
    default_sql_library_root().is_ok_and(|root| same_directory(&root, directory))
}

pub fn mounted_sql_library_directories() -> Result<Vec<PathBuf>> {
    Ok(load_settings(&settings_path()?)?.directories)
}

pub fn add_sql_library_directory(directory: &Path) -> Result<PathBuf> {
    add_directory_to(&settings_path()?, &default_sql_library_root()?, directory)
}

pub fn remove_sql_library_directory(directory: &Path) -> Result<bool> {
    remove_directory_from(&settings_path()?, directory)
}

/// 列出一个库目录下的子目录与 `.sql` 文件，目录排在前。
pub fn list_sql_library_directory(directory: &Path) -> Result<Vec<SqlLibraryEntry>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read SQL library {}", directory.display()))?;
    let mut items = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry of {}", directory.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", path.display()))?;

        let (name, kind) = if file_type.is_dir() {
            (
                entry.file_name().to_string_lossy().into_owned(),
                QueryDirectoryEntryKind::Directory,
            )
        } else if file_type.is_file() && is_sql_file(&path) {
            (
                entry.file_name().to_string_lossy().into_owned(),
                QueryDirectoryEntryKind::SqlFile,
            )
        } else {
            continue;
        };

        items.push(SqlLibraryEntry { name, path, kind });
    }

    items.sort_by(|left, right| {
        kind_rank(left.kind)
            .cmp(&kind_rank(right.kind))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(items)
}

pub fn create_sql_library_file(directory: &Path, name: &str) -> Result<PathBuf> {
    let path = sql_file_path_for_name(directory, name)?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("failed to create SQL file {}", path.display()))?;
    Ok(path)
}

pub fn create_sql_library_folder(directory: &Path, name: &str) -> Result<PathBuf> {
    if is_invalid_sql_file_name(name.trim()) {
        bail!("folder name is not a valid directory name");
    }
    create_query_subdirectory(directory, name)
}

/// 重命名库内条目；文件名会保留 `.sql` 后缀。
pub fn rename_sql_library_entry(path: &Path, new_name: &str) -> Result<PathBuf> {
    let is_dir = path.is_dir();
    let target = if is_dir {
        directory_name_for(path, new_name)?
    } else {
        sql_file_path_for_name_ignoring_self(path, new_name)?
    };

    if target.exists() {
        bail!("{} already exists", target.display());
    }
    fs::rename(path, &target).with_context(|| {
        format!(
            "failed to rename {} to {}",
            path.display(),
            target.display()
        )
    })?;
    Ok(target)
}

pub fn delete_sql_library_entry(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to delete folder {}", path.display()))
    } else {
        fs::remove_file(path).with_context(|| format!("failed to delete file {}", path.display()))
    }
}

/// 解析目录下新建 SQL 文件的绝对路径，必要时补 `.sql` 后缀。
pub fn sql_file_path_for_name(directory: &Path, name: &str) -> Result<PathBuf, SqlFileNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SqlFileNameError::Empty);
    }
    if is_invalid_sql_file_name(name) {
        return Err(SqlFileNameError::Invalid);
    }

    let file_name = with_sql_extension(name);
    if file_name.eq_ignore_ascii_case(".sql") {
        return Err(SqlFileNameError::Invalid);
    }
    if directory_contains(directory, &file_name) {
        return Err(SqlFileNameError::AlreadyExists);
    }

    Ok(directory.join(file_name))
}

pub fn is_invalid_sql_file_name(name: &str) -> bool {
    const INVALID_CHARACTERS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    const RESERVED_NAMES: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    if matches!(name, "." | "..")
        || name.chars().any(char::is_control)
        || name
            .chars()
            .any(|character| INVALID_CHARACTERS.contains(&character))
    {
        return true;
    }
    let base_name = name.split('.').next().unwrap_or(name);
    RESERVED_NAMES
        .iter()
        .any(|reserved| base_name.eq_ignore_ascii_case(reserved))
}

fn kind_rank(kind: QueryDirectoryEntryKind) -> u8 {
    match kind {
        QueryDirectoryEntryKind::Directory => 0,
        QueryDirectoryEntryKind::SqlFile => 1,
    }
}

fn with_sql_extension(name: &str) -> String {
    let has_sql_extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(SQL_FILE_EXTENSION));
    if has_sql_extension {
        name.to_owned()
    } else {
        format!("{name}.{SQL_FILE_EXTENSION}")
    }
}

fn directory_contains(directory: &Path, file_name: &str) -> bool {
    match fs::read_dir(directory) {
        Ok(entries) => entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(file_name)
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

fn directory_name_for(path: &Path, new_name: &str) -> Result<PathBuf> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        bail!("folder name cannot be empty");
    }
    if is_invalid_sql_file_name(new_name) {
        bail!("folder name is not a valid directory name");
    }
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    Ok(parent.join(new_name))
}

fn sql_file_path_for_name_ignoring_self(
    path: &Path,
    new_name: &str,
) -> Result<PathBuf, SqlFileNameError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = new_name.trim();
    if name.is_empty() {
        return Err(SqlFileNameError::Empty);
    }
    if is_invalid_sql_file_name(name) {
        return Err(SqlFileNameError::Invalid);
    }
    let file_name = with_sql_extension(name);
    if file_name.eq_ignore_ascii_case(".sql") {
        return Err(SqlFileNameError::Invalid);
    }
    let target = parent.join(&file_name);
    if target != path && directory_contains(parent, &file_name) {
        return Err(SqlFileNameError::AlreadyExists);
    }
    Ok(target)
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn ensure_directory(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create SQL library {}", directory.display()))
}

fn roots_from(settings: &Path, default_root: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = vec![default_root.to_path_buf()];
    for directory in load_settings(settings)?.directories {
        if !roots.iter().any(|root| same_directory(root, &directory)) {
            roots.push(directory);
        }
    }
    Ok(roots)
}

fn add_directory_to(settings: &Path, default_root: &Path, directory: &Path) -> Result<PathBuf> {
    if !directory.is_dir() {
        bail!(
            "SQL library directory does not exist: {}",
            directory.display()
        );
    }

    let directory = directory
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", directory.display()))?;
    if same_directory(default_root, &directory) {
        return Ok(directory);
    }

    let mut stored = load_settings(settings)?;
    if !stored
        .directories
        .iter()
        .any(|existing| same_directory(existing, &directory))
    {
        stored.directories.push(directory.clone());
        stored.directories.sort_by(|left, right| {
            display_name(left)
                .to_lowercase()
                .cmp(&display_name(right).to_lowercase())
                .then_with(|| left.cmp(right))
        });
        save_settings_to(&stored, settings)?;
    }
    Ok(directory)
}

fn remove_directory_from(settings: &Path, directory: &Path) -> Result<bool> {
    let mut stored = load_settings(settings)?;
    let before = stored.directories.len();
    stored
        .directories
        .retain(|existing| !same_directory(existing, directory));
    if stored.directories.len() == before {
        return Ok(false);
    }
    save_settings_to(&stored, settings)?;
    Ok(true)
}

fn display_name(directory: &Path) -> String {
    directory
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| directory.display().to_string())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct SqlLibraryDirectories {
    #[serde(default)]
    directories: Vec<PathBuf>,
}

fn load_settings(path: &Path) -> Result<SqlLibraryDirectories> {
    if !path.exists() {
        return Ok(SqlLibraryDirectories::default());
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read SQL library settings {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse SQL library settings {}", path.display()))
}

fn save_settings_to(settings: &SqlLibraryDirectories, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(&settings)
        .context("failed to serialize SQL library settings")?;
    fs::write(path, content)
        .with_context(|| format!("failed to save SQL library settings {}", path.display()))
}

fn settings_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join(SQL_LIBRARY_DIRECTORIES_FILE))
}

#[cfg(test)]
#[path = "sql_library_tests.rs"]
mod tests;
