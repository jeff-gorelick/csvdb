use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

use crate::commands::checksum::normalize_value;
use crate::core::csv::{find_table_file, read_table_csv_auto};
use crate::core::{InputFormat, Schema, Table};
use crate::{NullMode, OrderMode, TableFilter};

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiffResult {
    pub left: String,
    pub right: String,
    pub has_differences: bool,
    pub tables: Vec<TableDiff>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TableDiff {
    pub name: String,
    pub status: TableStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<RowCounts>,
    pub changes: Vec<RowChange>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TableStatus {
    Identical,
    Modified,
    Added,
    Removed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RowCounts {
    pub total_left: usize,
    pub total_right: usize,
    pub added: usize,
    pub deleted: usize,
    pub modified: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RowChange {
    pub kind: ChangeKind,
    pub pk: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<ColumnChange>>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ColumnChange {
    pub column: String,
    pub left: String,
    pub right: String,
}

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
            let pk_indices: Vec<usize> = table_schema
                .pk_columns
                .iter()
                .filter_map(|pk| table.columns.iter().position(|c| c == pk))
                .collect();
            table.pk_columns = table_schema.pk_columns.clone();
            for row in &mut table.rows {
                row.pk_values = pk_indices.iter().map(|&i| row.values[i].clone()).collect();
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
        let result = Table::from_sqlite_with_order(
            &conn,
            table_schema,
            OrderMode::AllColumns,
            NullMode::Marker,
        )?;
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
        let result = Table::from_duckdb_with_order(
            &conn,
            table_schema,
            OrderMode::AllColumns,
            NullMode::Marker,
        )?;
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
                .with_context(|| format!("Failed to execute: {stmt}"))?;
        }
    }

    // Load parquet data in FK dependency order
    for table_name in schema.tables_in_fk_order()? {
        let parquet_path = parquetdb_dir.join(format!("{table_name}.parquet"));
        if parquet_path.exists() {
            let abs_path = parquet_path.canonicalize()?;
            let path_str = abs_path.to_string_lossy().replace('\\', "/");
            let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

            conn.execute(
                &format!("INSERT INTO \"{table_name}\" SELECT * FROM read_parquet('{path_str}')"),
                [],
            )?;
        }
    }

    let mut tables = BTreeMap::new();
    for (table_name, table_schema) in &schema.tables {
        let result = Table::from_duckdb_with_order(
            &conn,
            table_schema,
            OrderMode::AllColumns,
            NullMode::Marker,
        )?;
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
        &format!("CREATE TABLE \"{table_name}\" AS SELECT * FROM read_parquet('{path_str}')"),
        [],
    )?;

    // Get schema from DuckDB
    let schema = Schema::from_duckdb_with_order(&conn, OrderMode::AllColumns)?;

    let mut tables = BTreeMap::new();
    let table_schema = schema
        .tables
        .get(table_name)
        .ok_or_else(|| anyhow::anyhow!("Table not found after creation"))?;

    let result = Table::from_duckdb_with_order(
        &conn,
        table_schema,
        OrderMode::AllColumns,
        NullMode::Marker,
    )?;
    let mut table = result.table;
    rebuild_pk_from_schema(&mut table, &schema);
    tables.insert(table_name.to_string(), table);

    Ok((schema, tables))
}

/// Parse a PK key string into a BTreeMap of column_name -> value.
fn pk_to_map(pk_key: &str, pk_col_names: &[String]) -> BTreeMap<String, String> {
    let pk_parts: Vec<&str> = pk_key.split('\x00').collect();
    pk_parts
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let col = pk_col_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| "?".to_string());
            (col, v.to_string())
        })
        .collect()
}

