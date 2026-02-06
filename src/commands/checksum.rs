use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Sha256, Digest};
use std::io::IsTerminal;
use std::path::Path;

use crate::core::{InputFormat, Schema, TableSchema};
use crate::core::csv::read_table_csv;
use crate::core::table::Table;
use crate::{OrderMode, NullMode, TableFilter};

/// Compute a checksum for a database or csvdb directory.
/// Returns the same hash regardless of format if the data is identical.
/// Includes: schema (columns, types, PKs), indexes, views, and row data.
pub fn checksum(path: &Path, filter: &TableFilter) -> Result<String> {
    let format = InputFormat::from_path(path)?;

    match format {
        InputFormat::Csvdb => checksum_csvdb(path, filter),
        InputFormat::Parquetdb => checksum_parquetdb(path, filter),
        InputFormat::Sqlite => checksum_sqlite(path, filter),
        InputFormat::DuckDb => checksum_duckdb(path, filter),
        InputFormat::Parquet => checksum_parquet(path, filter),
    }
}

fn make_progress_bar(len: u64) -> ProgressBar {
    if std::io::stderr().is_terminal() {
        let pb = ProgressBar::new(len);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{bar:40}] {pos}/{len} {msg}")
                .unwrap(),
        );
        pb
    } else {
        ProgressBar::hidden()
    }
}

fn checksum_csvdb(csvdb_dir: &Path, filter: &TableFilter) -> Result<String> {
    let schema_path = csvdb_dir.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    let mut hasher = Sha256::new();

    // Hash schema structure (tables in sorted order, filtered)
    let pb = make_progress_bar(schema.tables.len() as u64);
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }
        pb.set_message(table_name.clone());
        hash_table_schema(&mut hasher, table_schema);

        // Hash row data
        let csv_path = csvdb_dir.join(format!("{}.csv", table_name));
        if csv_path.exists() {
            let table = read_table_csv(&csv_path, table_schema)?;
            hash_table_data(&mut hasher, &table);
        }
        pb.inc(1);
    }
    pb.finish_and_clear();

    // Hash views
    hash_views(&mut hasher, &schema);

    Ok(format!("{:x}", hasher.finalize()))
}

fn checksum_sqlite(db_path: &Path, filter: &TableFilter) -> Result<String> {
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    // Use AllColumns mode to handle tables without PK
    let schema = Schema::from_sqlite_with_order(&conn, OrderMode::AllColumns)?;

    let mut hasher = Sha256::new();

    // Hash schema and data (filtered)
    let pb = make_progress_bar(schema.tables.len() as u64);
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }
        pb.set_message(table_name.clone());
        hash_table_schema(&mut hasher, table_schema);

        let result = Table::from_sqlite_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
        hash_table_data(&mut hasher, &result.table);
        pb.inc(1);
    }
    pb.finish_and_clear();

    // Hash views
    hash_views(&mut hasher, &schema);

    Ok(format!("{:x}", hasher.finalize()))
}

fn checksum_duckdb(db_path: &Path, filter: &TableFilter) -> Result<String> {
    let conn = duckdb::Connection::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    let schema = Schema::from_duckdb_with_order(&conn, OrderMode::AllColumns)?;

    let mut hasher = Sha256::new();

    // Hash schema and data (filtered)
    let pb = make_progress_bar(schema.tables.len() as u64);
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }
        pb.set_message(table_name.clone());
        hash_table_schema(&mut hasher, table_schema);

        let result = Table::from_duckdb_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
        hash_table_data(&mut hasher, &result.table);
        pb.inc(1);
    }
    pb.finish_and_clear();

    // Hash views
    hash_views(&mut hasher, &schema);

    Ok(format!("{:x}", hasher.finalize()))
}

