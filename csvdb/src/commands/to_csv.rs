use anyhow::{Context, Result, bail};
use duckdb::Connection as DuckDbConnection;
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::Connection as SqliteConnection;
use sha2::{Sha256, Digest};
use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::core::config::{CURRENT_FORMAT_VERSION, created_by_string, CsvdbConfig};
use crate::core::csv::{find_table_file, write_table_csv_gz, write_table_csv_sorted};
use crate::core::table::Table;
use crate::core::{InputFormat, Schema};
use crate::{NullMode, OrderMode, TableFilter};

/// Compute a SHA256 checksum of a Table's column names and row data.
fn table_checksum(table: &Table) -> String {
    let mut hasher = Sha256::new();
    // Hash column names
    for col in &table.columns {
        hasher.update(col.as_bytes());
        hasher.update(b"\x00");
    }
    // Hash row data (sorted by pk_values for determinism)
    let mut sorted_rows = table.rows.clone();
    sorted_rows.sort_by(|a, b| a.pk_values.cmp(&b.pk_values));
    for row in &sorted_rows {
        for val in &row.values {
            hasher.update(val.as_bytes());
            hasher.update(b"\x00");
        }
        hasher.update(b"\x01");
    }
    format!("{:x}", hasher.finalize())
}

fn resolve_output_dir(input_path: &Path, output_dir: Option<&Path>) -> PathBuf {
    match output_dir {
        Some(dir) => dir.to_path_buf(),
        None => {
            let stem = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("database");
            let stem = stem
                .strip_suffix(".csvdb")
                .or_else(|| stem.strip_suffix(".parquetdb"))
                .unwrap_or(stem);
            input_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(format!("{}.csvdb", stem))
        }
    }
}

fn build_config(
    order_mode: OrderMode,
    null_mode: NullMode,
    natural_sort: bool,
    order_by: Option<&str>,
    compress: bool,
    filter: &TableFilter,
    table_checksums: Option<BTreeMap<String, String>>,
) -> CsvdbConfig {
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
    CsvdbConfig {
        format_version: Some(CURRENT_FORMAT_VERSION.to_string()),
        created_by: Some(created_by_string()),
        order: if order_by.is_none() { Some(order_str.to_string()) } else { None },
        null_mode: Some(null_str.to_string()),
        natural_sort: if natural_sort { Some(true) } else { None },
        order_by: order_by.map(|s| s.to_string()),
        compressed: if compress { Some(true) } else { None },
        tables: if filter.tables.is_empty() { None } else { Some(filter.tables.clone()) },
        exclude: if filter.exclude.is_empty() { None } else { Some(filter.exclude.clone()) },
        table_checksums,
    }
}

/// Convert any supported format to a .csvdb directory.
///
/// If `output_dir` is None, creates a .csvdb directory next to the input file.
pub fn to_csv(input_path: &Path, order_mode: OrderMode, null_mode: NullMode, natural_sort: bool, order_by: Option<&str>, compress: bool, output_dir: Option<&Path>, force: bool, filter: &TableFilter) -> Result<PathBuf> {
    let input_format = InputFormat::from_path(input_path)?;
    let csvdb_dir = resolve_output_dir(input_path, output_dir);

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
        InputFormat::Sqlite => export_sqlite(input_path, &csvdb_dir, order_mode, null_mode, natural_sort, order_by, compress, filter)?,
        InputFormat::DuckDb => export_duckdb(input_path, &csvdb_dir, order_mode, null_mode, natural_sort, order_by, compress, filter)?,
        InputFormat::Parquetdb => export_parquetdb(input_path, &csvdb_dir, order_mode, null_mode, natural_sort, order_by, compress, filter)?,
        InputFormat::Parquet => export_single_parquet(input_path, &csvdb_dir, order_mode, null_mode, natural_sort, order_by, compress, filter)?,
        InputFormat::Csvdb => bail!("Input is already a csvdb directory"),
    }

    let config = build_config(order_mode, null_mode, natural_sort, order_by, compress, filter, None);
    config.write(&csvdb_dir)?;

    Ok(csvdb_dir)
}

