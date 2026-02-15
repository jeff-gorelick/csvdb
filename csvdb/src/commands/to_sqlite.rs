use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::Connection;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::csv::{find_table_file, read_table_csv_auto};
use crate::core::{InputFormat, Schema};
use crate::{NullMode, OrderMode, TableFilter};

/// Convert any supported format to a SQLite database.
pub fn to_sqlite(input_path: &Path, output: Option<&Path>, force: bool, filter: &TableFilter) -> Result<PathBuf> {
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
                false,
                None,
                false,
                Some(&temp_csvdb),
                true,
                filter,
            )?;
            (temp_csvdb, Some(temp_dir))
        }
    };

    let schema_path = csvdb_dir.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    // Determine output path
    let db_path = if let Some(out) = output {
        out.to_path_buf()
    } else {
        let stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("database");
        let stem = stem
            .strip_suffix(".csvdb")
            .or_else(|| stem.strip_suffix(".parquetdb"))
            .unwrap_or(stem);
        let db_name = format!("{}.sqlite", stem);
        input_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(db_name)
    };

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

    // Check if any table files are compressed (.csv.gz)
    let has_compressed = schema.tables.keys().any(|t| {
        csvdb_dir.join(format!("{}.csv.gz", t)).exists()
    });

    // Try to use sqlite3 CLI for fast import, fall back to rusqlite if unavailable.
    // sqlite3 CLI cannot read .csv.gz files, so use rusqlite for compressed data.
    if sqlite3_available() && !has_compressed {
        to_sqlite_via_cli(&csvdb_dir, &db_path, &schema_path, &schema, filter)
    } else {
        to_sqlite_via_rusqlite(&csvdb_dir, &db_path, &schema_path, &schema, filter)
    }
}

/// Check if sqlite3 CLI is available
fn sqlite3_available() -> bool {
    Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Fast import using sqlite3 CLI's .import command
fn to_sqlite_via_cli(
    csvdb_dir: &Path,
    db_path: &Path,
    schema_path: &Path,
    schema: &Schema,
    filter: &TableFilter,
) -> Result<PathBuf> {
    use std::io::Write;

    let abs_db_path = if db_path.is_absolute() {
        db_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(db_path)
    };

    // Build sqlite3 commands
    let mut commands = String::new();

    // Disable FK enforcement during import (some sqlite3 builds enable it by default)
    commands.push_str("PRAGMA foreign_keys = OFF;\n");

    // Read and execute schema
    let abs_schema_path = schema_path.canonicalize()
        .with_context(|| format!("Failed to get absolute path: {}", schema_path.display()))?;
    commands.push_str(&format!(".read '{}'\n", abs_schema_path.display()));

    // Import each CSV file (filtered)
    commands.push_str(".mode csv\n");
    for table_name in schema.tables.keys() {
        if !filter.matches(table_name) {
            continue;
        }
        if let Some(csv_path) = find_table_file(csvdb_dir, table_name) {
            let abs_csv_path = csv_path.canonicalize()
                .with_context(|| format!("Failed to get absolute path: {}", csv_path.display()))?;
            // .import with --skip 1 to skip header row
            commands.push_str(&format!(
                ".import --skip 1 '{}' {}\n",
                abs_csv_path.display(),
                table_name
            ));
        }
    }

    // Convert \N markers to actual NULL values.
    // sqlite3's .nullvalue does not affect .import, so we fix up after import.
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            continue;
        }
        for col in &table_schema.columns {
            commands.push_str(&format!(
                "UPDATE \"{}\" SET \"{}\" = NULL WHERE \"{}\" = '\\N';\n",
                table_name, col.name, col.name
            ));
        }
    }

    // Run sqlite3 with commands via stdin
    let mut child = Command::new("sqlite3")
        .arg(&abs_db_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to start sqlite3")?;

    // Write commands to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(commands.as_bytes())
            .context("Failed to write to sqlite3 stdin")?;
    }

    let output = child.wait_with_output()
        .context("Failed to run sqlite3")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("sqlite3 import failed: {}", stderr);
    }

    Ok(db_path.to_path_buf())
}

