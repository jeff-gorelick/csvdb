use anyhow::{bail, Context, Result};
use arrow::record_batch::RecordBatch;
use duckdb::Connection;
use std::path::Path;

use crate::core::input::InputFormat;
use crate::commands::sql::load_data;
use crate::TableFilter;

/// Read all tables from any supported format as Arrow RecordBatches.
///
/// Returns a list of (table_name, batches) pairs, sorted by table name.
/// Use the `filter` parameter to include/exclude specific tables.
pub fn read_tables_arrow(
    path: &Path,
    filter: &TableFilter,
) -> Result<Vec<(String, Vec<RecordBatch>)>> {
    let input_format = InputFormat::from_path(path)?;

    let conn = Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    load_data(&conn, path, input_format)?;

    let mut table_names = get_table_names(&conn)?;
    table_names.retain(|t| filter.matches(t));
    table_names.sort();

    let mut result = Vec::new();
    for name in table_names {
        let batches = query_table_arrow(&conn, &name)?;
        result.push((name, batches));
    }

    Ok(result)
}

/// Read a single table from any supported format as Arrow RecordBatches.
pub fn read_table_arrow(
    path: &Path,
    table_name: &str,
) -> Result<Vec<RecordBatch>> {
    let input_format = InputFormat::from_path(path)?;

    let conn = Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    load_data(&conn, path, input_format)?;

    // Verify the table exists
    let table_names = get_table_names(&conn)?;
    if !table_names.iter().any(|t| t == table_name) {
        bail!("Table '{}' not found. Available tables: {}", table_name, table_names.join(", "));
    }

    query_table_arrow(&conn, table_name)
}

fn get_table_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SHOW TABLES")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names)
}

fn query_table_arrow(conn: &Connection, table_name: &str) -> Result<Vec<RecordBatch>> {
    let query = format!("SELECT * FROM \"{}\"", table_name);
    let mut stmt = conn.prepare(&query)
        .with_context(|| format!("Failed to prepare query for table: {}", table_name))?;
    let arrow = stmt.query_arrow([])
        .with_context(|| format!("Failed to query table: {}", table_name))?;
    let batches: Vec<RecordBatch> = arrow.collect();
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_test_csvdb(dir: &std::path::Path) -> std::path::PathBuf {
        let csvdb_dir = dir.join("test.csvdb");
        fs::create_dir(&csvdb_dir).unwrap();

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"users\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT NOT NULL,\n    \"score\" INTEGER\n);\n",
        ).unwrap();
        fs::write(
            csvdb_dir.join("users.csv"),
            "id,name,score\n1,Alice,95\n2,Bob,87\n3,Charlie,92\n",
        ).unwrap();

        csvdb_dir
    }

    fn make_multi_table_csvdb(dir: &std::path::Path) -> std::path::PathBuf {
        let csvdb_dir = dir.join("multi.csvdb");
        fs::create_dir(&csvdb_dir).unwrap();

        fs::write(
            csvdb_dir.join("schema.sql"),
            concat!(
                "CREATE TABLE \"users\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT NOT NULL\n);\n",
                "CREATE TABLE \"orders\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"user_id\" INTEGER,\n    \"total\" INTEGER\n);\n",
            ),
        ).unwrap();
        fs::write(
            csvdb_dir.join("users.csv"),
            "id,name\n1,Alice\n2,Bob\n",
        ).unwrap();
        fs::write(
            csvdb_dir.join("orders.csv"),
            "id,user_id,total\n1,1,100\n2,2,200\n",
        ).unwrap();

        csvdb_dir
    }

    #[test]
    fn test_read_table_arrow() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = make_test_csvdb(dir.path());

        let batches = read_table_arrow(&csvdb_dir, "users")?;

        assert!(!batches.is_empty());
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);

        let schema = batches[0].schema();
        let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(field_names, vec!["id", "name", "score"]);

        Ok(())
    }

    #[test]
    fn test_read_table_arrow_not_found() {
        let dir = tempdir().unwrap();
        let csvdb_dir = make_test_csvdb(dir.path());

        let result = read_table_arrow(&csvdb_dir, "nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_read_tables_arrow() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = make_multi_table_csvdb(dir.path());

        let filter = TableFilter::new(vec![], vec![]);
        let tables = read_tables_arrow(&csvdb_dir, &filter)?;

        assert_eq!(tables.len(), 2);
        // Should be sorted alphabetically
        assert_eq!(tables[0].0, "orders");
        assert_eq!(tables[1].0, "users");

        // Check row counts
        let order_rows: usize = tables[0].1.iter().map(|b| b.num_rows()).sum();
        let user_rows: usize = tables[1].1.iter().map(|b| b.num_rows()).sum();
        assert_eq!(order_rows, 2);
        assert_eq!(user_rows, 2);

        Ok(())
    }

    #[test]
    fn test_read_tables_arrow_with_filter() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = make_multi_table_csvdb(dir.path());

        let filter = TableFilter::new(vec!["users".to_string()], vec![]);
        let tables = read_tables_arrow(&csvdb_dir, &filter)?;

        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].0, "users");

        Ok(())
    }

    #[test]
    fn test_read_tables_arrow_with_exclude() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = make_multi_table_csvdb(dir.path());

        let filter = TableFilter::new(vec![], vec!["orders".to_string()]);
        let tables = read_tables_arrow(&csvdb_dir, &filter)?;

        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].0, "users");

        Ok(())
    }

    #[test]
    fn test_read_tables_arrow_from_sqlite() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                [],
            )?;
            conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])?;
            conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])?;
        }

        let filter = TableFilter::new(vec![], vec![]);
        let tables = read_tables_arrow(&db_path, &filter)?;

        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].0, "users");

        let total_rows: usize = tables[0].1.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        Ok(())
    }
}