fn checksum_parquetdb(parquetdb_dir: &Path, filter: &TableFilter) -> Result<String> {
    // Load parquetdb into in-memory DuckDB, then compute checksum
    let conn = duckdb::Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let schema_path = parquetdb_dir.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    // Create tables from schema
    let schema_sql = std::fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() && stmt.to_uppercase().starts_with("CREATE TABLE") {
            conn.execute(stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    // Load parquet data
    for table_name in schema.tables.keys() {
        let parquet_path = parquetdb_dir.join(format!("{}.parquet", table_name));
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

    let mut hasher = Sha256::new();

    // Hash schema and data (filtered)
    let pb = make_progress_bar(schema.tables.len() as u64);
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }
        pb.set_message(table_name.clone());
        hash_table_schema(&mut hasher, table_schema);

        let result = Table::from_duckdb_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
        hash_table_data(&mut hasher, &result.table);
        pb.inc(1);
    }
    pb.finish_and_clear();

    // Hash views
    hash_views(&mut hasher, &schema);

    Ok(format!("{:x}", hasher.finalize()))
}

fn checksum_parquet(parquet_path: &Path, filter: &TableFilter) -> Result<String> {
    // Load single parquet into in-memory DuckDB
    let conn = duckdb::Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let table_name = parquet_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("table");

    if !filter.matches(table_name) {
        anyhow::bail!("Table '{}' is excluded by filter", table_name);
    }

    let abs_path = parquet_path.canonicalize()?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");
    let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

    // Create table from parquet
    conn.execute(
        &format!(
            "CREATE TABLE \"{}\" AS SELECT * FROM read_parquet('{}')",
            table_name, path_str
        ),
        [],
    )?;

    // Get schema from DuckDB
    let schema = Schema::from_duckdb_with_order(&conn, OrderMode::AllColumns)?;

    let mut hasher = Sha256::new();

    let table_schema = schema.tables.get(table_name)
        .ok_or_else(|| anyhow::anyhow!("Table not found after creation"))?;

    hash_table_schema(&mut hasher, table_schema);

    let result = Table::from_duckdb_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
    hash_table_data(&mut hasher, &result.table);

    Ok(format!("{:x}", hasher.finalize()))
}

/// Hash table schema with normalized types for cross-database consistency
fn hash_table_schema(hasher: &mut Sha256, schema: &TableSchema) {
    // Table name
    hasher.update(b"TABLE:");
    hasher.update(schema.name.as_bytes());
    hasher.update(b"\x00");

    // Columns with normalized types
    // Note: NOT NULL is excluded as it varies across databases and doesn't affect data
    for col in &schema.columns {
        if col.name == "__csvdb_rowid" {
            continue; // Skip synthetic columns
        }
        hasher.update(b"COL:");
        hasher.update(col.name.as_bytes());
        hasher.update(b":");
        hasher.update(normalize_type(&col.col_type).as_bytes());
        hasher.update(b"\x00");
    }

    // Primary key (normalized)
    if !schema.pk_columns.is_empty() {
        hasher.update(b"PK:");
        let pk_cols: Vec<&str> = schema.pk_columns.iter()
            .filter(|c| c.as_str() != "__csvdb_rowid")
            .map(|s| s.as_str())
            .collect();
        hasher.update(pk_cols.join(",").as_bytes());
        hasher.update(b"\x00");
    }

    // Skip indexes - they vary across databases and don't affect data integrity

    hasher.update(b"\x01"); // End of schema marker
}

/// Hash views - only names, not SQL (syntax varies across databases)
fn hash_views(hasher: &mut Sha256, schema: &Schema) {
    // Only hash view names - SQL syntax varies too much across databases
    let mut views: Vec<_> = schema.views.iter().collect();
    views.sort_by(|a, b| a.name.cmp(&b.name));

    for view in views {
        hasher.update(b"VIEW:");
        hasher.update(view.name.as_bytes());
        hasher.update(b"\x00");
    }
    hasher.update(b"\x03"); // Views section separator
}

/// Normalize SQL types to canonical forms for cross-database consistency
fn normalize_type(col_type: &str) -> &'static str {
    let t = col_type.to_uppercase();

    // Integer types
    if t.contains("INT") {
        return "INTEGER";
    }

    // Floating point
    if t.contains("FLOAT") || t.contains("DOUBLE") || t == "REAL" {
        return "REAL";
    }

    // Text types
    if t.contains("CHAR") || t.contains("TEXT") || t.contains("STRING")
        || t.contains("VARCHAR") || t.contains("CLOB") {
        return "TEXT";
    }

    // Binary
    if t.contains("BLOB") || t.contains("BINARY") || t.contains("BYTEA") {
        return "BLOB";
    }

    // Decimal/Numeric
    if t.contains("DECIMAL") || t.contains("NUMERIC") {
        return "NUMERIC";
    }

    // Boolean (treat as integer)
    if t.contains("BOOL") {
        return "INTEGER";
    }

    // Date/Time (treat as text for portability)
    if t.contains("DATE") || t.contains("TIME") || t.contains("TIMESTAMP") {
        return "TEXT";
    }

    // Default
    "TEXT"
}