/// Fallback import using rusqlite (slower but always available)
fn to_sqlite_via_rusqlite(
    csvdb_dir: &Path,
    db_path: &Path,
    schema_path: &Path,
    schema: &Schema,
    filter: &TableFilter,
) -> Result<PathBuf> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("Failed to create database: {}", db_path.display()))?;

    // Disable FK enforcement during import to avoid ordering issues
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;

    // Create tables from schema
    let schema_sql = fs::read_to_string(schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() {
            conn.execute(stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    // Import data from CSV files (within a transaction for performance)
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
    conn.execute("BEGIN TRANSACTION", [])?;
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }
        pb.set_message(table_name.clone());
        if let Some(csv_path) = find_table_file(csvdb_dir, table_name) {
            let table = read_table_csv_auto(&csv_path, table_schema)?;
            table.write_to_sqlite(&conn)?;
        }
        pb.inc(1);
    }
    conn.execute("COMMIT", [])?;
    pb.finish_and_clear();

    Ok(db_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::to_csv::to_csv;
    use crate::{OrderMode, NullMode, TableFilter};
    use tempfile::tempdir;

    #[test]
    fn test_roundtrip() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        // Create test database
        {
            let conn = Connection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                [],
            )?;
            conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])?;
            conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])?;
        }

        // Convert to CSV
        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, false, None, false, None, true, &TableFilter::new(vec![], vec![]))?;

        // Remove original database
        fs::remove_file(&db_path)?;

        // Rebuild from CSV
        let rebuilt_path = to_sqlite(&csvdb, None, true, &TableFilter::new(vec![], vec![]))?;

        // Verify data
        let conn = Connection::open(&rebuilt_path)?;
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
    fn test_output_path_csvdb_suffix() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("foo.csvdb");
        fs::create_dir(&csvdb_dir)?;
        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;

        let db_path = to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec![], vec![]))?;
        assert!(db_path.file_name().unwrap().to_str().unwrap().ends_with("foo.sqlite"));
        Ok(())
    }

    #[test]
    fn test_output_path_no_suffix() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("bar");
        fs::create_dir(&csvdb_dir)?;
        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;

        let db_path = to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec![], vec![]))?;
        // Input "bar" (no .csvdb suffix) -> output "bar.sqlite"
        assert!(db_path.file_name().unwrap().to_str().unwrap().ends_with("bar.sqlite"));
        Ok(())
    }

    #[test]
    fn test_force_overwrites() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("f.csvdb");
        fs::create_dir(&csvdb_dir)?;
        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;

        // First create
        let db_path = to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec![], vec![]))?;
        assert!(db_path.exists());

        // Force overwrite should succeed
        let db_path2 = to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec![], vec![]))?;
        assert!(db_path2.exists());
        Ok(())
    }

    #[test]
    fn test_no_force_rejects_existing() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("nf.csvdb");
        fs::create_dir(&csvdb_dir)?;
        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;

        // First create with force
        to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec![], vec![]))?;

        // Second create without force should fail
        let result = to_sqlite(&csvdb_dir, None, false, &TableFilter::new(vec![], vec![]));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("--force"));
        Ok(())
    }

    #[test]
    fn test_compressed_csv_to_sqlite() -> Result<()> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("gz.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;

        // Write compressed CSV
        let gz_path = csvdb_dir.join("t.csv.gz");
        let f = fs::File::create(&gz_path)?;
        let mut encoder = GzEncoder::new(f, Compression::default());
        encoder.write_all(b"\"id\",\"name\"\n\"1\",\"Alice\"\n\"2\",\"Bob\"\n")?;
        encoder.finish()?;

        // This forces the rusqlite path (sqlite3 CLI can't read .csv.gz)
        let db_path = to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec![], vec![]))?;

        let conn = Connection::open(&db_path)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))?;
        assert_eq!(count, 2);

        let name: String = conn.query_row("SELECT name FROM t WHERE id = 1", [], |r| r.get(0))?;
        assert_eq!(name, "Alice");
        Ok(())
    }

    #[test]
    fn test_sqlite_to_sqlite_via_csvdb() -> Result<()> {
        let dir = tempdir()?;
        let src_db = dir.path().join("src.sqlite");

        {
            let conn = Connection::open(&src_db)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'test')", [])?;
        }

        // SQLite input (non-csvdb) goes through to_csv internally
        let output = dir.path().join("out.sqlite");
        let db_path = to_sqlite(&src_db, Some(&output), true, &TableFilter::new(vec![], vec![]))?;

        let conn = Connection::open(&db_path)?;
        let val: String = conn.query_row("SELECT val FROM t WHERE id = 1", [], |r| r.get(0))?;
        assert_eq!(val, "test");
        Ok(())
    }

    #[test]
    fn test_duckdb_to_sqlite() -> Result<()> {
        let dir = tempdir()?;
        let duckdb_path = dir.path().join("src.duckdb");

        {
            let conn = duckdb::Connection::open(&duckdb_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name VARCHAR)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'Duck')", [])?;
        }

        let output = dir.path().join("out.sqlite");
        let db_path = to_sqlite(&duckdb_path, Some(&output), true, &TableFilter::new(vec![], vec![]))?;

        let conn = Connection::open(&db_path)?;
        let name: String = conn.query_row("SELECT name FROM t WHERE id = 1", [], |r| r.get(0))?;
        assert_eq!(name, "Duck");
        Ok(())
    }

    #[test]
    fn test_to_sqlite_with_nulls() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("nulls.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "\"id\",\"name\"\n\"1\",\"Alice\"\n\"2\",\"\\N\"\n")?;

        let db_path = to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec![], vec![]))?;

        let conn = Connection::open(&db_path)?;
        let name: Option<String> = conn.query_row("SELECT name FROM t WHERE id = 2", [], |r| r.get(0))?;
        assert_eq!(name, None);
        Ok(())
    }

    #[test]
    fn test_to_sqlite_table_filter() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("filter.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t1\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n\
             CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        fs::write(csvdb_dir.join("t1.csv"), "id\n1\n")?;
        fs::write(csvdb_dir.join("t2.csv"), "id\n1\n")?;

        let db_path = to_sqlite(&csvdb_dir, None, true, &TableFilter::new(vec!["t1".to_string()], vec![]))?;

        let conn = Connection::open(&db_path)?;
        // t1 should have data
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM t1", [], |r| r.get(0))?;
        assert_eq!(count, 1);

        // t2 exists (schema created) but has no data
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM t2", [], |r| r.get(0))?;
        assert_eq!(count, 0);
        Ok(())
    }
}
