use super::*;
use tempfile::TempDir;

fn settings_file(temp: &TempDir) -> PathBuf {
    temp.path()
        .join("config")
        .join("sql-library-directories.json")
}

fn default_root(temp: &TempDir) -> PathBuf {
    temp.path().join("data").join("sql-library")
}

#[test]
fn empty_name_is_rejected() {
    let temp = TempDir::new().unwrap();
    assert_eq!(
        sql_file_path_for_name(temp.path(), ""),
        Err(SqlFileNameError::Empty)
    );
    assert_eq!(
        sql_file_path_for_name(temp.path(), "   "),
        Err(SqlFileNameError::Empty)
    );
}

#[test]
fn name_is_validated_before_being_used() {
    let temp = TempDir::new().unwrap();
    for name in [
        "../report",
        "nested/report",
        "report:daily",
        "CON",
        "nul.sql",
    ] {
        assert_eq!(
            sql_file_path_for_name(temp.path(), name),
            Err(SqlFileNameError::Invalid),
            "{name} should be rejected"
        );
    }
}

#[test]
fn sql_extension_is_appended_when_missing() {
    let temp = TempDir::new().unwrap();
    assert_eq!(
        sql_file_path_for_name(temp.path(), "report").unwrap(),
        temp.path().join("report.sql")
    );
    assert_eq!(
        sql_file_path_for_name(temp.path(), "report.sql").unwrap(),
        temp.path().join("report.sql")
    );
    assert_eq!(
        sql_file_path_for_name(temp.path(), "REPORT.SQL").unwrap(),
        temp.path().join("REPORT.SQL")
    );
}

#[test]
fn duplicate_names_are_rejected_case_insensitively() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("report.sql"), "").unwrap();
    assert_eq!(
        sql_file_path_for_name(temp.path(), "Report"),
        Err(SqlFileNameError::AlreadyExists)
    );
}

#[test]
fn creating_and_deleting_files_and_folders() {
    let temp = TempDir::new().unwrap();

    let file = create_sql_library_file(temp.path(), "orders").unwrap();
    assert_eq!(file, temp.path().join("orders.sql"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "");
    assert!(create_sql_library_file(temp.path(), "orders").is_err());

    let folder = create_sql_library_folder(temp.path(), "reports").unwrap();
    assert!(folder.is_dir());
    assert!(create_sql_library_folder(temp.path(), "reports").is_err());
    assert!(create_sql_library_folder(temp.path(), "bad:name").is_err());

    let nested = create_sql_library_file(&folder, "monthly").unwrap();
    delete_sql_library_entry(&file).unwrap();
    assert!(!file.exists());
    delete_sql_library_entry(&folder).unwrap();
    assert!(!nested.exists());
    assert!(!folder.exists());
}

#[test]
fn renaming_keeps_sql_extension_and_avoids_collisions() {
    let temp = TempDir::new().unwrap();
    let file = create_sql_library_file(temp.path(), "report").unwrap();
    let taken = create_sql_library_file(temp.path(), "archive").unwrap();

    let renamed = rename_sql_library_entry(&file, "report_2026").unwrap();
    assert_eq!(renamed, temp.path().join("report_2026.sql"));
    assert!(!file.exists());

    assert!(rename_sql_library_entry(&renamed, "archive").is_err());
    assert!(taken.exists());

    let folder = create_sql_library_folder(temp.path(), "drafts").unwrap();
    let renamed_folder = rename_sql_library_entry(&folder, "archive_2026").unwrap();
    assert_eq!(renamed_folder, temp.path().join("archive_2026"));
    assert!(renamed_folder.is_dir());
}

#[test]
fn listing_only_returns_folders_and_sql_files_with_folders_first() {
    let temp = TempDir::new().unwrap();
    create_sql_library_folder(temp.path(), "zebra").unwrap();
    create_sql_library_folder(temp.path(), "Alpha").unwrap();
    create_sql_library_file(temp.path(), "orders").unwrap();
    fs::write(temp.path().join("notes.md"), "").unwrap();

    let entries = list_sql_library_directory(temp.path()).unwrap();
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["Alpha", "zebra", "orders.sql"]);
    assert!(entries[0].is_dir());
    assert!(!entries[2].is_dir());
    assert_eq!(entries[2].path, temp.path().join("orders.sql"));
}

#[test]
fn missing_directory_lists_as_empty() {
    let temp = TempDir::new().unwrap();
    assert!(
        list_sql_library_directory(&temp.path().join("nope"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn roots_start_with_the_default_library() {
    let temp = TempDir::new().unwrap();
    let settings = settings_file(&temp);
    let default = default_root(&temp);

    assert_eq!(
        roots_from(&settings, &default).unwrap(),
        vec![default.clone()]
    );

    let mounted = temp.path().join("mounted");
    fs::create_dir_all(&mounted).unwrap();
    add_directory_to(&settings, &default, &mounted).unwrap();

    let roots = roots_from(&settings, &default).unwrap();
    assert_eq!(
        roots,
        vec![default.clone(), mounted.canonicalize().unwrap()]
    );
}

#[test]
fn mounted_directories_are_deduplicated_and_default_is_implicit() {
    let temp = TempDir::new().unwrap();
    let settings = settings_file(&temp);
    let default = default_root(&temp);
    fs::create_dir_all(&default).unwrap();

    let mounted = temp.path().join("mounted");
    fs::create_dir_all(&mounted).unwrap();
    add_directory_to(&settings, &default, &mounted).unwrap();
    add_directory_to(&settings, &default, &mounted).unwrap();
    assert_eq!(load_settings(&settings).unwrap().directories.len(), 1);

    add_directory_to(&settings, &default, &default).unwrap();
    assert_eq!(load_settings(&settings).unwrap().directories.len(), 1);

    assert!(add_directory_to(&settings, &default, &temp.path().join("missing")).is_err());
}

#[test]
fn removing_a_mounted_directory_reports_whether_it_existed() {
    let temp = TempDir::new().unwrap();
    let settings = settings_file(&temp);
    let default = default_root(&temp);
    let mounted = temp.path().join("mounted");
    fs::create_dir_all(&mounted).unwrap();
    add_directory_to(&settings, &default, &mounted).unwrap();

    assert!(remove_directory_from(&settings, &mounted).unwrap());
    assert!(!remove_directory_from(&settings, &mounted).unwrap());
    assert!(load_settings(&settings).unwrap().directories.is_empty());
}

#[test]
fn settings_with_unknown_fields_still_load() {
    let temp = TempDir::new().unwrap();
    let settings = settings_file(&temp);
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"directories":["/tmp/a"],"future":1}"#).unwrap();

    assert_eq!(
        load_settings(&settings).unwrap().directories,
        vec![PathBuf::from("/tmp/a")]
    );
}

#[test]
fn reserved_and_control_characters_are_invalid() {
    assert!(is_invalid_sql_file_name("nul"));
    assert!(is_invalid_sql_file_name("com1.sql"));
    assert!(is_invalid_sql_file_name("a\nb"));
    assert!(is_invalid_sql_file_name("."));
    assert!(!is_invalid_sql_file_name("report"));
    assert!(!is_invalid_sql_file_name("月度报表.sql"));
}