/// Collect detailed diff data without printing anything.
pub fn diff_detail(
    left_path: &Path,
    right_path: &Path,
    summary: bool,
    filter: &TableFilter,
) -> Result<DiffResult> {
    let (left_schema, left_tables) = load_tables(left_path)
        .with_context(|| format!("Failed to load left: {}", left_path.display()))?;
    let (right_schema, right_tables) = load_tables(right_path)
        .with_context(|| format!("Failed to load right: {}", right_path.display()))?;

    let mut has_differences = false;
    let mut table_diffs = Vec::new();

    // Collect all table names from both sides
    let mut all_tables: Vec<String> = left_schema
        .tables
        .keys()
        .chain(right_schema.tables.keys())
        .cloned()
        .collect();
    all_tables.sort();
    all_tables.dedup();
    all_tables.retain(|t| filter.matches(t));

    for table_name in &all_tables {
        let in_left = left_tables.contains_key(table_name);
        let in_right = right_tables.contains_key(table_name);

        if in_left && !in_right {
            has_differences = true;
            table_diffs.push(TableDiff {
                name: table_name.clone(),
                status: TableStatus::Removed,
                rows: None,
                changes: vec![],
            });
            continue;
        }

        if !in_left && in_right {
            has_differences = true;
            table_diffs.push(TableDiff {
                name: table_name.clone(),
                status: TableStatus::Added,
                rows: None,
                changes: vec![],
            });
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
                        let values_differ = left_row
                            .values
                            .iter()
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
            table_diffs.push(TableDiff {
                name: table_name.clone(),
                status: TableStatus::Identical,
                rows: Some(RowCounts {
                    total_left: left_table.rows.len(),
                    total_right: right_table.rows.len(),
                    added: 0,
                    deleted: 0,
                    modified: 0,
                }),
                changes: vec![],
            });
            continue;
        }

        has_differences = true;

        let mut changes = Vec::new();

        if !summary {
            // Collect added row changes
            for pk in &added {
                changes.push(RowChange {
                    kind: ChangeKind::Added,
                    pk: pk_to_map(pk, &right_table.pk_columns),
                    columns: None,
                });
            }

            // Collect deleted row changes
            for pk in &deleted {
                changes.push(RowChange {
                    kind: ChangeKind::Deleted,
                    pk: pk_to_map(pk, &left_table.pk_columns),
                    columns: None,
                });
            }

            // Collect modified row changes with column-level diffs
            for pk in &modified {
                let left_row = left_by_pk[pk];
                let right_row = right_by_pk[pk];

                let col_changes: Vec<ColumnChange> = left_row
                    .values
                    .iter()
                    .zip(right_row.values.iter())
                    .enumerate()
                    .filter(|(_, (lv, rv))| normalize_value(lv) != normalize_value(rv))
                    .map(|(i, (lv, rv))| ColumnChange {
                        column: left_table
                            .columns
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| "?".to_string()),
                        left: lv.clone(),
                        right: rv.clone(),
                    })
                    .collect();

                changes.push(RowChange {
                    kind: ChangeKind::Modified,
                    pk: pk_to_map(pk, &left_table.pk_columns),
                    columns: Some(col_changes),
                });
            }
        }

        table_diffs.push(TableDiff {
            name: table_name.clone(),
            status: TableStatus::Modified,
            rows: Some(RowCounts {
                total_left: left_table.rows.len(),
                total_right: right_table.rows.len(),
                added: added.len(),
                deleted: deleted.len(),
                modified: modified.len(),
            }),
            changes,
        });
    }

    Ok(DiffResult {
        left: left_path.display().to_string(),
        right: right_path.display().to_string(),
        has_differences,
        tables: table_diffs,
    })
}

