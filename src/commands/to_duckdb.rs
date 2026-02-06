use anyhow::{bail, Context, Result};
use duckdb::Connection;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::core::{InputFormat, Schema};
use crate::{NullMode, OrderMode, TableFilter};

/// Convert any supported format to a DuckDB database.
pub fn to_duckdb(input_path: &Path, force: bool, filter: &TableFilter) -> Result<PathBuf> {
    let input_format = InputFormat::from_path(input_path)?;

    // For non-csvdb formats, convert to csvdb first in a temp directory
    let (csvdb_dir, _temp_dir) = match input_format {
        InputFormat::Csvdb => (input_path.to_path_buf(), None),
        _ => {
            let temp_dir = tempfile::tempdir()?;
            let temp_csvdb = temp_dir.path().join("temp.csvdb");
            crate::commands::to_csv::to_csv(
                input_path,
                OrderMode::Pk,
                NullMode::Marker,
                Some(&temp_csvdb),
                true,
                filter,
            )?;
            (temp_csvdb, Some(temp_dir))
        }
    };

    let schema_path = csvdb_dir.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    // Determine output path based on input
    let stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("database");
    let stem = stem
        .strip_suffix(".csvdb")
        .or_else(|| stem.strip_suffix(".parquetdb"))
        .unwrap_or(stem);

    let db_name = format!("{}.duckdb", stem);
    let db_path = input_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(db_name);

    // Check for existing database
    if db_path.exists() {
        if !force {
            bail!(
                "Output file already exists: {}\nUse --force to overwrite.",
                db_path.display()
            );
        }
        fs::remove_file(&db_path)?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to create database: {}", db_path.display()))?;

    // Create tables from schema
    let schema_sql = fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() {
            conn.execute(stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    // Import data from CSV files using DuckDB's native COPY command (filtered)
    let pb = if std::io::stderr().is_terminal() {
        let pb = ProgressBar::new(schema.tables.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{bar:40}] {pos}/{len} {msg}")
                .unwrap(),
        );
        pb
    } else {
        ProgressBar::hidden()
    };
    for table_name in schema.tables.keys() {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }
        pb.set_message(table_name.clone());
        let csv_path = csvdb_dir.join(format!("{}.csv", table_name));
        if csv_path.exists() {
            // Get absolute path and convert to forward slashes for DuckDB
            let abs_path = csv_path.canonicalize()
                .with_context(|| format!("Failed to get absolute path: {}", csv_path.display()))?;
            let path_str = abs_path.to_string_lossy().replace('\\', "/");

            // Remove Windows UNC prefix if present (\\?\)
            let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

            let copy_sql = format!(
                "COPY \"{}\" FROM '{}' (HEADER, NULL '\\N')",
                table_name, path_str
            );
            conn.execute(&copy_sql, [])
                .with_context(|| format!("Failed to import CSV for table {}", table_name))?;
        }
        pb.inc(1);
    }
    pb.finish_and_clear();

    Ok(db_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::to_csv::to_csv;
    use crate::{OrderMode, NullMode, TableFilter};
    use rusqlite::Connection as SqliteConnection;
    use tempfile::tempdir;

    #[test]
    fn test_csvdb_to_duckdb() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        // Create test database with SQLite
        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                [],
            )?;
            conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])?;
            conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])?;
        }

        // Convert to CSV
        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]))?;

        // Convert to DuckDB
        let duckdb_path = to_duckdb(&csvdb, true, &TableFilter::new(vec![], vec![]))?;

        // Verify data
        let conn = duckdb::Connection::open(&duckdb_path)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        assert_eq!(count, 2);

        let name: String = conn.query_row(
            "SELECT name FROM users WHERE id = 1",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(name, "Alice");

        Ok(())
    }

    #[test]
    fn test_sqlite_to_duckdb_roundtrip() -> Result<()> {
        use crate::commands::to_sqlite::to_sqlite;

        let dir = tempdir()?;
        let original_db = dir.path().join("original.sqlite");

        // Create original SQLite database
        {
            let conn = SqliteConnection::open(&original_db)?;
            conn.execute(
                "CREATE TABLE orders (
                    id INTEGER PRIMARY KEY,
                    customer TEXT NOT NULL,
                    amount REAL NOT NULL
                )",
                [],
            )?;
            conn.execute("INSERT INTO orders VALUES (1, 'Alice', 99.99)", [])?;
            conn.execute("INSERT INTO orders VALUES (2, 'Bob', 149.50)", [])?;
            conn.execute("INSERT INTO orders VALUES (3, 'Alice', 25.00)", [])?;
        }

        // SQLite -> CSV
        let csvdb = to_csv(&original_db, OrderMode::Pk, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]))?;

        // CSV -> DuckDB
        let duckdb_path = to_duckdb(&csvdb, true, &TableFilter::new(vec![], vec![]))?;

        // Verify DuckDB data
        let duck_conn = duckdb::Connection::open(&duckdb_path)?;
        let total: f64 = duck_conn.query_row(
            "SELECT SUM(amount) FROM orders WHERE customer = 'Alice'",
            [],
            |r| r.get(0),
        )?;
        assert!((total - 124.99).abs() < 0.01);

        // CSV -> SQLite (verify both targets work from same CSV)
        let sqlite_path = to_sqlite(&csvdb, true, &TableFilter::new(vec![], vec![]))?;
        let sqlite_conn = SqliteConnection::open(&sqlite_path)?;
        let sqlite_total: f64 = sqlite_conn.query_row(
            "SELECT SUM(amount) FROM orders WHERE customer = 'Alice'",
            [],
            |r| r.get(0),
        )?;
        assert!((sqlite_total - 124.99).abs() < 0.01);

        Ok(())
    }

    #[test]
    fn test_output_path_csvdb_suffix() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("foo.csvdb");
        std::fs::create_dir(&csvdb_dir)?;
        std::fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        std::fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;

        let db_path = to_duckdb(&csvdb_dir, true, &TableFilter::new(vec![], vec![]))?;
        assert!(db_path.file_name().unwrap().to_str().unwrap().ends_with("foo.duckdb"));
        Ok(())
    }

    #[test]
    fn test_output_path_no_suffix() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("bar");
        std::fs::create_dir(&csvdb_dir)?;
        std::fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        std::fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;

        let db_path = to_duckdb(&csvdb_dir, true, &TableFilter::new(vec![], vec![]))?;
        // Input "bar" (no .csvdb suffix) -> output "bar.duckdb"
        assert!(db_path.file_name().unwrap().to_str().unwrap().ends_with("bar.duckdb"));
        Ok(())
    }

    #[test]
    fn test_force_overwrites() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("f.csvdb");
        std::fs::create_dir(&csvdb_dir)?;
        std::fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        std::fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;

        let db_path = to_duckdb(&csvdb_dir, true, &TableFilter::new(vec![], vec![]))?;
        assert!(db_path.exists());

        // Force overwrite should succeed
        let db_path2 = to_duckdb(&csvdb_dir, true, &TableFilter::new(vec![], vec![]))?;
        assert!(db_path2.exists());
        Ok(())
    }

    #[test]
    fn test_null_values_roundtrip() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("nulls.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score INTEGER)",
                [],
            )?;
            conn.execute("INSERT INTO t VALUES (1, NULL, NULL)", [])?;
            conn.execute("INSERT INTO t VALUES (2, 'Bob', 42)", [])?;
        }

        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, None, true, &TableFilter::new(vec![], vec![]))?;
        let duckdb_path = to_duckdb(&csvdb, true, &TableFilter::new(vec![], vec![]))?;

        let conn = duckdb::Connection::open(&duckdb_path)?;

        // Row 1: NULLs preserved
        let name: Option<String> = conn.query_row(
            "SELECT name FROM t WHERE id = 1", [], |r| r.get(0),
        )?;
        assert_eq!(name, None);

        let score: Option<i64> = conn.query_row(
            "SELECT score FROM t WHERE id = 1", [], |r| r.get(0),
        )?;
        assert_eq!(score, None);

        // Row 2: values preserved
        let name: Option<String> = conn.query_row(
            "SELECT name FROM t WHERE id = 2", [], |r| r.get(0),
        )?;
        assert_eq!(name, Some("Bob".to_string()));

        let score: Option<i64> = conn.query_row(
            "SELECT score FROM t WHERE id = 2", [], |r| r.get(0),
        )?;
        assert_eq!(score, Some(42));

        Ok(())
    }
}
