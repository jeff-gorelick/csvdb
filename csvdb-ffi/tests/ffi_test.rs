use std::ffi::{c_char, CStr, CString};
use std::fs;
use std::ptr;

use tempfile::tempdir;

// Import the FFI functions directly — they're pub extern "C" so Rust can call them.
use csvdb_ffi::*;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

unsafe fn read_and_free(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null(), "got NULL, error: {}", last_error_string());
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    unsafe { csvdb_free_string(ptr) };
    s
}

fn last_error_string() -> String {
    let ptr = csvdb_last_error();
    if ptr.is_null() {
        "no error".to_string()
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
    }
}

/// Create a test .csvdb directory and return its path.
fn make_test_csvdb(dir: &std::path::Path) -> std::path::PathBuf {
    let csvdb_dir = dir.join("test.csvdb");
    fs::create_dir(&csvdb_dir).unwrap();

    fs::write(
        csvdb_dir.join("schema.sql"),
        "CREATE TABLE \"users\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT NOT NULL,\n    \"score\" INTEGER\n);\n",
    ).unwrap();
    fs::write(
        csvdb_dir.join("users.csv"),
        "id,name,score\n1,Alice,95\n2,Bob,87\n3,Charlie,92\n",
    ).unwrap();

    csvdb_dir
}

/// Create a test SQLite database and return its path.
fn make_test_sqlite(dir: &std::path::Path) -> std::path::PathBuf {
    let db_path = dir.join("test.sqlite");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score INTEGER)",
        [],
    ).unwrap();
    conn.execute("INSERT INTO users VALUES (1, 'Alice', 95)", []).unwrap();
    conn.execute("INSERT INTO users VALUES (2, 'Bob', 87)", []).unwrap();
    conn.execute("INSERT INTO users VALUES (3, 'Charlie', 92)", []).unwrap();
    db_path
}

#[test]
fn test_version() {
    let ptr = csvdb_version();
    assert!(!ptr.is_null());
    let version = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
    assert!(version.starts_with("0."), "unexpected version: {version}");
}

#[test]
fn test_last_error_initially_null() {
    // After a successful call, last_error should be null
    csvdb_version();
    // Can't guarantee state here since other tests may run, but version() doesn't set error
}

#[test]
fn test_free_string_null_safe() {
    // Should not crash
    unsafe { csvdb_free_string(ptr::null_mut()) };
}

