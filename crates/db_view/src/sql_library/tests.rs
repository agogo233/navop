use super::tree::{SqlLibraryTree, read_directory};
use one_core::storage::{QueryDirectoryEntryKind, SqlLibraryEntry};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn file(directory: &Path, name: &str) -> SqlLibraryEntry {
    SqlLibraryEntry {
        name: name.to_string(),
        path: directory.join(name),
        kind: QueryDirectoryEntryKind::SqlFile,
    }
}

fn folder(directory: &Path, name: &str) -> SqlLibraryEntry {
    SqlLibraryEntry {
        name: name.to_string(),
        path: directory.join(name),
        kind: QueryDirectoryEntryKind::Directory,
    }
}

fn tree_with_root(root: &Path) -> SqlLibraryTree {
    let mut tree = SqlLibraryTree::default();
    tree.set_roots(vec![root.to_path_buf()], HashSet::new());
    tree
}

#[test]
fn rows_only_contain_expanded_children() {
    let root = PathBuf::from("/library");
    let mut tree = tree_with_root(&root);
    let reports = folder(&root, "reports");
    tree.set_children(&root, vec![reports.clone(), file(&root, "orders.sql")]);

    let collapsed = tree.rows();
    assert_eq!(collapsed.len(), 1);
    assert_eq!(collapsed[0].name, "library");
    assert!(collapsed[0].is_dir);
    assert!(!collapsed[0].expanded);
    assert_eq!(collapsed[0].depth, 0);

    tree.expand(&root);
    let expanded = tree.rows();
    let names: Vec<&str> = expanded.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, vec!["library", "reports", "orders.sql"]);
    assert_eq!(expanded[1].depth, 1);
    assert_eq!(expanded[2].depth, 1);
    assert!(!expanded[1].expanded);
}

#[test]
fn nested_rows_follow_expansion_depth() {
    let root = PathBuf::from("/library");
    let mut tree = tree_with_root(&root);
    let reports = folder(&root, "reports");
    tree.set_children(&root, vec![reports.clone()]);
    tree.set_children(&reports.path, vec![file(&reports.path, "monthly.sql")]);
    tree.expand(&root);
    tree.expand(&reports.path);

    let rows = tree.rows();
    let names: Vec<(&str, usize)> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.depth))
        .collect();
    assert_eq!(
        names,
        vec![("library", 0), ("reports", 1), ("monthly.sql", 2)]
    );
    assert!(rows[1].expanded);
}

#[test]
fn toggling_reports_when_children_still_need_loading() {
    let root = PathBuf::from("/library");
    let mut tree = tree_with_root(&root);

    assert!(tree.toggle(&root), "首次展开需要加载子项");
    assert!(tree.is_expanded(&root));
    assert!(!tree.toggle(&root), "收起不需要加载");
    assert!(!tree.is_expanded(&root));

    tree.set_children(&root, vec![file(&root, "orders.sql")]);
    assert!(!tree.toggle(&root), "已有缓存时不需要重新加载");
}

#[test]
fn visible_directories_cover_roots_and_expanded_folders() {
    let root = PathBuf::from("/library");
    let mut tree = tree_with_root(&root);
    let reports = folder(&root, "reports");
    tree.set_children(&root, vec![reports.clone()]);
    tree.expand(&root);
    tree.expand(&reports.path);
    tree.expand(&root);

    let directories = tree.visible_directories();
    assert_eq!(directories, vec![root, reports.path]);
}

#[test]
fn resetting_roots_drops_stale_state() {
    let first = PathBuf::from("/first");
    let second = PathBuf::from("/second");
    let mut tree = tree_with_root(&first);
    tree.set_children(&first, vec![file(&first, "orders.sql")]);
    tree.expand(&first);

    let mut mounted = HashSet::new();
    mounted.insert(second.clone());
    tree.set_roots(vec![second.clone()], mounted);

    assert!(!tree.is_expanded(&first));
    assert_eq!(tree.visible_directories(), vec![second.clone()]);
    let rows = tree.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, second);
    assert!(rows[0].is_mounted);
}

#[test]
fn mounted_flag_is_only_set_for_mounted_roots() {
    let default = PathBuf::from("/default");
    let mounted = PathBuf::from("/mounted");
    let mut tree = SqlLibraryTree::default();
    tree.set_roots(
        vec![default.clone(), mounted.clone()],
        HashSet::from([mounted.clone()]),
    );

    let rows = tree.rows();
    assert!(!rows[0].is_mounted);
    assert!(rows[1].is_mounted);
}

#[test]
fn read_directory_treats_a_missing_directory_as_empty() {
    let missing = std::env::temp_dir().join("navop-sql-library-missing-dir");
    assert!(!missing.exists());
    assert!(read_directory(&missing).unwrap().is_empty());
}

/// 接线契约：点击库内文件后要能落到 SQL 编辑器。
#[test]
fn database_tab_opens_library_files_in_the_sql_editor() {
    let source = include_str!("../database_tab.rs");
    assert!(source.contains("SqlLibraryEvent::OpenSqlFile"));
    assert!(source.contains("open_sql_library_file"));
    assert!(source.contains("SqlEditorTab::new_with_config"));
}

/// 接线契约：数据库页侧边栏必须保留 SQL 库入口。
#[test]
fn database_sidebar_exposes_the_sql_library_panel() {
    let source = include_str!("../sidebar/mod.rs");
    assert!(source.contains("SidebarPanel::SqlLibrary"));
    assert!(source.contains("IconName::FolderQueries"));
    assert!(source.contains("SqlLibraryPanel"));
}

/// 接线契约：文件点击事件由面板自身发出。
#[test]
fn panel_emits_open_sql_file_for_clicked_files() {
    let source = include_str!("panel.rs");
    assert!(source.contains("SqlLibraryEvent::OpenSqlFile"));
}

/// i18n 契约：代码里用到的 `SqlLibrary.*` 键都必须在语言包里存在。
#[test]
fn every_sql_library_locale_key_exists() {
    let locale = include_str!("../../locales/db_view.yml");
    let mut checked = 0;
    for source in [include_str!("panel.rs"), include_str!("view.rs")] {
        for key in sql_library_locale_keys(source) {
            assert!(
                locale.contains(&format!("\n  {key}:\n")),
                "语言包缺少键 {key}"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "没有扫描到任何 SqlLibrary.* 键");
}

/// 抽取源码中 `t!("SqlLibrary.<key>"` 形式的键名。
fn sql_library_locale_keys(source: &str) -> Vec<String> {
    const PREFIX: &str = "t!(\"SqlLibrary.";
    let mut keys = Vec::new();
    let mut rest = source;
    while let Some(index) = rest.find(PREFIX) {
        rest = &rest[index + PREFIX.len()..];
        match rest.find('"') {
            Some(end) => {
                keys.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    keys
}
