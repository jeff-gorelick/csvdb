use anyhow::{bail, Context, Result};
use duckdb::Connection;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::core::config::{created_by_string, CsvdbConfig, CURRENT_FORMAT_VERSION};
use crate::core::{InputFormat, Schema};
use crate::{NullMode, OrderMode, TableFilter};

/// Convert any supported format to a .parquetdb directory.
pub fn to_parquetdb(
    input_path: &Path,
    order_mode: OrderMode,
    null_mode: NullMode,
    order_by: Option<&str>,
    output_dir: Option<&Path>,
    force: bool,
    filter: &TableFilter,
) -> Result<PathBuf> {
    let input_format = InputFormat::from_path(input_path)?;

    // Determine output directory
    let parquetdb_dir = match output_dir {
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
                .join(format!("{}.parquetdb", stem))
        }
    };

    // Check for existing output
    if parquetdb_dir.exists() {
        if !force {
            bail!(
                "Output directory already exists: {}\nUse --force to overwrite.",
                parquetdb_dir.display()
            );
        }
        fs::remove_dir_all(&parquetdb_dir)?;
    }
    fs::create_dir_all(&parquetdb_dir)?;

    // Load into in-memory DuckDB, then export to parquet
    let conn = Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let schema = match input_format {
        InputFormat::Sqlite => load_sqlite(&conn, input_path)?,
        InputFormat::DuckDb => load_duckdb(&conn, input_path)?,
        InputFormat::Csvdb => load_csvdb(&conn, input_path)?,
        InputFormat::Parquetdb => load_parquetdb(&conn, input_path)?,
        InputFormat::Parquet => load_single_parquet(&conn, input_path)?,
    };

    // Export each table to parquet
    let pb = make_progress_bar(schema.tables.len() as u64);
    for table_name in schema.tables.keys() {
        if !filter.matches(table_name) {
            pb.inc(1);
            continue;
        }

        pb.set_message(table_name.clone());

        let parquet_path = parquetdb_dir.join(format!("{}.parquet", table_name));
        let abs_path = parquet_path
            .canonicalize()
            .unwrap_or_else(|_| parquet_path.clone());
        let path_str = abs_path.to_string_lossy().replace('\\', "/");
        let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

        // Build ORDER BY clause based on order_by or order_mode
        let order_clause = if let Some(custom_order) = order_by {
            format!("ORDER BY {}", custom_order)
        } else {
            build_order_clause(&schema.tables[table_name], order_mode)
        };

        let copy_sql = format!(
            "COPY (SELECT * FROM \"{}\" {}) TO '{}' (FORMAT PARQUET)",
            table_name, order_clause, path_str
        );

        conn.execute(&copy_sql, [])
            .with_context(|| format!("Failed to export table {} to parquet", table_name))?;

        pb.inc(1);
    }
    pb.finish_and_clear();

    // Write schema.sql
    let schema_path = parquetdb_dir.join("schema.sql");
    schema.write_schema_sql(&schema_path)?;

    // Write csvdb.toml
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
        order: if order_by.is_none() { Some(order_str.to_string()) } else { None },
        null_mode: Some(null_str.to_string()),
        natural_sort: None,
        order_by: order_by.map(|s| s.to_string()),
        compressed: None,
        tables: if filter.tables.is_empty() {
            None
        } else {
            Some(filter.tables.clone())
        },
        exclude: if filter.exclude.is_empty() {
            None
        } else {
            Some(filter.exclude.clone())
        },
        table_checksums: None,
    };
    config.write(&parquetdb_dir)?;

    Ok(parquetdb_dir)
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

fn build_order_clause(
    table_schema: &crate::core::TableSchema,
    order_mode: OrderMode,
) -> String {
    match order_mode {
        OrderMode::Pk => {
            if table_schema.pk_columns.is_empty() {
                String::new()
            } else {
                let pk_cols: Vec<String> = table_schema
                    .pk_columns
                    .iter()
                    .map(|c| format!("\"{}\"", c))
                    .collect();
                format!("ORDER BY {}", pk_cols.join(", "))
            }
        }
        OrderMode::AllColumns => {
            let all_cols: Vec<String> = table_schema
                .columns
                .iter()
                .map(|c| format!("\"{}\"", c.name))
                .collect();
            format!("ORDER BY {}", all_cols.join(", "))
        }
        OrderMode::AddSyntheticKey => {
            // Synthetic key should already be in the data
            format!("ORDER BY \"{}\"", crate::core::SYNTHETIC_KEY_COLUMN)
        }
    }
}

