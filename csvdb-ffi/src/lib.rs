use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;
use std::ptr;

use csvdb::commands::{
    checksum, diff, init, sql, to_csv, to_duckdb, to_parquetdb, to_sqlite, validate,
};
use csvdb::{NullMode, OrderMode, TableFilter};

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_error(msg: String) {
    let c = CString::new(msg).unwrap_or_else(|_| CString::new("unknown error").unwrap());
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(c));
}

fn clear_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = None);
}

/// # Safety
///
/// `s` must be a valid, NUL-terminated C string or null.
unsafe fn to_str<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(s) }.to_str().ok()
}

fn to_cstring(s: &str) -> *mut c_char {
    CString::new(s)
        .map(|c| c.into_raw())
        .unwrap_or(ptr::null_mut())
}

fn parse_order(s: Option<&str>) -> OrderMode {
    match s {
        Some("pk") | None => OrderMode::Pk,
        Some("all-columns") => OrderMode::AllColumns,
        Some("add-synthetic-key") => OrderMode::AddSyntheticKey,
        _ => OrderMode::Pk,
    }
}

fn parse_filter(tables: Option<&str>, exclude: Option<&str>) -> TableFilter {
    let tables = tables
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let exclude = exclude
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    TableFilter::new(tables, exclude)
}

fn parse_null_mode(s: Option<&str>) -> NullMode {
    match s {
        Some("marker") | None => NullMode::Marker,
        Some("empty") => NullMode::Empty,
        Some("literal") => NullMode::Literal,
        _ => NullMode::Marker,
    }
}

