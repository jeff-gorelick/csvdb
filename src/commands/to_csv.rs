use anyhow::{Context, Result, bail};
use duckdb::Connection as DuckDbConnection;
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::Connection as SqliteConnection;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::core::Schema;
use crate::core::csv::write_table_csv;
use crate::core::table::Table;
use crate::{OrderMode, NullMode, TableFilter, CsvdbConfig};
use crate::core::config::{CURRENT_FORMAT_VERSION, created_by_string};

/// Detected input format type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Sqlite,
    DuckDb,
}

impl InputFormat {
    /// Detect input format by examining file extension.
    pub fn from_path(path: &Path) -> Result<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        match ext.as_deref() {
            Some("sqlite") | Some("sqlite3") | Some("db") => Ok(InputFormat::Sqlite),
            Some("duckdb") => Ok(InputFormat::DuckDb),
            _ => bail!(
                "Cannot detect input format for: {}. \
                 Supported: SQLite (.sqlite, .db), DuckDB (.duckdb).",
                path.display()
            ),
        }
    }
}

/// Convert a database (SQLite or DuckDB) to a .csvdb directory.
///
/// If `output_dir` is None, creates a .csvdb directory next to the input file.
pub fn to_csv(input_path: &Path, order_mode: OrderMode, null_mode: NullMode, output_dir: Option<&Path>, force: bool, filter: &TableFilter) -> Result<PathBuf> {
    let input_format = InputFormat::from_path(input_path)?;

    // Determine output directory
    let csvdb_dir = match output_dir {
        Some(dir) => dir.to_path_buf(),
        None => input_path.with_extension("csvdb"),
    };

    // Check for existing output
    if csvdb_dir.exists() {
        if !force {
            bail!(
                "Output directory already exists: {}\nUse --force to overwrite.",
                csvdb_dir.display()
            );
        }
        fs::remove_dir_all(&csvdb_dir)?;
    }
    fs::create_dir_all(&csvdb_dir)?;

    match input_format {
        InputFormat::Sqlite => export_sqlite(input_path, &csvdb_dir, order_mode, null_mode, filter)?,
        InputFormat::DuckDb => export_duckdb(input_path, &csvdb_dir, order_mode, null_mode, filter)?,
    }

    // Write csvdb.toml with effective settings
    let order_str = match order_mode {
        OrderMode::Pk => "pk",
        OrderMode::AllColumns => "all-columns",
        OrderMode::AddSyntheticKey => "add-synthetic-key",
    };
    let null_str = match null_mode {
        NullMode::Marker => "marker",
        NullMode::Empty => "empty",
        NullMode::Literal => "literal",
    };
    let config = CsvdbConfig {
        format_version: Some(CURRENT_FORMAT_VERSION.to_string()),
        created_by: Some(created_by_string()),
        order: Some(order_str.to_string()),
        null_mode: Some(null_str.to_string()),
        tables: if filter.tables.is_empty() { None } else { Some(filter.tables.clone()) },
        exclude: if filter.exclude.is_empty() { None } else { Some(filter.exclude.clone()) },
    };
    config.write(&csvdb_dir)?;

    Ok(csvdb_dir)
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

fn export_sqlite(db_path: &Path, csvdb_dir: &Path, order_mode: OrderMode, null_mode: NullMode, filter: &TableFilter) -> Result<()> {
    let conn = SqliteConnection::open(db_path)
        .with_context(|| format!("Failed to open SQLite database: {}", db_path.display()))?;

    let schema = Schema::from_sqlite_with_order(&conn, order_mode)?;

    // Write schema.sql
    let schema_path = csvdb_dir.join("schema.sql");
    schema.write_schema_sql(&schema_path)?;

    // Export each table to CSV (filtered)
    let pb = make_progress_bar(schema.tables.len() as u64);
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }

        pb.set_message(table_name.clone());
        let result = Table::from_sqlite_with_order(&conn, table_schema, order_mode, null_mode)?;

        for warning in &result.warnings {
            eprintln!("Warning: {}", warning);
        }

        let csv_path = csvdb_dir.join(format!("{}.csv", table_name));
        write_table_csv(&result.table, &csv_path)?;
        pb.inc(1);
    }
    pb.finish_and_clear();

    Ok(())
}