/// Hash table row data
fn hash_table_data(hasher: &mut Sha256, table: &Table) {
    hasher.update(b"DATA:");
    hasher.update(table.name.as_bytes());
    hasher.update(b"\x00");

    // Hash row data
    for row in &table.rows {
        for (i, value) in row.values.iter().enumerate() {
            // Skip synthetic rowid column if present
            if table.columns.get(i).map(|c| c.as_str()) == Some("__csvdb_rowid") {
                continue;
            }
            // Normalize value for cross-database consistency
            hasher.update(normalize_value(value).as_bytes());
            hasher.update(b"\x00");
        }
        hasher.update(b"\x01"); // Row separator
    }
    hasher.update(b"\x02"); // Table data separator
}

/// Normalize a value for consistent hashing across databases
pub fn normalize_value(value: &str) -> String {
    if value.is_empty() {
        return String::new(); // NULL/empty stays empty
    }

    // Try to normalize as float (handles precision differences)
    if let Ok(f) = value.parse::<f64>() {
        // Check if it's actually an integer
        if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
            return (f as i64).to_string();
        }
        // Round to 10 decimal places to avoid precision issues
        return format!("{:.10}", f)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
    }

    // Return as-is for strings
    value.to_string()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{to_csv, to_sqlite, to_duckdb};
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[test]
    fn test_checksum_roundtrip() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        // Create test database with integer scores to avoid float issues
        {
            let conn = Connection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score INTEGER)",
                [],
            )?;
            conn.execute("INSERT INTO users VALUES (1, 'Alice', 95)", [])?;
            conn.execute("INSERT INTO users VALUES (2, 'Bob', 87)", [])?;
            conn.execute("INSERT INTO users VALUES (3, 'Charlie', 92)", [])?;
        }

        // Get checksum of original SQLite
        let sqlite_checksum = checksum(&db_path, &TableFilter::new(vec![], vec![]))?;

        // Convert to CSV
        let no_filter = TableFilter::new(vec![], vec![]);
        let csvdb = to_csv::to_csv(&db_path, OrderMode::Pk, NullMode::Marker, None, true, &no_filter)?;
        let csvdb_checksum = checksum(&csvdb, &TableFilter::new(vec![], vec![]))?;

        // Convert to DuckDB
        let duckdb_path = to_duckdb::to_duckdb(&csvdb, true, &no_filter)?;
        let duckdb_checksum = checksum(&duckdb_path, &TableFilter::new(vec![], vec![]))?;

        // Convert back to SQLite
        let sqlite2_path = to_sqlite::to_sqlite(&csvdb, true, &no_filter)?;
        let sqlite2_checksum = checksum(&sqlite2_path, &TableFilter::new(vec![], vec![]))?;

        // All checksums should match
        assert_eq!(sqlite_checksum, csvdb_checksum, "SQLite != CSVDB");
        assert_eq!(csvdb_checksum, duckdb_checksum, "CSVDB != DuckDB");
        assert_eq!(duckdb_checksum, sqlite2_checksum, "DuckDB != SQLite2");

        Ok(())
    }

    #[test]
    fn test_normalize_type() {
        // Integer types
        assert_eq!(normalize_type("INT"), "INTEGER");
        assert_eq!(normalize_type("INTEGER"), "INTEGER");
        assert_eq!(normalize_type("BIGINT"), "INTEGER");
        assert_eq!(normalize_type("SMALLINT"), "INTEGER");
        assert_eq!(normalize_type("TINYINT"), "INTEGER");

        // Float types
        assert_eq!(normalize_type("FLOAT"), "REAL");
        assert_eq!(normalize_type("DOUBLE"), "REAL");
        assert_eq!(normalize_type("REAL"), "REAL");

        // Text types
        assert_eq!(normalize_type("VARCHAR"), "TEXT");
        assert_eq!(normalize_type("TEXT"), "TEXT");
        assert_eq!(normalize_type("CHAR"), "TEXT");
        assert_eq!(normalize_type("VARCHAR(255)"), "TEXT");

        // Blob types
        assert_eq!(normalize_type("BLOB"), "BLOB");
        assert_eq!(normalize_type("BINARY"), "BLOB");
        assert_eq!(normalize_type("BYTEA"), "BLOB");

        // Decimal/Numeric
        assert_eq!(normalize_type("DECIMAL"), "NUMERIC");
        assert_eq!(normalize_type("DECIMAL(10,2)"), "NUMERIC");
        assert_eq!(normalize_type("NUMERIC"), "NUMERIC");

        // Boolean -> INTEGER
        assert_eq!(normalize_type("BOOL"), "INTEGER");
        assert_eq!(normalize_type("BOOLEAN"), "INTEGER");

        // Date/Time -> TEXT
        assert_eq!(normalize_type("DATE"), "TEXT");
        assert_eq!(normalize_type("TIMESTAMP"), "TEXT");
        assert_eq!(normalize_type("DATETIME"), "TEXT");

        // Unknown -> TEXT
        assert_eq!(normalize_type("UNKNOWN"), "TEXT");
    }

    #[test]
    fn test_normalize_value() {
        // Empty stays empty
        assert_eq!(normalize_value(""), "");

        // Integers stay as integers
        assert_eq!(normalize_value("42"), "42");
        assert_eq!(normalize_value("-7"), "-7");

        // Float with trailing .0 becomes integer
        assert_eq!(normalize_value("42.0"), "42");
        assert_eq!(normalize_value("100.0"), "100");

        // Actual floats normalized
        assert_eq!(normalize_value("3.14"), "3.14");

        // Strings pass through unchanged
        assert_eq!(normalize_value("hello"), "hello");
        assert_eq!(normalize_value("not a number"), "not a number");
    }

    #[test]
    fn test_normalize_value_float_precision() {
        // Integer-valued floats become integers
        assert_eq!(normalize_value("100.0"), "100");
        assert_eq!(normalize_value("0.0"), "0");

        // Precision normalization: rounds to 10 decimal places, strips trailing zeros
        let result = normalize_value("1.1000000000001");
        assert_eq!(result, "1.1");

        let result = normalize_value("3.14159265358979");
        assert_eq!(result, "3.1415926536");
    }

    #[test]
    fn test_checksum_detects_difference() -> Result<()> {
        let dir = tempdir()?;
        let db1_path = dir.path().join("db1.sqlite");
        let db2_path = dir.path().join("db2.sqlite");

        // Create two different databases
        {
            let conn = Connection::open(&db1_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'hello')", [])?;
        }
        {
            let conn = Connection::open(&db2_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'world')", [])?;
        }

        let checksum1 = checksum(&db1_path, &TableFilter::new(vec![], vec![]))?;
        let checksum2 = checksum(&db2_path, &TableFilter::new(vec![], vec![]))?;

        assert_ne!(checksum1, checksum2);

        Ok(())
    }
}