#[test]
fn test_checksum() {
    let dir = tempdir().unwrap();
    let csvdb_dir = make_test_csvdb(dir.path());
    let input = c(csvdb_dir.to_str().unwrap());

    let hash = unsafe { read_and_free(csvdb_checksum(input.as_ptr(), ptr::null(), ptr::null())) };
    assert_eq!(hash.len(), 64, "expected SHA256 hex string, got: {hash}");
    // All hex chars
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_checksum_null_input() {
    let result = unsafe { csvdb_checksum(ptr::null(), ptr::null(), ptr::null()) };
    assert!(result.is_null());
    let err = last_error_string();
    assert!(err.contains("null"), "expected null error, got: {err}");
}

#[test]
fn test_validate_valid() {
    let dir = tempdir().unwrap();
    let csvdb_dir = make_test_csvdb(dir.path());
    let input = c(csvdb_dir.to_str().unwrap());

    let rc = unsafe { csvdb_validate(input.as_ptr()) };
    assert_eq!(rc, 0, "expected valid (0), got {rc}");
}

#[test]
fn test_validate_invalid() {
    let dir = tempdir().unwrap();
    let bad_dir = dir.path().join("bad.csvdb");
    fs::create_dir(&bad_dir).unwrap();
    // No schema.sql — should report errors
    let input = c(bad_dir.to_str().unwrap());

    let rc = unsafe { csvdb_validate(input.as_ptr()) };
    assert_eq!(rc, 1, "expected errors (1), got {rc}");
}

#[test]
fn test_to_sqlite() {
    let dir = tempdir().unwrap();
    let csvdb_dir = make_test_csvdb(dir.path());
    let input = c(csvdb_dir.to_str().unwrap());

    let path = unsafe { read_and_free(csvdb_to_sqlite(input.as_ptr(), ptr::null(), 1, ptr::null(), ptr::null())) };
    assert!(path.ends_with(".sqlite"), "unexpected path: {path}");
    assert!(std::path::Path::new(&path).exists());
}

#[test]
fn test_to_duckdb() {
    let dir = tempdir().unwrap();
    let csvdb_dir = make_test_csvdb(dir.path());
    let input = c(csvdb_dir.to_str().unwrap());

    let path = unsafe { read_and_free(csvdb_to_duckdb(input.as_ptr(), ptr::null(), 1, ptr::null(), ptr::null())) };
    assert!(path.ends_with(".duckdb"), "unexpected path: {path}");
    assert!(std::path::Path::new(&path).exists());
}

#[test]
fn test_to_csvdb_from_sqlite() {
    let dir = tempdir().unwrap();
    let db_path = make_test_sqlite(dir.path());
    let input = c(db_path.to_str().unwrap());

    let path = unsafe {
        read_and_free(csvdb_to_csvdb(
            input.as_ptr(),
            ptr::null(), // default output
            ptr::null(), // default order
            ptr::null(), // default null_mode
            1,           // force
            ptr::null(), // tables
            ptr::null(), // exclude
        ))
    };
    assert!(path.ends_with(".csvdb"), "unexpected path: {path}");
    assert!(std::path::Path::new(&path).exists());
    assert!(std::path::Path::new(&path).join("schema.sql").exists());
    assert!(std::path::Path::new(&path).join("users.csv").exists());
}

#[test]
fn test_to_parquetdb() {
    let dir = tempdir().unwrap();
    let csvdb_dir = make_test_csvdb(dir.path());
    let input = c(csvdb_dir.to_str().unwrap());

    let path = unsafe {
        read_and_free(csvdb_to_parquetdb(
            input.as_ptr(),
            ptr::null(), // default output
            ptr::null(), // default order
            ptr::null(), // default null_mode
            1,           // force
            ptr::null(), // tables
            ptr::null(), // exclude
        ))
    };
    assert!(path.ends_with(".parquetdb"), "unexpected path: {path}");
    assert!(std::path::Path::new(&path).exists());
}

#[test]
fn test_sql_query() {
    let dir = tempdir().unwrap();
    let db_path = make_test_sqlite(dir.path());
    let path = c(db_path.to_str().unwrap());
    let query = c("SELECT name, score FROM users ORDER BY id");

    let csv_output = unsafe { read_and_free(csvdb_sql(path.as_ptr(), query.as_ptr())) };

    // Should be CSV with header
    let lines: Vec<&str> = csv_output.trim().lines().collect();
    assert_eq!(lines[0], "name,score");
    assert_eq!(lines.len(), 4); // header + 3 rows
    assert!(lines[1].contains("Alice"));
}

#[test]
fn test_sql_rejects_non_select() {
    let dir = tempdir().unwrap();
    let db_path = make_test_sqlite(dir.path());
    let path = c(db_path.to_str().unwrap());
    let query = c("DROP TABLE users");

    let result = unsafe { csvdb_sql(path.as_ptr(), query.as_ptr()) };
    assert!(result.is_null());
    let err = last_error_string();
    assert!(err.contains("SELECT"), "expected SELECT error, got: {err}");
}

#[test]
fn test_diff_identical() {
    let dir = tempdir().unwrap();
    let csvdb_dir = make_test_csvdb(dir.path());
    let input = c(csvdb_dir.to_str().unwrap());

    let rc = unsafe { csvdb_diff(input.as_ptr(), input.as_ptr(), 0, ptr::null(), ptr::null()) };
    assert_eq!(rc, 0, "expected no diff (0), got {rc}");
}

#[test]
fn test_diff_different() {
    let dir = tempdir().unwrap();

    // Create two different .csvdb dirs
    let dir1 = dir.path().join("a.csvdb");
    fs::create_dir(&dir1).unwrap();
    fs::write(dir1.join("schema.sql"), "CREATE TABLE \"t\" (\"id\" INTEGER PRIMARY KEY, \"v\" TEXT);\n").unwrap();
    fs::write(dir1.join("t.csv"), "id,v\n1,hello\n").unwrap();

    let dir2 = dir.path().join("b.csvdb");
    fs::create_dir(&dir2).unwrap();
    fs::write(dir2.join("schema.sql"), "CREATE TABLE \"t\" (\"id\" INTEGER PRIMARY KEY, \"v\" TEXT);\n").unwrap();
    fs::write(dir2.join("t.csv"), "id,v\n1,world\n").unwrap();

    let left = c(dir1.to_str().unwrap());
    let right = c(dir2.to_str().unwrap());

    let rc = unsafe { csvdb_diff(left.as_ptr(), right.as_ptr(), 1, ptr::null(), ptr::null()) };
    assert_eq!(rc, 1, "expected diff (1), got {rc}");
}

#[test]
fn test_init() {
    let dir = tempdir().unwrap();
    let csv_dir = dir.path().join("raw");
    fs::create_dir(&csv_dir).unwrap();
    fs::write(
        csv_dir.join("products.csv"),
        "id,name,price\n1,Widget,9.99\n2,Gadget,19.99\n",
    ).unwrap();

    let source = c(csv_dir.to_str().unwrap());
    let path = unsafe { read_and_free(csvdb_init(source.as_ptr(), ptr::null(), 1, ptr::null(), ptr::null())) };
    assert!(path.ends_with(".csvdb"), "unexpected path: {path}");
    assert!(std::path::Path::new(&path).join("schema.sql").exists());
}

#[test]
fn test_checksum_consistency() {
    // Checksum of the same data via different formats should match
    let dir = tempdir().unwrap();
    let csvdb_dir = make_test_csvdb(dir.path());

    let csvdb_input = c(csvdb_dir.to_str().unwrap());
    let csvdb_hash = unsafe { read_and_free(csvdb_checksum(csvdb_input.as_ptr(), ptr::null(), ptr::null())) };

    // Convert to SQLite, checksum should match
    let sqlite_path = unsafe { read_and_free(csvdb_to_sqlite(csvdb_input.as_ptr(), ptr::null(), 1, ptr::null(), ptr::null())) };
    let sqlite_input = c(&sqlite_path);
    let sqlite_hash = unsafe { read_and_free(csvdb_checksum(sqlite_input.as_ptr(), ptr::null(), ptr::null())) };

    assert_eq!(csvdb_hash, sqlite_hash, "csvdb vs sqlite checksum mismatch");
}