fn export_duckdb(db_path: &Path, csvdb_dir: &Path, order_mode: OrderMode, null_mode: NullMode, filter: &TableFilter) -> Result<()> {
    let conn = DuckDbConnection::open(db_path)
        .with_context(|| format!("Failed to open DuckDB database: {}", db_path.display()))?;

    let schema = Schema::from_duckdb_with_order(&conn, order_mode)?;

    // Write schema.sql
    let schema_path = csvdb_dir.join("schema.sql");
    schema.write_schema_sql(&schema_path)?;

    // Export each table to CSV (filtered)
    let pb = make_progress_bar(schema.tables.len() as u64);
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }

        pb.set_message(table_name.clone());
        let result = Table::from_duckdb_with_order(&conn, table_schema, order_mode, null_mode)?;

        for warning in &result.warnings {
            eprintln!("Warning: {}", warning);
        }

        let csv_path = csvdb_dir.join(format!("{}.csv", table_name));
        write_table_csv(&result.table, &csv_path)?;
        pb.inc(1);
    }
    pb.finish_and_clear();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_to_csv() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        // Create test database
        let conn = SqliteConnection::open(&db_path)?;
        conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            [],
        )?;
        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])?;
        conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])?;
        drop(conn);

        // Convert to CSV
        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]))?;

        // Verify structure
        assert!(csvdb.join("schema.sql").exists());
        assert!(csvdb.join("users.csv").exists());
        assert!(csvdb.to_string_lossy().ends_with(".csvdb"));

        Ok(())
    }

    #[test]
    fn test_to_csv_custom_output() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");
        let output = dir.path().join("custom_output");

        let conn = SqliteConnection::open(&db_path)?;
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", [])?;
        drop(conn);

        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, Some(&output), true, &TableFilter::new(vec![], vec![]))?;

        assert_eq!(csvdb, output);
        assert!(csvdb.join("schema.sql").exists());

        Ok(())
    }

    #[test]
    fn test_to_csv_no_pk_fails_with_pk_mode() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        let conn = SqliteConnection::open(&db_path)?;
        conn.execute("CREATE TABLE events (timestamp TEXT, message TEXT)", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'test')", [])?;
        drop(conn);

        let result = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("events"));
        assert!(err.contains("--order=all-columns"));

        Ok(())
    }

    #[test]
    fn test_to_csv_all_columns_mode() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        let conn = SqliteConnection::open(&db_path)?;
        conn.execute("CREATE TABLE events (timestamp TEXT, message TEXT)", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'test')", [])?;
        drop(conn);

        let csvdb = to_csv(&db_path, OrderMode::AllColumns, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]))?;
        assert!(csvdb.join("events.csv").exists());

        Ok(())
    }

    #[test]
    fn test_to_csv_synthetic_key_mode() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        let conn = SqliteConnection::open(&db_path)?;
        conn.execute("CREATE TABLE events (timestamp TEXT, message TEXT)", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'test')", [])?;
        drop(conn);

        let csvdb = to_csv(&db_path, OrderMode::AddSyntheticKey, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]))?;
        assert!(csvdb.join("events.csv").exists());

        // Read the CSV and verify it has the synthetic key column
        // Note: CSV uses quoted fields, so header is "__csvdb_rowid"
        let csv_content = fs::read_to_string(csvdb.join("events.csv"))?;
        assert!(csv_content.starts_with("\"__csvdb_rowid\""));

        Ok(())
    }

    #[test]
    fn test_to_csv_from_duckdb() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.duckdb");

        // Create DuckDB database
        let conn = DuckDbConnection::open(&db_path)?;
        conn.execute(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name VARCHAR NOT NULL)",
            [],
        )?;
        conn.execute("INSERT INTO items VALUES (1, 'Apple')", [])?;
        conn.execute("INSERT INTO items VALUES (2, 'Banana')", [])?;
        drop(conn);

        // Convert to CSV
        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]))?;

        assert!(csvdb.join("schema.sql").exists());
        assert!(csvdb.join("items.csv").exists());

        // Verify content
        let csv_content = fs::read_to_string(csvdb.join("items.csv"))?;
        assert!(csv_content.contains("Apple"));
        assert!(csv_content.contains("Banana"));

        Ok(())
    }

    #[test]
    fn test_db_type_detection() {
        assert_eq!(InputFormat::from_path(Path::new("test.sqlite")).unwrap(), InputFormat::Sqlite);
        assert_eq!(InputFormat::from_path(Path::new("test.sqlite3")).unwrap(), InputFormat::Sqlite);
        assert_eq!(InputFormat::from_path(Path::new("test.db")).unwrap(), InputFormat::Sqlite);
        assert_eq!(InputFormat::from_path(Path::new("test.duckdb")).unwrap(), InputFormat::DuckDb);
        assert!(InputFormat::from_path(Path::new("test.unknown")).is_err());
    }
}