/// Incremental export: only re-export tables whose data has changed.
pub fn to_csv_incremental(input_path: &Path, order_mode: OrderMode, null_mode: NullMode, natural_sort: bool, order_by: Option<&str>, compress: bool, output_dir: Option<&Path>, filter: &TableFilter) -> Result<(PathBuf, IncrementalSummary)> {
    let input_format = InputFormat::from_path(input_path)?;
    let csvdb_dir = resolve_output_dir(input_path, output_dir);

    // Load existing checksums if directory exists
    let old_checksums = if csvdb_dir.exists() {
        CsvdbConfig::load(&csvdb_dir)
            .ok()
            .and_then(|c| c.table_checksums)
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };

    // Ensure output directory exists
    fs::create_dir_all(&csvdb_dir)?;

    // We need to export to a temp location first to compute checksums,
    // then copy only changed files. Or we can compute checksums from the
    // source data directly.
    //
    // Strategy: read all tables from source, compute checksums, compare
    // with old checksums, and only write changed tables.

    let mut new_checksums = BTreeMap::new();
    let mut summary = IncrementalSummary::default();

    match input_format {
        InputFormat::Sqlite => {
            let conn = SqliteConnection::open(input_path)
                .with_context(|| format!("Failed to open SQLite database: {}", input_path.display()))?;
            let schema = Schema::from_sqlite_with_order(&conn, order_mode)?;
            let schema_path = csvdb_dir.join("schema.sql");
            schema.write_schema_sql(&schema_path)?;

            let pb = make_progress_bar(schema.tables.len() as u64);
            for (table_name, table_schema) in &schema.tables {
                if !filter.matches(table_name) {
                    pb.inc(1);
                    continue;
                }
                pb.set_message(table_name.clone());

                let result = if let Some(custom_order) = order_by {
                    Table::from_sqlite_custom_order(&conn, table_schema, custom_order, null_mode)?
                } else {
                    Table::from_sqlite_with_order(&conn, table_schema, order_mode, null_mode)?
                };
                for warning in &result.warnings {
                    eprintln!("Warning: {}", warning);
                }

                let cksum = table_checksum(&result.table);
                let changed = old_checksums.get(table_name).map(|old| old != &cksum).unwrap_or(true);

                if changed {
                    write_table(&result.table, &csvdb_dir, table_name, natural_sort, compress)?;
                    if old_checksums.contains_key(table_name) {
                        summary.updated.push(table_name.clone());
                    } else {
                        summary.added.push(table_name.clone());
                    }
                } else {
                    summary.unchanged.push(table_name.clone());
                }

                new_checksums.insert(table_name.clone(), cksum);
                pb.inc(1);
            }
            pb.finish_and_clear();
        }
        InputFormat::DuckDb => {
            let conn = DuckDbConnection::open(input_path)
                .with_context(|| format!("Failed to open DuckDB database: {}", input_path.display()))?;
            let schema = Schema::from_duckdb_with_order(&conn, order_mode)?;
            let schema_path = csvdb_dir.join("schema.sql");
            schema.write_schema_sql(&schema_path)?;

            let pb = make_progress_bar(schema.tables.len() as u64);
            for (table_name, table_schema) in &schema.tables {
                if !filter.matches(table_name) {
                    pb.inc(1);
                    continue;
                }
                pb.set_message(table_name.clone());

                let result = if let Some(custom_order) = order_by {
                    Table::from_duckdb_custom_order(&conn, table_schema, custom_order, null_mode)?
                } else {
                    Table::from_duckdb_with_order(&conn, table_schema, order_mode, null_mode)?
                };
                for warning in &result.warnings {
                    eprintln!("Warning: {}", warning);
                }

                let cksum = table_checksum(&result.table);
                let changed = old_checksums.get(table_name).map(|old| old != &cksum).unwrap_or(true);

                if changed {
                    write_table(&result.table, &csvdb_dir, table_name, natural_sort, compress)?;
                    if old_checksums.contains_key(table_name) {
                        summary.updated.push(table_name.clone());
                    } else {
                        summary.added.push(table_name.clone());
                    }
                } else {
                    summary.unchanged.push(table_name.clone());
                }

                new_checksums.insert(table_name.clone(), cksum);
                pb.inc(1);
            }
            pb.finish_and_clear();
        }
        _ => bail!("--incremental only supports SQLite and DuckDB sources"),
    }

    // Remove CSVs for tables that no longer exist in the source
    for old_table in old_checksums.keys() {
        if !new_checksums.contains_key(old_table) {
            // Remove old file (.csv or .csv.gz)
            if let Some(old_path) = find_table_file(&csvdb_dir, old_table) {
                fs::remove_file(&old_path)?;
            }
            summary.removed.push(old_table.clone());
        }
    }

    // Write config with new checksums
    let config = build_config(
        order_mode, null_mode, natural_sort, order_by, compress, filter,
        Some(new_checksums),
    );
    config.write(&csvdb_dir)?;

    Ok((csvdb_dir, summary))
}

/// Summary of what changed during an incremental export.
#[derive(Default)]
pub struct IncrementalSummary {
    pub unchanged: Vec<String>,
    pub updated: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
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

fn write_table(table: &Table, csvdb_dir: &Path, table_name: &str, natural_sort: bool, compress: bool) -> Result<()> {
    if compress {
        let gz_path = csvdb_dir.join(format!("{}.csv.gz", table_name));
        write_table_csv_gz(table, &gz_path, natural_sort)
    } else {
        let csv_path = csvdb_dir.join(format!("{}.csv", table_name));
        write_table_csv_sorted(table, &csv_path, natural_sort)
    }
}

fn export_sqlite(db_path: &Path, csvdb_dir: &Path, order_mode: OrderMode, null_mode: NullMode, natural_sort: bool, order_by: Option<&str>, compress: bool, filter: &TableFilter) -> Result<()> {
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
        let result = if let Some(custom_order) = order_by {
            Table::from_sqlite_custom_order(&conn, table_schema, custom_order, null_mode)?
        } else {
            Table::from_sqlite_with_order(&conn, table_schema, order_mode, null_mode)?
        };

        for warning in &result.warnings {
            eprintln!("Warning: {}", warning);
        }

        write_table(&result.table, csvdb_dir, table_name, natural_sort, compress)?;
        pb.inc(1);
    }
    pb.finish_and_clear();