/// Print a DiffResult in human-readable text format.
fn print_text_diff(result: &DiffResult, summary: bool) {
    println!("Comparing {} \u{2194} {}", result.left, result.right);
    println!();

    for table in &result.tables {
        match table.status {
            TableStatus::Removed => {
                println!("{}: removed table", table.name);
            }
            TableStatus::Added => {
                println!("{}: added table", table.name);
            }
            TableStatus::Identical => {
                if let Some(rows) = &table.rows {
                    println!("{}: identical ({} rows)", table.name, rows.total_left);
                }
            }
            TableStatus::Modified => {
                if let Some(rows) = &table.rows {
                    let mut parts = Vec::new();
                    if rows.added > 0 {
                        parts.push(format!("{} added", rows.added));
                    }
                    if rows.deleted > 0 {
                        parts.push(format!("{} deleted", rows.deleted));
                    }
                    if rows.modified > 0 {
                        parts.push(format!("{} modified", rows.modified));
                    }
                    println!("{}: {}", table.name, parts.join(", "));
                }

                if !summary {
                    for change in &table.changes {
                        let pk_display: String = change
                            .pk
                            .iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ");

                        match change.kind {
                            ChangeKind::Added => {
                                println!("  + ({})", pk_display);
                            }
                            ChangeKind::Deleted => {
                                println!("  - ({})", pk_display);
                            }
                            ChangeKind::Modified => {
                                if let Some(cols) = &change.columns {
                                    for col in cols {
                                        println!(
                                            "  ~ ({}) {}: \"{}\" \u{2192} \"{}\"",
                                            pk_display, col.column, col.left, col.right
                                        );
                                    }
                                }
                            }
                        }
                    }
                    println!();
                }
            }
        }
    }
}

/// Compare two sources and print differences.
/// Returns true if there are any differences.
pub fn diff(
    left_path: &Path,
    right_path: &Path,
    summary: bool,
    filter: &TableFilter,
) -> Result<bool> {
    let result = diff_detail(left_path, right_path, summary, filter)?;
    print_text_diff(&result, summary);
    Ok(result.has_differences)
}