fn load_sqlite(conn: &Connection, path: &Path) -> Result<Schema> {
    let sqlite_conn = rusqlite::Connection::open(path)
        .with_context(|| format!("Failed to open SQLite database: {}", path.display()))?;

    let schema = Schema::from_sqlite(&sqlite_conn)?;

    // Attach SQLite and copy data
    let abs_path = path.canonicalize()?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");

    conn.execute(&format!("ATTACH '{}' AS src (TYPE SQLITE)", path_str), [])?;

    for table_name in schema.tables.keys() {
        conn.execute(
            &format!(
                "CREATE TABLE \"{}\" AS SELECT * FROM src.\"{}\"",
                table_name, table_name
            ),
            [],
        )?;
    }

    conn.execute("DETACH src", [])?;

    Ok(schema)
}

fn load_duckdb(conn: &Connection, path: &Path) -> Result<Schema> {
    // Scope the connection so it's dropped before ATTACH (avoids Windows file lock)
    let schema = {
        let src_conn = Connection::open(path)
            .with_context(|| format!("Failed to open DuckDB database: {}", path.display()))?;
        Schema::from_duckdb(&src_conn)?
    };

    // Attach and copy data
    let abs_path = path.canonicalize()?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");
    // Strip Windows UNC prefix (//?/) that canonicalize() adds
    let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

    conn.execute(&format!("ATTACH '{}' AS src", path_str), [])?;

    for table_name in schema.tables.keys() {
        conn.execute(
            &format!(
                "CREATE TABLE \"{}\" AS SELECT * FROM src.\"{}\"",
                table_name, table_name
            ),
            [],
        )?;
    }

    conn.execute("DETACH src", [])?;

    Ok(schema)
}