/// Convert any supported format to a .csvdb directory.
///
/// Returns the output path on success (caller must free with `csvdb_free_string`),
/// or NULL on error (call `csvdb_last_error` for details).
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_to_csvdb(
    input: *const c_char,
    output: *const c_char,
    order: *const c_char,
    null_mode: *const c_char,
    force: c_int,
    tables: *const c_char,
    exclude: *const c_char,
    natural_sort: c_int,
    order_by: *const c_char,
    compress: c_int,
) -> *mut c_char {
    clear_error();
    let input = match unsafe { to_str(input) } {
        Some(s) => s,
        None => {
            set_error("input is null".into());
            return ptr::null_mut();
        }
    };
    let output = unsafe { to_str(output) };
    let order_mode = parse_order(unsafe { to_str(order) });
    let null_m = parse_null_mode(unsafe { to_str(null_mode) });
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });
    let order_by_str = unsafe { to_str(order_by) };

    match to_csv::to_csv(
        Path::new(input),
        order_mode,
        null_m,
        natural_sort != 0,
        order_by_str,
        compress != 0,
        output.map(Path::new),
        force != 0,
        &filter,
    ) {
        Ok(path) => to_cstring(&path.to_string_lossy()),
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Convert any supported format to a .csvdb directory (incremental mode).
///
/// Only re-exports tables whose data has changed. Returns a JSON string with
/// `path`, `unchanged`, `updated`, `added`, and `removed` fields.
/// Returns NULL on error.
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_to_csvdb_incremental(
    input: *const c_char,
    output: *const c_char,
    order: *const c_char,
    null_mode: *const c_char,
    tables: *const c_char,
    exclude: *const c_char,
    natural_sort: c_int,
    order_by: *const c_char,
    compress: c_int,
) -> *mut c_char {
    clear_error();
    let input = match unsafe { to_str(input) } {
        Some(s) => s,
        None => {
            set_error("input is null".into());
            return ptr::null_mut();
        }
    };
    let output = unsafe { to_str(output) };
    let order_mode = parse_order(unsafe { to_str(order) });
    let null_m = parse_null_mode(unsafe { to_str(null_mode) });
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });
    let order_by_str = unsafe { to_str(order_by) };

    match to_csv::to_csv_incremental(
        Path::new(input),
        order_mode,
        null_m,
        natural_sort != 0,
        order_by_str,
        compress != 0,
        output.map(Path::new),
        &filter,
    ) {
        Ok((path, summary)) => {
            let json = format!(
                r#"{{"path":"{}","unchanged":[{}],"updated":[{}],"added":[{}],"removed":[{}]}}"#,
                path.to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\""),
                summary
                    .unchanged
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(","),
                summary
                    .updated
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(","),
                summary
                    .added
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(","),
                summary
                    .removed
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            to_cstring(&json)
        }
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Convert any supported format to a SQLite database.
///
/// Returns the output path on success, or NULL on error.
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_to_sqlite(
    input: *const c_char,
    output: *const c_char,
    force: c_int,
    tables: *const c_char,
    exclude: *const c_char,
) -> *mut c_char {
    clear_error();
    let input = match unsafe { to_str(input) } {
        Some(s) => s,
        None => {
            set_error("input is null".into());
            return ptr::null_mut();
        }
    };
    let output = unsafe { to_str(output) };
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });

    match to_sqlite::to_sqlite(Path::new(input), output.map(Path::new), force != 0, &filter) {
        Ok(path) => to_cstring(&path.to_string_lossy()),
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Convert any supported format to a DuckDB database.
///
/// Returns the output path on success, or NULL on error.
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_to_duckdb(
    input: *const c_char,
    output: *const c_char,
    force: c_int,
    tables: *const c_char,
    exclude: *const c_char,
) -> *mut c_char {
    clear_error();
    let input = match unsafe { to_str(input) } {
        Some(s) => s,
        None => {
            set_error("input is null".into());
            return ptr::null_mut();
        }
    };
    let output = unsafe { to_str(output) };
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });

    match to_duckdb::to_duckdb(Path::new(input), output.map(Path::new), force != 0, &filter) {
        Ok(path) => to_cstring(&path.to_string_lossy()),
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Convert any supported format to a .parquetdb directory.
///
/// Returns the output path on success, or NULL on error.
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_to_parquetdb(
    input: *const c_char,
    output: *const c_char,
    order: *const c_char,
    null_mode: *const c_char,
    force: c_int,
    tables: *const c_char,
    exclude: *const c_char,
    order_by: *const c_char,
) -> *mut c_char {
    clear_error();
    let input = match unsafe { to_str(input) } {
        Some(s) => s,
        None => {
            set_error("input is null".into());
            return ptr::null_mut();
        }
    };
    let output = unsafe { to_str(output) };
    let order_mode = parse_order(unsafe { to_str(order) });
    let null_m = parse_null_mode(unsafe { to_str(null_mode) });
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });
    let order_by_str = unsafe { to_str(order_by) };

    match to_parquetdb::to_parquetdb(
        Path::new(input),
        order_mode,
        null_m,
        order_by_str,
        output.map(Path::new),
        force != 0,
        &filter,
    ) {
        Ok(path) => to_cstring(&path.to_string_lossy()),
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Compute a checksum of a database or .csvdb directory.
///
/// Returns the hash string on success, or NULL on error.
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_checksum(
    input: *const c_char,
    tables: *const c_char,
    exclude: *const c_char,
) -> *mut c_char {
    clear_error();
    let input = match unsafe { to_str(input) } {
        Some(s) => s,
        None => {
            set_error("input is null".into());
            return ptr::null_mut();
        }
    };
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });

    match checksum::checksum(Path::new(input), &filter) {
        Ok(hash) => to_cstring(&hash),
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Compare two databases or .csvdb directories.
///
/// Returns 1 if differences found, 0 if identical, -1 on error.
/// `tables` and `exclude` are optional comma-separated table names (pass NULL for none).
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
#[no_mangle]
pub unsafe extern "C" fn csvdb_diff(
    left: *const c_char,
    right: *const c_char,
    summary: c_int,
    tables: *const c_char,
    exclude: *const c_char,
) -> c_int {
    clear_error();
    let left = match unsafe { to_str(left) } {
        Some(s) => s,
        None => {
            set_error("left is null".into());
            return -1;
        }
    };
    let right = match unsafe { to_str(right) } {
        Some(s) => s,
        None => {
            set_error("right is null".into());
            return -1;
        }
    };
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });

    match diff::diff(Path::new(left), Path::new(right), summary != 0, &filter) {
        Ok(has_diff) => {
            if has_diff {
                1
            } else {
                0
            }
        }
        Err(e) => {
            set_error(format!("{e:#}"));
            -1
        }
    }
}

