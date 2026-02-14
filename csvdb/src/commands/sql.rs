use anyhow::{bail, Context, Result};
use comfy_table::{Cell, CellAlignment, Table as ComfyTable};
use duckdb::Connection;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;

use crate::core::csv::find_table_file;
use crate::core::input::InputFormat;
use crate::core::Schema;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Csv,
    Table,
}

/// Structured result from a SQL query.
pub struct QueryResult {
    pub column_names: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub null_flags: Vec<Vec<bool>>,
}

/// Execute a read-only SQL query and return structured results.
pub fn sql_query(path: &Path, query: &str) -> Result<QueryResult> {
    let (result, _) = sql_query_internal(path, query)?;
    Ok(result)
}

/// Internal: returns QueryResult + numeric column flags (for table formatting).
fn sql_query_internal(path: &Path, query: &str) -> Result<(QueryResult, Vec<bool>)> {
    // Validate query is read-only
    let trimmed = query.trim();
    let upper = trimmed.to_uppercase();
    let first_word = upper.split_whitespace().next().unwrap_or("");
    if first_word != "SELECT" && first_word != "WITH" {
        bail!("Only SELECT queries are supported. Got: {}", first_word);
    }

    let input_format = InputFormat::from_path(path)?;

    let conn = Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    load_data(&conn, path, input_format)?;

    // Execute query using Arrow interface for reliable column metadata
    let mut stmt = conn
        .prepare(trimmed)
        .with_context(|| format!("Failed to prepare query: {}", trimmed))?;

    let arrow = stmt
        .query_arrow([])
        .context("Failed to execute query")?;

    let schema = arrow.get_schema();
    let column_names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    let column_count = column_names.len();

    // Detect numeric columns from Arrow schema
    let numeric_cols: Vec<bool> = schema.fields().iter().map(|f| is_numeric_type(f.data_type())).collect();

    // Collect all record batches
    let batches: Vec<arrow::record_batch::RecordBatch> = arrow.collect();

    // Convert Arrow batches to Vec<Vec<String>> for output
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut null_flags: Vec<Vec<bool>> = Vec::new();

    for batch in &batches {
        for row_idx in 0..batch.num_rows() {
            let mut row_values = Vec::with_capacity(column_count);
            let mut row_nulls = Vec::with_capacity(column_count);
            for col_idx in 0..column_count {
                let col = batch.column(col_idx);
                if col.is_null(row_idx) {
                    row_values.push(String::new());
                    row_nulls.push(true);
                } else {
                    let val = arrow_value_to_string(col.as_ref(), row_idx);
                    row_values.push(val);
                    row_nulls.push(false);
                }
            }
            rows.push(row_values);
            null_flags.push(row_nulls);
        }
    }

    Ok((QueryResult { column_names, rows, null_flags }, numeric_cols))
}

/// Execute a read-only SQL query and return Arrow RecordBatches directly.
pub fn sql_query_arrow(path: &Path, query: &str) -> Result<Vec<arrow::record_batch::RecordBatch>> {
    let trimmed = query.trim();
    let upper = trimmed.to_uppercase();
    let first_word = upper.split_whitespace().next().unwrap_or("");
    if first_word != "SELECT" && first_word != "WITH" {
        bail!("Only SELECT queries are supported. Got: {}", first_word);
    }

    let input_format = InputFormat::from_path(path)?;

    let conn = Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    load_data(&conn, path, input_format)?;

    let mut stmt = conn
        .prepare(trimmed)
        .with_context(|| format!("Failed to prepare query: {}", trimmed))?;

    let arrow = stmt
        .query_arrow([])
        .context("Failed to execute query")?;

    let batches: Vec<arrow::record_batch::RecordBatch> = arrow.collect();
    Ok(batches)
}

pub fn sql(path: &Path, query: &str, format: Option<OutputFormat>) -> Result<()> {
    let (result, numeric_cols) = sql_query_internal(path, query)?;

    // Determine output format
    let format = format.unwrap_or_else(|| {
        if io::stdout().is_terminal() {
            OutputFormat::Table
        } else {
            OutputFormat::Csv
        }
    });

    match format {
        OutputFormat::Csv => print_csv(&result.column_names, &result.rows)?,
        OutputFormat::Table => print_table(&result.column_names, &result.rows, &result.null_flags, &numeric_cols)?,
    }

    Ok(())
}