fn load_csvdb(conn: &Connection, path: &Path) -> Result<Schema> {
    let schema_path = path.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    // Create tables and load CSV data
    // Replace REAL with DOUBLE to avoid 32-bit precision loss in DuckDB
    let schema_sql = fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() && stmt.to_uppercase().starts_with("CREATE TABLE") {
            let stmt = stmt.replace(" REAL", " DOUBLE");
            conn.execute(&stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    for table_name in schema.tables_in_fk_order()? {
        let csv_path = path.join(format!("{}.csv", table_name));
        if csv_path.exists() {
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

    Ok(schema)
}

fn load_parquetdb(conn: &Connection, path: &Path) -> Result<Schema> {
    let schema_path = path.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    // Create tables from schema and load parquet data
    // Replace REAL with DOUBLE to avoid 32-bit precision loss in DuckDB
    let schema_sql = fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() && stmt.to_uppercase().starts_with("CREATE TABLE") {
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

    Ok(schema)
}

fn load_single_parquet(conn: &Connection, path: &Path) -> Result<Schema> {
    let table_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("table");

    let abs_path = path.canonicalize()?;
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

    // Build schema from DuckDB's view of the table
    Schema::from_duckdb(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection as SqliteConnection;
    use tempfile::tempdir;

    #[test]
    fn test_sqlite_to_parquetdb() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        // Create test database
        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                [],
            )?;
            conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])?;
            conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])?;
        }

        // Convert to parquetdb
        let parquetdb = to_parquetdb(
            &db_path,
            OrderMode::Pk,
            NullMode::Marker,
            None,
            None,
            true,
            &TableFilter::new(vec![], vec![]),
        )?;

        // Verify structure
        assert!(parquetdb.join("schema.sql").exists());
        assert!(parquetdb.join("users.parquet").exists());
        assert!(parquetdb.join("csvdb.toml").exists());
        assert!(parquetdb.to_string_lossy().ends_with(".parquetdb"));

        // Verify data can be read back
        let conn = Connection::open_in_memory()?;
        let parquet_path = parquetdb.join("users.parquet").canonicalize()?;
        let path_str = parquet_path.to_string_lossy().replace('\\', "/");
        conn.execute(
            &format!(
                "CREATE TABLE users AS SELECT * FROM read_parquet('{}')",
                path_str
            ),
            [],
        )?;

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        assert_eq!(count, 2);

        Ok(())
    }

    #[test]
    fn test_csvdb_to_parquetdb() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"items\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        fs::write(csvdb_dir.join("items.csv"), "\"id\",\"name\"\n\"1\",\"Apple\"\n\"2\",\"Banana\"\n")?;

        let parquetdb = to_parquetdb(
            &csvdb_dir,
            OrderMode::Pk,
            NullMode::Marker,
            None,
            None,
            true,
            &TableFilter::new(vec![], vec![]),
        )?;

        assert!(parquetdb.join("items.parquet").exists());
        assert!(parquetdb.to_string_lossy().ends_with("test.parquetdb"));

        Ok(())
    }

    #[test]
    fn test_duckdb_to_parquetdb() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.duckdb");

        {
            let conn = Connection::open(&db_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name VARCHAR)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'Hello')", [])?;
        }

        let parquetdb = to_parquetdb(
            &db_path, OrderMode::Pk, NullMode::Marker, None, None, true,
            &TableFilter::new(vec![], vec![]),
        )?;

        assert!(parquetdb.join("t.parquet").exists());
        assert!(parquetdb.join("schema.sql").exists());
        Ok(())
    }

    #[test]
    fn test_parquetdb_to_parquetdb_roundtrip() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("src.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'round')", [])?;
        }

        // sqlite -> parquetdb -> parquetdb (with explicit output to avoid path collision)
        let pdb1 = to_parquetdb(
            &db_path, OrderMode::Pk, NullMode::Marker, None, None, true,
            &TableFilter::new(vec![], vec![]),
        )?;
        let pdb2_path = dir.path().join("copy.parquetdb");
        let pdb2 = to_parquetdb(
            &pdb1, OrderMode::Pk, NullMode::Marker, None, Some(&pdb2_path), true,
            &TableFilter::new(vec![], vec![]),
        )?;

        assert!(pdb2.join("t.parquet").exists());

        // Verify data
        let conn = Connection::open_in_memory()?;
        let abs = pdb2.join("t.parquet").canonicalize()?;
        let p = abs.to_string_lossy().replace('\\', "/");
        conn.execute(&format!("CREATE TABLE t AS SELECT * FROM read_parquet('{}')", p), [])?;
        let val: String = conn.query_row("SELECT val FROM t WHERE id = 1", [], |r| r.get(0))?;
        assert_eq!(val, "round");
        Ok(())
    }

    #[test]
    fn test_to_parquetdb_all_columns_mode() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            // Use a table with PK (load_sqlite in to_parquetdb uses from_sqlite which requires PK)
            conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, category TEXT, price REAL)", [])?;
            conn.execute("INSERT INTO items VALUES (1, 'A', 10.0)", [])?;
            conn.execute("INSERT INTO items VALUES (2, 'B', 20.0)", [])?;
        }

        let parquetdb = to_parquetdb(
            &db_path, OrderMode::AllColumns, NullMode::Marker, None, None, true,
            &TableFilter::new(vec![], vec![]),
        )?;

        assert!(parquetdb.join("items.parquet").exists());
        Ok(())
    }

    #[test]
    fn test_to_parquetdb_custom_order_by() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'Charlie')", [])?;
            conn.execute("INSERT INTO t VALUES (2, 'Alice')", [])?;
        }

        let parquetdb = to_parquetdb(
            &db_path, OrderMode::Pk, NullMode::Marker, Some("name ASC"), None, true,
            &TableFilter::new(vec![], vec![]),
        )?;

        assert!(parquetdb.join("t.parquet").exists());
        Ok(())
    }

    #[test]
    fn test_to_parquetdb_no_force_rejects() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", [])?;
        }

        // First create with force
        to_parquetdb(
            &db_path, OrderMode::Pk, NullMode::Marker, None, None, true,
            &TableFilter::new(vec![], vec![]),
        )?;

        // Second without force should fail
        let result = to_parquetdb(
            &db_path, OrderMode::Pk, NullMode::Marker, None, None, false,
            &TableFilter::new(vec![], vec![]),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("--force"));
        Ok(())
    }

    #[test]
    fn test_to_parquetdb_table_filter() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)", [])?;
            conn.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY)", [])?;
            conn.execute("INSERT INTO t1 VALUES (1)", [])?;
            conn.execute("INSERT INTO t2 VALUES (1)", [])?;
        }

        let parquetdb = to_parquetdb(
            &db_path, OrderMode::Pk, NullMode::Marker, None, None, true,
            &TableFilter::new(vec!["t1".to_string()], vec![]),
        )?;

        assert!(parquetdb.join("t1.parquet").exists());
        assert!(!parquetdb.join("t2.parquet").exists());
        Ok(())
    }

    #[test]
    fn test_to_parquetdb_custom_output() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");
        let output = dir.path().join("custom.parquetdb");

        {
            let conn = SqliteConnection::open(&db_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", [])?;
        }

        let parquetdb = to_parquetdb(
            &db_path, OrderMode::Pk, NullMode::Marker, None, Some(&output), true,
            &TableFilter::new(vec![], vec![]),
        )?;

        assert_eq!(parquetdb, output);
        Ok(())
    }
}