/// Compare two databases or .csvdb directories and return structured JSON.
///
/// Returns a JSON string on success (caller must free with `csvdb_free_string`),
/// or NULL on error (call `csvdb_last_error` for details).
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_diff_json(
    left: *const c_char,
    right: *const c_char,
    summary: c_int,
    tables: *const c_char,
    exclude: *const c_char,
) -> *mut c_char {
    clear_error();
    let left = match unsafe { to_str(left) } {
        Some(s) => s,
        None => {
            set_error("left is null".into());
            return ptr::null_mut();
        }
    };
    let right = match unsafe { to_str(right) } {
        Some(s) => s,
        None => {
            set_error("right is null".into());
            return ptr::null_mut();
        }
    };
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });

    match diff::diff_json(Path::new(left), Path::new(right), summary != 0, &filter) {
        Ok(json) => to_cstring(&json),
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Validate a .csvdb or .parquetdb directory.
///
/// Returns 0 if valid, 1 if errors found, -1 on error.
///
/// # Safety
///
/// `input` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn csvdb_validate(input: *const c_char) -> c_int {
    clear_error();
    let input = match unsafe { to_str(input) } {
        Some(s) => s,
        None => {
            set_error("input is null".into());
            return -1;
        }
    };

    match validate::validate(Path::new(input)) {
        Ok(result) => {
            if result.errors.is_empty() {
                0
            } else {
                1
            }
        }
        Err(e) => {
            set_error(format!("{e:#}"));
            -1
        }
    }
}

/// Run a read-only SQL query against any supported format.
///
/// Returns the result as CSV text on success, or NULL on error.
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_sql(path: *const c_char, query: *const c_char) -> *mut c_char {
    clear_error();
    let path = match unsafe { to_str(path) } {
        Some(s) => s,
        None => {
            set_error("path is null".into());
            return ptr::null_mut();
        }
    };
    let query = match unsafe { to_str(query) } {
        Some(s) => s,
        None => {
            set_error("query is null".into());
            return ptr::null_mut();
        }
    };

    match sql::sql_query(Path::new(path), query) {
        Ok(result) => {
            let mut wtr = csv::Writer::from_writer(Vec::new());
            if wtr.write_record(&result.column_names).is_err() {
                set_error("failed to write CSV header".into());
                return ptr::null_mut();
            }
            for row in &result.rows {
                if wtr.write_record(row).is_err() {
                    set_error("failed to write CSV row".into());
                    return ptr::null_mut();
                }
            }
            match wtr.into_inner() {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(s) => to_cstring(&s),
                    Err(_) => {
                        set_error("invalid UTF-8 in CSV output".into());
                        ptr::null_mut()
                    }
                },
                Err(_) => {
                    set_error("failed to flush CSV writer".into());
                    ptr::null_mut()
                }
            }
        }
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Initialize a .csvdb directory from raw CSV files.
///
/// Returns the output directory path on success, or NULL on error.
/// `tables` and `exclude` are optional comma-separated table names (pass NULL for none).
///
/// # Safety
///
/// All pointer parameters must be valid NUL-terminated C strings or null.
/// Caller must free the returned string with `csvdb_free_string`.
#[no_mangle]
pub unsafe extern "C" fn csvdb_init(
    source: *const c_char,
    output: *const c_char,
    force: c_int,
    tables: *const c_char,
    exclude: *const c_char,
    detect_pk: c_int,
    detect_fk: c_int,
) -> *mut c_char {
    clear_error();
    let source = match unsafe { to_str(source) } {
        Some(s) => s,
        None => {
            set_error("source is null".into());
            return ptr::null_mut();
        }
    };
    let output = unsafe { to_str(output) };
    let filter = parse_filter(unsafe { to_str(tables) }, unsafe { to_str(exclude) });

    let config = init::InferConfig {
        detect_pk: detect_pk != 0,
        detect_fk: detect_fk != 0,
        ..Default::default()
    };
    match init::init_csvdb(
        Path::new(source),
        output.map(Path::new),
        force != 0,
        &filter,
        &config,
    ) {
        Ok(result) => to_cstring(&result.output_dir.to_string_lossy()),
        Err(e) => {
            set_error(format!("{e:#}"));
            ptr::null_mut()
        }
    }
}

/// Get the csvdb library version.
///
/// Returns a static string — do NOT free it.
#[no_mangle]
pub extern "C" fn csvdb_version() -> *const c_char {
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr() as *const c_char
}

/// Get the last error message.
///
/// Returns a pointer to a static string — do NOT free it.
/// Valid until the next FFI call.
/// Returns NULL if no error.
#[no_mangle]
pub extern "C" fn csvdb_last_error() -> *const c_char {
    LAST_ERROR.with(|e| match e.borrow().as_ref() {
        Some(c) => c.as_ptr(),
        None => ptr::null(),
    })
}

/// Free a string returned by csvdb FFI functions.
///
/// It is safe to pass NULL.
///
/// # Safety
///
/// `s` must be a pointer returned by a csvdb FFI function, or null.
#[no_mangle]
pub unsafe extern "C" fn csvdb_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