    Ok(())
}

fn export_duckdb(db_path: &Path, csvdb_dir: &Path, order_mode: OrderMode, null_mode: NullMode, natural_sort: bool, order_by: Option<&str>, compress: bool, filter: &TableFilter) -> Result<()> {
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
        let result = if let Some(custom_order) = order_by {
            Table::from_duckdb_custom_order(&conn, table_schema, custom_order, null_mode)?
        } else {
            Table::from_duckdb_with_order(&conn, table_schema, order_mode, null_mode)?
        };

        for warning in &result.warnings {
            eprintln!("Warning: {}", warning);
        }

        write_table(&result.table, csvdb_dir, table_name, natural_sort, compress)?;
        pb.inc(1);
    }
    pb.finish_and_clear();

    Ok(())
}

fn export_parquetdb(parquetdb_path: &Path, csvdb_dir: &Path, order_mode: OrderMode, null_mode: NullMode, natural_sort: bool, order_by: Option<&str>, compress: bool, filter: &TableFilter) -> Result<()> {
    // Load parquetdb into in-memory DuckDB, then export to CSV
    let conn = DuckDbConnection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let schema_path = parquetdb_path.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    // Create tables from schema
    let schema_sql = fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() && stmt.to_uppercase().starts_with("CREATE TABLE") {
            conn.execute(stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    // Load parquet data
    for table_name in schema.tables.keys() {
        let parquet_path = parquetdb_path.join(format!("{}.parquet", table_name));
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

    // Write schema.sql (including views and indexes)
    let out_schema_path = csvdb_dir.join("schema.sql");
    schema.write_schema_sql(&out_schema_path)?;

    // Export each table to CSV
    let pb = make_progress_bar(schema.tables.len() as u64);
    for (table_name, table_schema) in &schema.tables {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }

        pb.set_message(table_name.clone());
        let result = if let Some(custom_order) = order_by {
            Table::from_duckdb_custom_order(&conn, table_schema, custom_order, null_mode)?
        } else {
            Table::from_duckdb_with_order(&conn, table_schema, order_mode, null_mode)?
        };

        for warning in &result.warnings {
            eprintln!("Warning: {}", warning);
        }

        write_table(&result.table, csvdb_dir, table_name, natural_sort, compress)?;
        pb.inc(1);
    }
    pb.finish_and_clear();

    Ok(())
}

fn export_single_parquet(parquet_path: &Path, csvdb_dir: &Path, order_mode: OrderMode, null_mode: NullMode, natural_sort: bool, order_by: Option<&str>, compress: bool, filter: &TableFilter) -> Result<()> {
    // Load single parquet into in-memory DuckDB
    let conn = DuckDbConnection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let table_name = parquet_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("table");

    if !filter.matches(table_name) {
        bail!("Table '{}' is excluded by filter", table_name);
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
    let schema = Schema::from_duckdb_with_order(&conn, order_mode)?;

    // Write schema.sql
    let schema_path = csvdb_dir.join("schema.sql");
    schema.write_schema_sql(&schema_path)?;

    // Export to CSV
    let table_schema = schema.tables.get(table_name)
        .ok_or_else(|| anyhow::anyhow!("Table not found after creation"))?;

    let result = if let Some(custom_order) = order_by {
        Table::from_duckdb_custom_order(&conn, table_schema, custom_order, null_mode)?
    } else {
        Table::from_duckdb_with_order(&conn, table_schema, order_mode, null_mode)?
    };

    for warning in &result.warnings {
        eprintln!("Warning: {}", warning);
    }

    write_table(&result.table, csvdb_dir, table_name, natural_sort, compress)?;

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
        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, false, None, false, None, true, &TableFilter::new(vec![], vec![]))?;

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

        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, false, None, false, Some(&output), true, &TableFilter::new(vec![], vec![]))?;

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

        let result = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, false, None, false, None, true, &TableFilter::new(vec![], vec![]));
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

        let csvdb = to_csv(&db_path, OrderMode::AllColumns, NullMode::Marker, false, None, false, None, true, &TableFilter::new(vec![], vec![]))?;
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

        let csvdb = to_csv(&db_path, OrderMode::AddSyntheticKey, NullMode::Marker, false, None, false, None, true, &TableFilter::new(vec![], vec![]))?;
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
        let csvdb = to_csv(&db_path, OrderMode::Pk, NullMode::Marker, false, None, false, None, true, &TableFilter::new(vec![], vec![]))?;

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