pub(crate) fn load_data(conn: &Connection, path: &Path, format: InputFormat) -> Result<()> {
    match format {
        InputFormat::Sqlite => load_sqlite(conn, path),
        InputFormat::DuckDb => load_duckdb(conn, path),
        InputFormat::Csvdb => load_csvdb(conn, path),
        InputFormat::Parquetdb => load_parquetdb(conn, path),
        InputFormat::Parquet => load_single_parquet(conn, path),
    }
}

fn load_sqlite(conn: &Connection, path: &Path) -> Result<()> {
    let abs_path = path.canonicalize()?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");
    let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

    conn.execute(
        &format!("ATTACH '{}' AS src (TYPE SQLITE, READ_ONLY)", path_str),
        [],
    )?;
    conn.execute("USE src", [])?;
    Ok(())
}

fn load_duckdb(conn: &Connection, path: &Path) -> Result<()> {
    let abs_path = path.canonicalize()?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");
    let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

    conn.execute(
        &format!("ATTACH '{}' AS src (READ_ONLY)", path_str),
        [],
    )?;
    conn.execute("USE src", [])?;
    Ok(())
}

fn load_csvdb(conn: &Connection, path: &Path) -> Result<()> {
    let schema_path = path.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    let schema_sql = fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() && stmt.to_uppercase().starts_with("CREATE") {
            let stmt = stmt.replace(" REAL", " DOUBLE");
            conn.execute(&stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    for table_name in schema.tables_in_fk_order()? {
        if let Some(csv_path) = find_table_file(path, table_name) {
            let abs_path = csv_path.canonicalize()?;
            let path_str = abs_path.to_string_lossy().replace('\\', "/");
            let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

            conn.execute(
                &format!(
                    "COPY \"{}\" FROM '{}' (HEADER, NULL '\\N')",
                    table_name, path_str
                ),
                [],
            )?;
        }
    }

    Ok(())
}

fn load_parquetdb(conn: &Connection, path: &Path) -> Result<()> {
    let schema_path = path.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    let schema_sql = fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() && stmt.to_uppercase().starts_with("CREATE") {
            let stmt = stmt.replace(" REAL", " DOUBLE");
            conn.execute(&stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    for table_name in schema.tables_in_fk_order()? {
        let parquet_path = path.join(format!("{}.parquet", table_name));
        if parquet_path.exists() {
            let abs_path = parquet_path.canonicalize()?;
            let path_str = abs_path.to_string_lossy().replace('\\', "/");
            let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

            conn.execute(
                &format!(
                    "INSERT INTO \"{}\" SELECT * FROM read_parquet('{}')",
                    table_name, path_str
                ),
                [],
            )?;
        }
    }

    Ok(())
}

fn load_single_parquet(conn: &Connection, path: &Path) -> Result<()> {
    let table_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("table");

    let abs_path = path.canonicalize()?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");
    let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

    conn.execute(
        &format!(
            "CREATE TABLE \"{}\" AS SELECT * FROM read_parquet('{}')",
            table_name, path_str
        ),
        [],
    )?;

    Ok(())
}

pub(crate) fn arrow_value_to_string(col: &dyn arrow::array::Array, row: usize) -> String {
    use arrow::array::*;
    use arrow::datatypes::DataType;

    match col.data_type() {
        DataType::Boolean => {
            let arr = col.as_any().downcast_ref::<BooleanArray>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Int8 => {
            let arr = col.as_any().downcast_ref::<Int8Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Int16 => {
            let arr = col.as_any().downcast_ref::<Int16Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Int32 => {
            let arr = col.as_any().downcast_ref::<Int32Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Int64 => {
            let arr = col.as_any().downcast_ref::<Int64Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::UInt8 => {
            let arr = col.as_any().downcast_ref::<UInt8Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::UInt16 => {
            let arr = col.as_any().downcast_ref::<UInt16Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::UInt32 => {
            let arr = col.as_any().downcast_ref::<UInt32Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::UInt64 => {
            let arr = col.as_any().downcast_ref::<UInt64Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Float32 => {
            let arr = col.as_any().downcast_ref::<Float32Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Float64 => {
            let arr = col.as_any().downcast_ref::<Float64Array>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Utf8 => {
            let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
            arr.value(row).to_string()
        }
        DataType::LargeUtf8 => {
            let arr = col.as_any().downcast_ref::<LargeStringArray>().unwrap();
            arr.value(row).to_string()
        }
        DataType::Binary => {
            let arr = col.as_any().downcast_ref::<BinaryArray>().unwrap();
            arr.value(row).iter().map(|b| format!("{:02x}", b)).collect()
        }
        DataType::LargeBinary => {
            let arr = col.as_any().downcast_ref::<LargeBinaryArray>().unwrap();
            arr.value(row).iter().map(|b| format!("{:02x}", b)).collect()
        }
        _ => {
            // Fallback: use Arrow's display formatting
            arrow::util::display::array_value_to_string(col, row).unwrap_or_default()
        }
    }
}

fn is_numeric_type(dt: &arrow::datatypes::DataType) -> bool {
    use arrow::datatypes::DataType;
    matches!(
        dt,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    )
}

fn print_csv(column_names: &[String], rows: &[Vec<String>]) -> Result<()> {
    let stdout = io::stdout();
    let mut wtr = csv::Writer::from_writer(stdout.lock());

    wtr.write_record(column_names)?;

    for row in rows {
        wtr.write_record(row)?;
    }

    wtr.flush()?;
    Ok(())
}

fn print_table(
    column_names: &[String],
    rows: &[Vec<String>],
    null_flags: &[Vec<bool>],
    numeric_cols: &[bool],
) -> Result<()> {
    let mut table = ComfyTable::new();
    table.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED);
    table.set_header(column_names);

    for (row_idx, row) in rows.iter().enumerate() {
        let cells: Vec<Cell> = row
            .iter()
            .enumerate()
            .map(|(col_idx, val)| {
                let is_null = null_flags[row_idx][col_idx];
                let display = if is_null {
                    "NULL".to_string()
                } else {
                    val.clone()
                };
                let mut cell = Cell::new(display);
                if col_idx < numeric_cols.len() && numeric_cols[col_idx] && !is_null {
                    cell = cell.set_alignment(CellAlignment::Right);
                }
                cell
            })
            .collect();
        table.add_row(cells);
    }

    println!("{table}");
    eprintln!("({} rows)", rows.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection as SqliteConnection;
    use tempfile::tempdir;

    #[test]
    fn test_query_sqlite() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                [],
            )?;
            conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])?;
            conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])?;
        }

        // Should succeed without error
        sql(&db_path, "SELECT count(*) FROM users", Some(OutputFormat::Csv))?;
        Ok(())
    }

    #[test]
    fn test_rejects_non_select() {
        let result = sql(
            Path::new("nonexistent.sqlite"),
            "DROP TABLE users",
            Some(OutputFormat::Csv),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_allows_with_cte() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE nums (n INTEGER PRIMARY KEY)",
                [],
            )?;
            conn.execute("INSERT INTO nums VALUES (1)", [])?;
        }

        sql(
            &db_path,
            "WITH cte AS (SELECT n FROM nums) SELECT * FROM cte",
            Some(OutputFormat::Csv),
        )?;
        Ok(())
    }

    #[test]
    fn test_sql_query_arrow() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score INTEGER)",
                [],
            )?;
            conn.execute("INSERT INTO users VALUES (1, 'Alice', 95)", [])?;
            conn.execute("INSERT INTO users VALUES (2, 'Bob', 87)", [])?;
            conn.execute("INSERT INTO users VALUES (3, 'Charlie', 92)", [])?;
        }

        let batches = sql_query_arrow(&db_path, "SELECT id, name, score FROM users ORDER BY id")?;

        // Should have at least one batch
        assert!(!batches.is_empty());

        // Check total row count
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);

        // Check schema
        let schema = batches[0].schema();
        let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(field_names, vec!["id", "name", "score"]);

        Ok(())
    }

    #[test]
    fn test_sql_query_arrow_rejects_non_select() {
        let result = sql_query_arrow(
            Path::new("nonexistent.sqlite"),
            "DROP TABLE users",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Only SELECT"));
    }
}
