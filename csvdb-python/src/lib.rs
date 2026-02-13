use std::path::Path;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use csvdb::commands::{checksum, diff, init, sql, to_csv, to_duckdb, to_parquetdb, to_sqlite, validate};
use csvdb::{NullMode, OrderMode, TableFilter};

fn to_py_err(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(format!("{:#}", e))
}

fn parse_order(s: &str) -> PyResult<OrderMode> {
    match s {
        "pk" => Ok(OrderMode::Pk),
        "all-columns" => Ok(OrderMode::AllColumns),
        "add-synthetic-key" => Ok(OrderMode::AddSyntheticKey),
        _ => Err(PyRuntimeError::new_err(format!("Unknown order mode: {}", s))),
    }
}

fn parse_null_mode(s: &str) -> PyResult<NullMode> {
    match s {
        "marker" => Ok(NullMode::Marker),
        "empty" => Ok(NullMode::Empty),
        "literal" => Ok(NullMode::Literal),
        _ => Err(PyRuntimeError::new_err(format!("Unknown null mode: {}", s))),
    }
}

/// Convert any supported format to a .csvdb directory.
///
/// Args:
///     input: Path to input file (.sqlite, .duckdb) or directory
///     output: Output directory (default: <input>.csvdb)
///     order: Row ordering mode - "pk", "all-columns", or "add-synthetic-key"
///     null_mode: NULL representation - "marker", "empty", or "literal"
///     natural_sort: Sort string PKs naturally (e.g. "item2" before "item10")
///     order_by: Custom ORDER BY clause (e.g. "created_at DESC")
///     compress: Compress CSV files with gzip
///     force: Overwrite existing output directory
///     tables: Only include these tables
///     exclude: Exclude these tables
///
/// Returns:
///     Output directory path as string
#[pyfunction]
#[pyo3(name = "to_csvdb", signature = (input, *, output=None, order="pk", null_mode="marker", natural_sort=false, order_by=None, compress=false, force=false, tables=vec![], exclude=vec![]))]
fn py_to_csvdb(
    input: &str,
    output: Option<&str>,
    order: &str,
    null_mode: &str,
    natural_sort: bool,
    order_by: Option<&str>,
    compress: bool,
    force: bool,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<String> {
    let order_mode = parse_order(order)?;
    let null_m = parse_null_mode(null_mode)?;
    let filter = TableFilter::new(tables, exclude);

    let path = to_csv::to_csv(
        Path::new(input),
        order_mode,
        null_m,
        natural_sort,
        order_by,
        compress,
        output.map(Path::new),
        force,
        &filter,
    ).map_err(to_py_err)?;

    Ok(path.to_string_lossy().into_owned())
}

/// Convert any supported format to a .csvdb directory (incremental mode).
///
/// Only re-exports tables whose data has changed.
///
/// Returns:
///     Dict with "path", "unchanged", "updated", "added", "removed"
#[pyfunction]
#[pyo3(name = "to_csvdb_incremental", signature = (input, *, output=None, order="pk", null_mode="marker", natural_sort=false, order_by=None, compress=false, tables=vec![], exclude=vec![]))]
fn py_to_csvdb_incremental(
    py: Python<'_>,
    input: &str,
    output: Option<&str>,
    order: &str,
    null_mode: &str,
    natural_sort: bool,
    order_by: Option<&str>,
    compress: bool,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<PyObject> {
    let order_mode = parse_order(order)?;
    let null_m = parse_null_mode(null_mode)?;
    let filter = TableFilter::new(tables, exclude);

    let (path, summary) = to_csv::to_csv_incremental(
        Path::new(input),
        order_mode,
        null_m,
        natural_sort,
        order_by,
        compress,
        output.map(Path::new),
        &filter,
    ).map_err(to_py_err)?;

    let dict = PyDict::new(py);
    dict.set_item("path", path.to_string_lossy().into_owned())?;
    dict.set_item("unchanged", summary.unchanged)?;
    dict.set_item("updated", summary.updated)?;
    dict.set_item("added", summary.added)?;
    dict.set_item("removed", summary.removed)?;
    Ok(dict.into())
}

/// Convert any supported format to a SQLite database.
///
/// Returns:
///     Output database path as string
#[pyfunction]
#[pyo3(name = "to_sqlite", signature = (input, *, output=None, force=false, tables=vec![], exclude=vec![]))]
fn py_to_sqlite(
    input: &str,
    output: Option<&str>,
    force: bool,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<String> {
    let filter = TableFilter::new(tables, exclude);
    let path = to_sqlite::to_sqlite(Path::new(input), output.map(Path::new), force, &filter).map_err(to_py_err)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Convert any supported format to a DuckDB database.
///
/// Returns:
///     Output database path as string
#[pyfunction]
#[pyo3(name = "to_duckdb", signature = (input, *, output=None, force=false, tables=vec![], exclude=vec![]))]
fn py_to_duckdb(
    input: &str,
    output: Option<&str>,
    force: bool,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<String> {
    let filter = TableFilter::new(tables, exclude);
    let path = to_duckdb::to_duckdb(Path::new(input), output.map(Path::new), force, &filter).map_err(to_py_err)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Convert any supported format to a .parquetdb directory.
///
/// Returns:
///     Output directory path as string
#[pyfunction]
#[pyo3(name = "to_parquetdb", signature = (input, *, output=None, order="pk", null_mode="marker", order_by=None, force=false, tables=vec![], exclude=vec![]))]
fn py_to_parquetdb(
    input: &str,
    output: Option<&str>,
    order: &str,
    null_mode: &str,
    order_by: Option<&str>,
    force: bool,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<String> {
    let order_mode = parse_order(order)?;
    let null_m = parse_null_mode(null_mode)?;
    let filter = TableFilter::new(tables, exclude);

    let path = to_parquetdb::to_parquetdb(
        Path::new(input),
        order_mode,
        null_m,
        order_by,
        output.map(Path::new),
        force,
        &filter,
    ).map_err(to_py_err)?;

    Ok(path.to_string_lossy().into_owned())
}

/// Run a read-only SQL query against any supported format.
///
/// Returns:
///     List of dicts, one per row, with column names as keys.
///     NULL values are represented as Python None.
#[pyfunction]
#[pyo3(name = "sql", signature = (path, query))]
fn sql_query(
    py: Python<'_>,
    path: &str,
    query: &str,
) -> PyResult<PyObject> {
    let result = sql::sql_query(Path::new(path), query).map_err(to_py_err)?;

    let rows = PyList::empty(py);
    for (row_idx, row) in result.rows.iter().enumerate() {
        let dict = PyDict::new(py);
        for (col_idx, col_name) in result.column_names.iter().enumerate() {
            if result.null_flags[row_idx][col_idx] {
                dict.set_item(col_name, py.None())?;
            } else {
                dict.set_item(col_name, &row[col_idx])?;
            }
        }
        rows.append(dict)?;
    }

    Ok(rows.into())
}

/// Compute a checksum of a database or .csvdb directory.
///
/// Returns:
///     SHA256 hash as hex string
#[pyfunction]
#[pyo3(name = "checksum", signature = (path, *, tables=vec![], exclude=vec![]))]
fn checksum_db(
    path: &str,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<String> {
    let filter = TableFilter::new(tables, exclude);
    checksum::checksum(Path::new(path), &filter).map_err(to_py_err)
}

/// Compare two databases or .csvdb directories.
///
/// Returns:
///     True if differences found, False if identical
#[pyfunction]
#[pyo3(name = "diff", signature = (left, right, *, summary=false, tables=vec![], exclude=vec![]))]
fn diff_db(
    left: &str,
    right: &str,
    summary: bool,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<bool> {
    let filter = TableFilter::new(tables, exclude);
    diff::diff(Path::new(left), Path::new(right), summary, &filter).map_err(to_py_err)
}

/// Validate a .csvdb or .parquetdb directory.
///
/// Returns:
///     Dict with "table_count", "view_count", "errors", and "warnings"
#[pyfunction]
#[pyo3(name = "validate", signature = (path,))]
fn validate_db(
    py: Python<'_>,
    path: &str,
) -> PyResult<PyObject> {
    let result = validate::validate(Path::new(path)).map_err(to_py_err)?;

    let dict = PyDict::new(py);
    dict.set_item("table_count", result.table_count)?;
    dict.set_item("view_count", result.view_count)?;
    dict.set_item("errors", result.errors)?;
    dict.set_item("warnings", result.warnings)?;
    Ok(dict.into())
}

/// Initialize a .csvdb directory from raw CSV files (infer schema).
///
/// Returns:
///     Dict with "output_dir", "tables" (list of table info dicts), and "warnings"
#[pyfunction]
#[pyo3(name = "init", signature = (source, *, output=None, force=false, detect_pk=true, detect_fk=true, tables=vec![], exclude=vec![]))]
fn init_csvdb(
    py: Python<'_>,
    source: &str,
    output: Option<&str>,
    force: bool,
    detect_pk: bool,
    detect_fk: bool,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<PyObject> {
    let config = init::InferConfig {
        detect_pk,
        detect_fk,
        ..Default::default()
    };
    let filter = TableFilter::new(tables, exclude);

    let result = init::init_csvdb(Path::new(source), output.map(Path::new), force, &filter, &config).map_err(to_py_err)?;

    let dict = PyDict::new(py);
    dict.set_item("output_dir", result.output_dir.to_string_lossy().into_owned())?;

    let tables = PyList::empty(py);
    for t in &result.tables {
        let td = PyDict::new(py);
        td.set_item("name", &t.name)?;
        td.set_item("row_count", t.row_count)?;
        td.set_item("column_count", t.columns.len())?;
        td.set_item("suggested_pk", t.suggested_pk.as_deref())?;

        let fks = PyList::empty(py);
        for fk in &t.suggested_fks {
            let fk_dict = PyDict::new(py);
            fk_dict.set_item("column", &fk.column)?;
            fk_dict.set_item("references_table", &fk.references_table)?;
            fk_dict.set_item("references_column", &fk.references_column)?;
            fks.append(fk_dict)?;
        }
        td.set_item("suggested_fks", fks)?;

        tables.append(td)?;
    }

    dict.set_item("tables", tables)?;
    dict.set_item("warnings", &result.warnings)?;
    Ok(dict.into())
}

/// Get the csvdb library version.
#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pymodule(name = "csvdb")]
fn csvdb_python(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(py_to_csvdb, m)?)?;
    m.add_function(wrap_pyfunction!(py_to_csvdb_incremental, m)?)?;
    m.add_function(wrap_pyfunction!(py_to_sqlite, m)?)?;
    m.add_function(wrap_pyfunction!(py_to_duckdb, m)?)?;
    m.add_function(wrap_pyfunction!(py_to_parquetdb, m)?)?;
    m.add_function(wrap_pyfunction!(sql_query, m)?)?;
    m.add_function(wrap_pyfunction!(checksum_db, m)?)?;
    m.add_function(wrap_pyfunction!(diff_db, m)?)?;
    m.add_function(wrap_pyfunction!(validate_db, m)?)?;
    m.add_function(wrap_pyfunction!(init_csvdb, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