/// Compare two sources and return the result as a JSON string.
pub fn diff_json(
    left_path: &Path,
    right_path: &Path,
    summary: bool,
    filter: &TableFilter,
) -> Result<String> {
    let result = diff_detail(left_path, right_path, summary, filter)?;
    serde_json::to_string_pretty(&result).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TableFilter;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Helper: create a minimal csvdb dir with one table.
    fn make_csvdb(base: &Path, name: &str, schema_sql: &str, csvs: &[(&str, &str)]) -> PathBuf {
        let csvdb_dir = base.join(name);
        fs::create_dir_all(&csvdb_dir).unwrap();
        fs::write(csvdb_dir.join("schema.sql"), schema_sql).unwrap();
        for (table_name, content) in csvs {
            fs::write(csvdb_dir.join(format!("{table_name}.csv")), content).unwrap();
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

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(!has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_added_rows() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n2,Bob\n")],
        );

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_deleted_rows() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n2,Bob\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_modified_rows() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alicia\n")],
        );

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_added_table() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );

        let schema2 = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                        CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            schema2,
            &[("t", "id,name\n1,Alice\n"), ("t2", "id,val\n1,x\n")],
        );

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_ignores_float_formatting() -> Result<()> {
        let dir = tempdir()?;
        let schema =
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"price\" REAL\n);\n";
        // Same numeric values, different string representations
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            schema,
            &[("t", "id,price\n1,32\n2,24.5\n3,149\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            schema,
            &[("t", "id,price\n1,32.00\n2,24.50\n3,149.00\n")],
        );

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(!has_diff, "float formatting differences should be ignored");
        Ok(())
    }

    #[test]
    fn test_diff_detects_real_change_with_float_noise() -> Result<()> {
        let dir = tempdir()?;
        let schema = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"price\" REAL,\n    \"category_id\" INTEGER\n);\n";
        // price differs only in formatting, but category_id has a real change
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            schema,
            &[("t", "id,price,category_id\n1,32,3\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            schema,
            &[("t", "id,price,category_id\n1,32.00,32\n")],
        );

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(
            has_diff,
            "real data change should be detected despite float noise"
        );
        Ok(())
    }

    #[test]
    fn test_diff_summary_mode() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alicia\n2,Bob\n")],
        );

        // summary=true should not panic and should still detect differences
        let has_diff = diff(&left, &right, true, &TableFilter::new(vec![], vec![]))?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_removed_table() -> Result<()> {
        let dir = tempdir()?;
        let schema2 = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                        CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            schema2,
            &[("t", "id,name\n1,Alice\n"), ("t2", "id,val\n1,x\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );

        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_table_filter() -> Result<()> {
        let dir = tempdir()?;
        let schema2 = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                        CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            schema2,
            &[("t", "id,name\n1,Alice\n"), ("t2", "id,val\n1,x\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            schema2,
            &[("t", "id,name\n1,Alice\n"), ("t2", "id,val\n1,changed\n")],
        );

        // Filter to only t (which is identical) — no diff expected
        let has_diff = diff(
            &left,
            &right,
            false,
            &TableFilter::new(vec!["t".to_string()], vec![]),
        )?;
        assert!(!has_diff);

        // Without filter — t2 differs
        let has_diff = diff(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_sqlite_sources() -> Result<()> {
        let dir = tempdir()?;
        let db1_path = dir.path().join("db1.sqlite");
        let db2_path = dir.path().join("db2.sqlite");

        {
            let conn = rusqlite::Connection::open(&db1_path).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1, 'hello')", [])
                .unwrap();
        }
        {
            let conn = rusqlite::Connection::open(&db2_path).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1, 'world')", [])
                .unwrap();
        }

        let has_diff = diff(
            &db1_path,
            &db2_path,
            false,
            &TableFilter::new(vec![], vec![]),
        )?;
        assert!(has_diff);

        // Same data should show no diff
        let has_diff = diff(
            &db1_path,
            &db1_path,
            false,
            &TableFilter::new(vec![], vec![]),
        )?;
        assert!(!has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_duckdb_sources() -> Result<()> {
        let dir = tempdir()?;
        let db1_path = dir.path().join("db1.duckdb");
        let db2_path = dir.path().join("db2.duckdb");

        {
            let conn = duckdb::Connection::open(&db1_path).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val VARCHAR)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1, 'hello')", [])
                .unwrap();
        }
        {
            let conn = duckdb::Connection::open(&db2_path).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val VARCHAR)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1, 'world')", [])
                .unwrap();
        }

        let has_diff = diff(
            &db1_path,
            &db2_path,
            false,
            &TableFilter::new(vec![], vec![]),
        )?;
        assert!(has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_parquetdb_sources() -> Result<()> {
        let dir = tempdir()?;
        let db1_path = dir.path().join("db1.sqlite");
        let db2_path = dir.path().join("db2.sqlite");

        {
            let conn = rusqlite::Connection::open(&db1_path).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1, 'same')", [])
                .unwrap();
        }
        {
            let conn = rusqlite::Connection::open(&db2_path).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1, 'same')", [])
                .unwrap();
        }

        let no_filter = TableFilter::new(vec![], vec![]);
        let pdb1 = crate::commands::to_parquetdb::to_parquetdb(
            &db1_path,
            OrderMode::Pk,
            NullMode::Marker,
            None,
            None,
            true,
            &no_filter,
        )?;
        let pdb2 = crate::commands::to_parquetdb::to_parquetdb(
            &db2_path,
            OrderMode::Pk,
            NullMode::Marker,
            None,
            None,
            true,
            &no_filter,
        )?;

        let has_diff = diff(&pdb1, &pdb2, false, &no_filter)?;
        assert!(!has_diff);
        Ok(())
    }

    #[test]
    fn test_diff_detail_identical() -> Result<()> {
        let dir = tempdir()?;
        let csv = "id,name\n1,Alice\n2,Bob\n";
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", csv)]);
        let right = make_csvdb(dir.path(), "b.csvdb", SCHEMA, &[("t", csv)]);

        let result = diff_detail(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(!result.has_differences);
        assert_eq!(result.tables.len(), 1);
        assert!(matches!(result.tables[0].status, TableStatus::Identical));
        assert_eq!(result.tables[0].rows.as_ref().unwrap().total_left, 2);
        Ok(())
    }

    #[test]
    fn test_diff_detail_modified() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n2,Bob\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alicia\n3,Charlie\n")],
        );

        let result = diff_detail(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(result.has_differences);
        assert_eq!(result.tables.len(), 1);
        assert!(matches!(result.tables[0].status, TableStatus::Modified));
        let rows = result.tables[0].rows.as_ref().unwrap();
        assert_eq!(rows.added, 1);
        assert_eq!(rows.deleted, 1);
        assert_eq!(rows.modified, 1);
        assert_eq!(result.tables[0].changes.len(), 3);
        Ok(())
    }

    #[test]
    fn test_diff_detail_added_table() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );

        let schema2 = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                        CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            schema2,
            &[("t", "id,name\n1,Alice\n"), ("t2", "id,val\n1,x\n")],
        );

        let result = diff_detail(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(result.has_differences);
        let added_table = result.tables.iter().find(|t| t.name == "t2").unwrap();
        assert!(matches!(added_table.status, TableStatus::Added));
        assert!(added_table.rows.is_none());
        Ok(())
    }

    #[test]
    fn test_diff_detail_removed_table() -> Result<()> {
        let dir = tempdir()?;
        let schema2 = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                        CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            schema2,
            &[("t", "id,name\n1,Alice\n"), ("t2", "id,val\n1,x\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );

        let result = diff_detail(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        assert!(result.has_differences);
        let removed_table = result.tables.iter().find(|t| t.name == "t2").unwrap();
        assert!(matches!(removed_table.status, TableStatus::Removed));
        assert!(removed_table.rows.is_none());
        Ok(())
    }

    #[test]
    fn test_diff_detail_summary_no_changes() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alicia\n2,Bob\n")],
        );

        let result = diff_detail(&left, &right, true, &TableFilter::new(vec![], vec![]))?;
        assert!(result.has_differences);
        // In summary mode, changes should be empty
        assert!(result.tables[0].changes.is_empty());
        // But row counts should still be present
        let rows = result.tables[0].rows.as_ref().unwrap();
        assert_eq!(rows.added, 1);
        assert_eq!(rows.modified, 1);
        Ok(())
    }

    #[test]
    fn test_diff_json_valid() -> Result<()> {
        let dir = tempdir()?;
        let csv = "id,name\n1,Alice\n2,Bob\n";
        let left = make_csvdb(dir.path(), "a.csvdb", SCHEMA, &[("t", csv)]);
        let right = make_csvdb(dir.path(), "b.csvdb", SCHEMA, &[("t", csv)]);

        let json = diff_json(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        let parsed: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(parsed["has_differences"], false);
        assert!(parsed["tables"].is_array());
        assert_eq!(parsed["tables"][0]["status"], "identical");
        Ok(())
    }

    #[test]
    fn test_diff_json_with_changes() -> Result<()> {
        let dir = tempdir()?;
        let left = make_csvdb(
            dir.path(),
            "a.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alice\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "b.csvdb",
            SCHEMA,
            &[("t", "id,name\n1,Alicia\n2,Bob\n")],
        );

        let json = diff_json(&left, &right, false, &TableFilter::new(vec![], vec![]))?;
        let parsed: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(parsed["has_differences"], true);
        assert_eq!(parsed["tables"][0]["status"], "modified");
        assert_eq!(parsed["tables"][0]["rows"]["added"], 1);
        assert_eq!(parsed["tables"][0]["rows"]["modified"], 1);
        let changes = parsed["tables"][0]["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 2);
        Ok(())
    }

    #[test]
    fn test_diff_cross_format() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1, 'cross')", [])
                .unwrap();
        }

        // sqlite -> csvdb
        let no_filter = TableFilter::new(vec![], vec![]);
        let csvdb = crate::commands::to_csv::to_csv(
            &db_path,
            OrderMode::Pk,
            NullMode::Marker,
            false,
            None,
            false,
            None,
            true,
            &no_filter,
        )?;

        // Diff sqlite vs csvdb should show no differences (same data)
        let has_diff = diff(&db_path, &csvdb, false, &no_filter)?;
        assert!(!has_diff);
        Ok(())
    }
}
