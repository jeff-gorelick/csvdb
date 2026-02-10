use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

use crate::core::{InputFormat, Schema, Table};
use crate::core::csv::{find_table_file, read_table_csv_auto};
use crate::commands::checksum::normalize_value;
use crate::{OrderMode, NullMode};

/// Load tables from any supported format.
/// Returns (Schema, map of table_name -> Table).
fn load_tables(path: &Path) -> Result<(Schema, BTreeMap<String, Table>)> {
    let format = InputFormat::from_path(path)?;

    match format {
        InputFormat::Csvdb => load_csvdb(path),
        InputFormat::Parquetdb => load_parquetdb(path),
        InputFormat::Sqlite => load_sqlite(path),
        InputFormat::DuckDb => load_duckdb(path),
        InputFormat::Parquet => load_parquet(path),
    }
}

fn load_csvdb(csvdb_dir: &Path) -> Result<(Schema, BTreeMap<String, Table>)> {
    let schema_path = csvdb_dir.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    let mut tables = BTreeMap::new();
    for (table_name, table_schema) in &schema.tables {
        if let Some(csv_path) = find_table_file(csvdb_dir, table_name) {
            let table = read_table_csv_auto(&csv_path, table_schema)?;
            tables.insert(table_name.clone(), table);
        }
    }

    Ok((schema, tables))
}

/// Rebuild pk_values from actual PK columns in the schema.
/// This ensures tables loaded from different formats use the actual PK.
fn rebuild_pk_from_schema(table: &mut Table, schema: &Schema) {
    if let Some(table_schema) = schema.tables.get(&table.name) {
        if !table_schema.pk_columns.is_empty() {
            let pk_indices: Vec<usize> = table_schema.pk_columns.iter()
                .filter_map(|pk| table.columns.iter().position(|c| c == pk))
                .collect();
            table.pk_columns = table_schema.pk_columns.clone();
            for row in &mut table.rows {
                row.pk_values = pk_indices.iter()
                    .map(|&i| row.values[i].clone())
                    .collect();
            }
        }
    }
}

fn load_sqlite(db_path: &Path) -> Result<(Schema, BTreeMap<String, Table>)> {
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    let schema = Schema::from_sqlite_with_order(&conn, OrderMode::AllColumns)?;

    let mut tables = BTreeMap::new();
    for (table_name, table_schema) in &schema.tables {
        let result = Table::from_sqlite_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
        let mut table = result.table;
        // Rebuild PK from actual schema (AllColumns mode uses all columns as pseudo-PK)
        rebuild_pk_from_schema(&mut table, &schema);
        tables.insert(table_name.clone(), table);
    }

    Ok((schema, tables))
}

fn load_duckdb(db_path: &Path) -> Result<(Schema, BTreeMap<String, Table>)> {
    let conn = duckdb::Connection::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    let schema = Schema::from_duckdb_with_order(&conn, OrderMode::AllColumns)?;

    let mut tables = BTreeMap::new();
    for (table_name, table_schema) in &schema.tables {
        let result = Table::from_duckdb_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
        let mut table = result.table;
        // Rebuild PK from actual schema (AllColumns mode uses all columns as pseudo-PK)
        rebuild_pk_from_schema(&mut table, &schema);
        tables.insert(table_name.clone(), table);
    }

    Ok((schema, tables))
}

fn load_parquetdb(parquetdb_dir: &Path) -> Result<(Schema, BTreeMap<String, Table>)> {
    // Load parquetdb into in-memory DuckDB
    let conn = duckdb::Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let schema_path = parquetdb_dir.join("schema.sql");
    let schema = Schema::from_schema_sql(&schema_path)?;

    // Create tables from schema
    // Replace REAL with DOUBLE to avoid 32-bit precision loss in DuckDB
    let schema_sql = std::fs::read_to_string(&schema_path)?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() && stmt.to_uppercase().starts_with("CREATE TABLE") {
            let stmt = stmt.replace(" REAL", " DOUBLE");
            conn.execute(&stmt, [])
                .with_context(|| format!("Failed to execute: {}", stmt))?;
        }
    }

    // Load parquet data in FK dependency order
    for table_name in schema.tables_in_fk_order()? {
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

    let mut tables = BTreeMap::new();
    for (table_name, table_schema) in &schema.tables {
        let result = Table::from_duckdb_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
        let mut table = result.table;
        rebuild_pk_from_schema(&mut table, &schema);
        tables.insert(table_name.clone(), table);
    }

    Ok((schema, tables))
}

