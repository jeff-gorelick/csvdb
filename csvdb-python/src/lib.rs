#![allow(clippy::too_many_arguments)]

use std::path::Path;

use arrow::pyarrow::{FromPyArrow, ToPyArrow};
use arrow::record_batch::RecordBatch;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};

use csvdb::commands::{checksum, diff, init, read, sql, to_csv, to_duckdb, to_parquetdb, to_sqlite, validate, write};
use csvdb::{NullMode, OrderMode, TableFilter};

fn to_py_err(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(format!("{e:#}"))
}

fn parse_order(s: &str) -> PyResult<OrderMode> {
    match s {
        "pk" => Ok(OrderMode::Pk),
        "all-columns" => Ok(OrderMode::AllColumns),
        "add-synthetic-key" => Ok(OrderMode::AddSyntheticKey),
        _ => Err(PyRuntimeError::new_err(format!("Unknown order mode: {s}"))),
    }
}

fn parse_null_mode(s: &str) -> PyResult<NullMode> {
    match s {
        "marker" => Ok(NullMode::Marker),
        "empty" => Ok(NullMode::Empty),
        "literal" => Ok(NullMode::Literal),
        _ => Err(PyRuntimeError::new_err(format!("Unknown null mode: {s}"))),
    }
}

/// Normalize a Python DataFrame (pandas, polars, or pyarrow) to a pyarrow.Table.
fn normalize_to_arrow_table<'py>(
    py: Python<'py>,
    obj: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    // pyarrow.Table: has to_batches() method
    if obj.hasattr("to_batches")? {
        return Ok(obj.clone());
    }
    // polars.DataFrame: has to_arrow() method
    if obj.hasattr("to_arrow")? {
        return obj.call_method0("to_arrow");
    }
    // pandas.DataFrame: use pa.Table.from_pandas()
    let pa = PyModule::import(py, "pyarrow")
        .map_err(|_| PyRuntimeError::new_err("pyarrow is required for DataFrame conversion. Install with: pip install csvdb-py[arrow]"))?;
    pa.getattr("Table")?.call_method1("from_pandas", (obj,))
        .map_err(|e| PyRuntimeError::new_err(format!(
            "Failed to convert to Arrow table: {e}. Expected pandas.DataFrame, polars.DataFrame, or pyarrow.Table"
        )))
}

/// Convert a Python dict of DataFrames to Vec<(String, Vec<RecordBatch>)>.
fn dict_to_arrow_tables(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
) -> PyResult<Vec<(String, Vec<RecordBatch>)>> {
    let mut tables = Vec::new();
    for (key, value) in dict {
        let name: String = key.extract()?;
        let arrow_table = normalize_to_arrow_table(py, &value)?;
        let py_batches = arrow_table.call_method0("to_batches")?;
        let batches_list = py_batches.downcast::<PyList>()?;
        let mut batches = Vec::new();
        for py_batch in batches_list.iter() {
            batches.push(RecordBatch::from_pyarrow_bound(&py_batch)?);
        }
        tables.push((name, batches));
    }
    Ok(tables)
}