fn load_parquet(parquet_path: &Path) -> Result<(Schema, BTreeMap<String, Table>)> {
    // Load single parquet into in-memory DuckDB
    let conn = duckdb::Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let table_name = parquet_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("table");

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

    let mut tables = BTreeMap::new();
    let table_schema = schema.tables.get(table_name)
        .ok_or_else(|| anyhow::anyhow!("Table not found after creation"))?;

    let result = Table::from_duckdb_with_order(&conn, table_schema, OrderMode::AllColumns, NullMode::Marker)?;
    let mut table = result.table;
    rebuild_pk_from_schema(&mut table, &schema);
    tables.insert(table_name.to_string(), table);

    Ok((schema, tables))
}

/// Compare two sources and print differences.
/// Returns true if there are any differences.
pub fn diff(left_path: &Path, right_path: &Path, summary: bool) -> Result<bool> {
    let (left_schema, left_tables) = load_tables(left_path)
        .with_context(|| format!("Failed to load left: {}", left_path.display()))?;
    let (right_schema, right_tables) = load_tables(right_path)
        .with_context(|| format!("Failed to load right: {}", right_path.display()))?;

    println!(
        "Comparing {} \u{2194} {}",
        left_path.display(),
        right_path.display()
    );
    println!();

    let mut has_differences = false;

    // Collect all table names from both sides
    let mut all_tables: Vec<String> = left_schema
        .tables
        .keys()
        .chain(right_schema.tables.keys())
        .cloned()
        .collect();
    all_tables.sort();
    all_tables.dedup();

    for table_name in &all_tables {
        let in_left = left_tables.contains_key(table_name);
        let in_right = right_tables.contains_key(table_name);

        if in_left && !in_right {
            println!("{}: removed table", table_name);
            has_differences = true;
            continue;
        }

        if !in_left && in_right {
            println!("{}: added table", table_name);
            has_differences = true;
            continue;
        }

        // Both sides have the table
        let left_table = &left_tables[table_name];
        let right_table = &right_tables[table_name];

        let left_by_pk = left_table.rows_by_pk();
        let right_by_pk = right_table.rows_by_pk();

        let mut added = Vec::new();
        let mut deleted = Vec::new();
        let mut modified = Vec::new();

        // Find deleted and modified rows
        for (pk, left_row) in &left_by_pk {
            match right_by_pk.get(pk) {
                None => deleted.push(pk.clone()),
                Some(right_row) => {
                    if left_row.content_hash() != right_row.content_hash() {
                        // Compare with normalized values to ignore float formatting differences
                        let values_differ = left_row.values.iter()
                            .zip(right_row.values.iter())
                            .any(|(lv, rv)| normalize_value(lv) != normalize_value(rv));
                        if values_differ {
                            modified.push(pk.clone());
                        }
                    }
                }
            }
        }

        // Find added rows
        for pk in right_by_pk.keys() {
            if !left_by_pk.contains_key(pk) {
                added.push(pk.clone());
            }
        }

        if added.is_empty() && deleted.is_empty() && modified.is_empty() {
            println!(
                "{}: identical ({} rows)",
                table_name,
                left_table.rows.len()
            );
            continue;
        }

        has_differences = true;

        // Summary line
        let mut parts = Vec::new();
        if !added.is_empty() {
            parts.push(format!("{} added", added.len()));
        }
        if !deleted.is_empty() {
            parts.push(format!("{} deleted", deleted.len()));
        }
        if !modified.is_empty() {
            parts.push(format!("{} modified", modified.len()));
        }
        println!("{}: {}", table_name, parts.join(", "));

        if summary {
            continue;
        }

        // Detailed output
        let pk_display = |pk_key: &str| -> String {
            let pk_parts: Vec<&str> = pk_key.split('\x00').collect();
            let pk_col_names = &left_table.pk_columns;
            pk_parts
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let col = pk_col_names
                        .get(i)
                        .map(|s| s.as_str())
                        .unwrap_or("?");
                    format!("{}={}", col, v)
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        // Show added rows
        for pk in &added {
            let row = right_by_pk[pk];
            let values: Vec<String> = row
                .values
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    right_table
                        .pk_columns
                        .iter()
                        .all(|pk_col| {
                            right_table
                                .columns
                                .get(*i)
                                .map(|c| c != pk_col)
                                .unwrap_or(true)
                        })
                })
                .map(|(_, v)| v.clone())
                .collect();
            println!("  + ({}) {}", pk_display(pk), values.join(", "));
        }

        // Show deleted rows
        for pk in &deleted {
            let row = left_by_pk[pk];
            let values: Vec<String> = row
                .values
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    left_table
                        .pk_columns
                        .iter()
                        .all(|pk_col| {
                            left_table
                                .columns
                                .get(*i)
                                .map(|c| c != pk_col)
                                .unwrap_or(true)
                        })
                })
                .map(|(_, v)| v.clone())
                .collect();
            println!("  - ({}) {}", pk_display(pk), values.join(", "));
        }

        // Show modified rows (column-level diff)
        for pk in &modified {
            let left_row = left_by_pk[pk];
            let right_row = right_by_pk[pk];

            for (i, (lv, rv)) in left_row
                .values
                .iter()
                .zip(right_row.values.iter())
                .enumerate()
            {
                if normalize_value(lv) != normalize_value(rv) {
                    let col_name = left_table
                        .columns
                        .get(i)
                        .map(|s| s.as_str())
                        .unwrap_or("?");
                    println!(
                        "  ~ ({}) {}: \"{}\" \u{2192} \"{}\"",
                        pk_display(pk),
                        col_name,
                        lv,
                        rv
                    );
                }
            }
        }

        println!();
    }

    Ok(has_differences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Helper: create a minimal csvdb dir with one table.
    fn make_csvdb(base: &Path, name: &str, schema_sql: &str, csvs: &[(&str, &str)]) -> PathBuf {
        let csvdb_dir = base.join(name);
        fs::create_dir_all(&csvdb_dir).unwrap();
        fs::write(csvdb_dir.join("schema.sql"), schema_sql).unwrap();
        for (table_name, content) in csvs {
            fs::write(csvdb_dir.join(format!("{}.csv", table_name)), content).unwrap();
        }
        csvdb_dir
    }

    const SCHEMA: &str =
        "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n";

    #[test]
    fn test_diff_identical() -> Result<()> {
        let dir = tempdir()?;
        let csv = "id,name\n1,Alice\n2,Bob\n";
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", csv)]);
        let right = make_csvdb(dir.path(), "b.csvdb", SCHEMA, &[("t", csv)]);

        let has_diff = diff(&left, &right, false)?;
        assert!(!has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_added_rows() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", "id,name\n1,Alice\n")]);
        let right = make_csvdb(dir.path(), "b.csvdb", SCHEMA, &[("t", "id,name\n1,Alice\n2,Bob\n")]);

        let has_diff = diff(&left, &right, false)?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_deleted_rows() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", "id,name\n1,Alice\n2,Bob\n")]);
        let right = make_csvdb(dir.path(), "b.csvdb", SCHEMA, &[("t", "id,name\n1,Alice\n")]);

        let has_diff = diff(&left, &right, false)?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_modified_rows() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", "id,name\n1,Alice\n")]);
        let right = make_csvdb(dir.path(), "b.csvdb", SCHEMA, &[("t", "id,name\n1,Alicia\n")]);

        let has_diff = diff(&left, &right, false)?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_added_table() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", "id,name\n1,Alice\n")]);

        let schema2 = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                        CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let right = make_csvdb(dir.path(), "b.csvdb", schema2, &[
            ("t", "id,name\n1,Alice\n"),
            ("t2", "id,val\n1,x\n"),
        ]);

        let has_diff = diff(&left, &right, false)?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_ignores_float_formatting() -> Result<()> {
        let dir = tempdir()?;
        let schema = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"price\" REAL\n);\n";
        // Same numeric values, different string representations
        let left = make_csvdb(dir.path(), "a.csvdb", schema, &[("t", "id,price\n1,32\n2,24.5\n3,149\n")]);
        let right = make_csvdb(dir.path(), "b.csvdb", schema, &[("t", "id,price\n1,32.00\n2,24.50\n3,149.00\n")]);

        let has_diff = diff(&left, &right, false)?;
        assert!(!has_diff, "float formatting differences should be ignored");
        Ok(())
    }

    #[test]
    fn test_diff_detects_real_change_with_float_noise() -> Result<()> {
        let dir = tempdir()?;
        let schema = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"price\" REAL,\n    \"category_id\" INTEGER\n);\n";
        // price differs only in formatting, but category_id has a real change
        let left = make_csvdb(dir.path(), "a.csvdb", schema, &[("t", "id,price,category_id\n1,32,3\n")]);
        let right = make_csvdb(dir.path(), "b.csvdb", schema, &[("t", "id,price,category_id\n1,32.00,32\n")]);

        let has_diff = diff(&left, &right, false)?;
        assert!(has_diff, "real data change should be detected despite float noise");
        Ok(())
    }

    #[test]
    fn test_diff_summary_mode() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", "id,name\n1,Alice\n")]);
        let right = make_csvdb(dir.path(), "b.csvdb", SCHEMA, &[("t", "id,name\n1,Alicia\n2,Bob\n")]);

        // summary=true should not panic and should still detect differences
        let has_diff = diff(&left, &right, true)?;
        assert!(has_diff);
        Ok(())
    }
}