/// Convert any supported format to a .csvdb directory, or write DataFrames to csvdb.
///
/// When input is a string path: converts .sqlite, .duckdb, .parquetdb, or .parquet to .csvdb.
/// When input is a dict of DataFrames: writes them as a new .csvdb directory.
///
/// Args:
///     input: Path to input file (str) or dict of DataFrames (pandas, polars, or pyarrow)
///     output: Output directory (required when input is a dict)
///     order: Row ordering mode - "pk", "all-columns", or "add-synthetic-key" (path mode only)
///     null_mode: NULL representation - "marker", "empty", or "literal" (path mode only)
///     natural_sort: Sort string PKs naturally (path mode only)
///     order_by: Custom ORDER BY clause (path mode only)
///     compress: Compress CSV files with gzip (path mode only)
///     force: Overwrite existing output directory
///     tables: Only include these tables (path mode only)
///     exclude: Exclude these tables (path mode only)
///
/// Returns:
///     Output directory path as string
#[pyfunction]
#[pyo3(name = "to_csvdb", signature = (input, *, output=None, order="pk", null_mode="marker", natural_sort=false, order_by=None, compress=false, force=false, tables=vec![], exclude=vec![]))]
fn py_to_csvdb(
    py: Python<'_>,
    input: &Bound<'_, PyAny>,
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
    // Dict of DataFrames → write to csvdb
    if input.is_instance_of::<PyDict>() {
        let output = output.ok_or_else(|| {
            PyRuntimeError::new_err("output is required when writing from DataFrames")
        })?;
        let dict = input.downcast::<PyDict>()?;
        let arrow_tables = dict_to_arrow_tables(py, dict)?;
        let result = write::write_csvdb_from_arrow(arrow_tables, Path::new(output), force)
            .map_err(to_py_err)?;
        return Ok(result.to_string_lossy().into_owned());
    }

    // String path → existing conversion
    let input_str: String = input.extract()
        .map_err(|_| PyRuntimeError::new_err("input must be a file path (str) or dict of DataFrames"))?;

    let order_mode = parse_order(order)?;
    let null_m = parse_null_mode(null_mode)?;
    let filter = TableFilter::new(tables, exclude);

    let path = to_csv::to_csv(
        Path::new(&input_str),
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

/// Convert Vec<RecordBatch> to a pyarrow.Table.
fn batches_to_py_table(py: Python<'_>, batches: Vec<RecordBatch>) -> PyResult<PyObject> {
    let pa = PyModule::import(py, "pyarrow")?;

    if batches.is_empty() {
        return Ok(pa.call_method1("table", (PyDict::new(py),))?.into());
    }

    let py_batches = PyList::empty(py);
    for batch in &batches {
        py_batches.append(batch.to_pyarrow(py)?)?;
    }

    Ok(pa.getattr("Table")?.call_method1("from_batches", (py_batches,))?.into())
}

/// Read tables as pyarrow Tables.
///
/// If `table` is specified, returns a single pyarrow.Table.
/// If `table` is None, returns a dict mapping table names to pyarrow.Tables.
///
/// Requires pyarrow to be installed.
#[pyfunction]
#[pyo3(name = "to_arrow", signature = (path, table=None, *, tables=vec![], exclude=vec![]))]
fn py_to_arrow(
    py: Python<'_>,
    path: &str,
    table: Option<&str>,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<PyObject> {
    match table {
        Some(table_name) => {
            let batches = read::read_table_arrow(Path::new(path), table_name).map_err(to_py_err)?;
            batches_to_py_table(py, batches)
        }
        None => {
            let filter = TableFilter::new(tables, exclude);
            let all_tables = read::read_tables_arrow(Path::new(path), &filter).map_err(to_py_err)?;
            let dict = PyDict::new(py);
            for (name, batches) in all_tables {
                let py_table = batches_to_py_table(py, batches)?;
                dict.set_item(name, py_table)?;
            }
            Ok(dict.into())
        }
    }
}

/// Run a read-only SQL query and return results as a pyarrow.Table.
///
/// Requires pyarrow to be installed.
#[pyfunction]
#[pyo3(name = "sql_arrow", signature = (path, query))]
fn py_sql_arrow(
    py: Python<'_>,
    path: &str,
    query: &str,
) -> PyResult<PyObject> {
    let batches = sql::sql_query_arrow(Path::new(path), query).map_err(to_py_err)?;
    batches_to_py_table(py, batches)
}

/// Convert Arrow result (single pa.Table or dict of pa.Tables) to pandas DataFrames.
fn arrow_to_pandas(py: Python<'_>, arrow_result: PyObject) -> PyResult<PyObject> {
    let bound = arrow_result.bind(py);
    if bound.is_instance_of::<PyDict>() {
        let dict = bound.downcast::<PyDict>()?;
        let result = PyDict::new(py);
        for (key, value) in dict {
            result.set_item(key, value.call_method0("to_pandas")?)?;
        }
        Ok(result.into())
    } else {
        Ok(bound.call_method0("to_pandas")?.into())
    }
}

/// Convert Arrow result (single pa.Table or dict of pa.Tables) to polars DataFrames.
fn arrow_to_polars(py: Python<'_>, arrow_result: PyObject) -> PyResult<PyObject> {
    let pl = PyModule::import(py, "polars")?;
    let bound = arrow_result.bind(py);
    if bound.is_instance_of::<PyDict>() {
        let dict = bound.downcast::<PyDict>()?;
        let result = PyDict::new(py);
        for (key, value) in dict {
            result.set_item(key, pl.call_method1("from_arrow", (value,))?)?;
        }
        Ok(result.into())
    } else {
        Ok(pl.call_method1("from_arrow", (bound,))?.into())
    }
}

/// Read tables as pandas DataFrames.
///
/// If `table` is specified, returns a single pd.DataFrame.
/// If `table` is None, returns a dict mapping table names to pd.DataFrames.
///
/// Requires pandas and pyarrow to be installed.
#[pyfunction]
#[pyo3(name = "to_pandas", signature = (path, table=None, *, tables=vec![], exclude=vec![]))]
fn py_to_pandas(
    py: Python<'_>,
    path: &str,
    table: Option<&str>,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<PyObject> {
    PyModule::import(py, "pandas")
        .map_err(|_| PyRuntimeError::new_err("pandas is required. Install with: pip install csvdb-py[pandas]"))?;
    let arrow = py_to_arrow(py, path, table, tables, exclude)?;
    arrow_to_pandas(py, arrow)
}

/// Read tables as polars DataFrames.
///
/// If `table` is specified, returns a single pl.DataFrame.
/// If `table` is None, returns a dict mapping table names to pl.DataFrames.
///
/// Requires polars and pyarrow to be installed.
#[pyfunction]
#[pyo3(name = "to_polars", signature = (path, table=None, *, tables=vec![], exclude=vec![]))]
fn py_to_polars(
    py: Python<'_>,
    path: &str,
    table: Option<&str>,
    tables: Vec<String>,
    exclude: Vec<String>,
) -> PyResult<PyObject> {
    PyModule::import(py, "polars")
        .map_err(|_| PyRuntimeError::new_err("polars is required. Install with: pip install csvdb-py[polars]"))?;
    let arrow = py_to_arrow(py, path, table, tables, exclude)?;
    arrow_to_polars(py, arrow)
}

/// Run a read-only SQL query and return results as a pandas DataFrame.
///
/// Requires pandas and pyarrow to be installed.
#[pyfunction]
#[pyo3(name = "sql_pandas", signature = (path, query))]
fn py_sql_pandas(
    py: Python<'_>,
    path: &str,
    query: &str,
) -> PyResult<PyObject> {
    PyModule::import(py, "pandas")
        .map_err(|_| PyRuntimeError::new_err("pandas is required. Install with: pip install csvdb-py[pandas]"))?;
    let arrow = py_sql_arrow(py, path, query)?;
    arrow_to_pandas(py, arrow)
}

/// Run a read-only SQL query and return results as a polars DataFrame.
///
/// Requires polars and pyarrow to be installed.
#[pyfunction]
#[pyo3(name = "sql_polars", signature = (path, query))]
fn py_sql_polars(
    py: Python<'_>,
    path: &str,
    query: &str,
) -> PyResult<PyObject> {
    PyModule::import(py, "polars")
        .map_err(|_| PyRuntimeError::new_err("polars is required. Install with: pip install csvdb-py[polars]"))?;
    let arrow = py_sql_arrow(py, path, query)?;
    arrow_to_polars(py, arrow)
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
    m.add_function(wrap_pyfunction!(py_to_arrow, m)?)?;
    m.add_function(wrap_pyfunction!(py_sql_arrow, m)?)?;
    m.add_function(wrap_pyfunction!(py_to_pandas, m)?)?;
    m.add_function(wrap_pyfunction!(py_to_polars, m)?)?;
    m.add_function(wrap_pyfunction!(py_sql_pandas, m)?)?;
    m.add_function(wrap_pyfunction!(py_sql_polars, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
