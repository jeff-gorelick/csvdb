use anyhow::{Context, Result};
use csv::{ReaderBuilder, WriterBuilder};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use super::table::{Row, Table, natural_cmp};
use super::schema::TableSchema;

/// Write a table to CSV with deterministic formatting.
/// - Header row with column names
/// - Data rows sorted by primary key
/// - Consistent quoting (quote all fields)
pub fn write_table_csv(table: &Table, path: &Path) -> Result<()> {
    write_table_csv_sorted(table, path, false)
}

/// Write a table to CSV with optional natural sort.
pub fn write_table_csv_sorted(table: &Table, path: &Path, natural_sort: bool) -> Result<()> {
    let file = File::create(path)
        .with_context(|| format!("Failed to create CSV: {}", path.display()))?;

    let mut writer = WriterBuilder::new()
        .quote_style(csv::QuoteStyle::Always)
        .from_writer(file);

    // Write header
    writer.write_record(&table.columns)?;

    // Sort rows by primary key for determinism
    let mut sorted_rows = table.rows.clone();
    if natural_sort {
        sorted_rows.sort_by(|a, b| {
            for (av, bv) in a.pk_values.iter().zip(b.pk_values.iter()) {
                let ord = natural_cmp(av, bv);
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });
    } else {
        sorted_rows.sort_by(|a, b| a.pk_values.cmp(&b.pk_values));
    }

    // Write data rows
    for row in &sorted_rows {
        writer.write_record(&row.values)?;
    }

    writer.flush()?;
    Ok(())
}

/// Write a table to a gzip-compressed CSV file (.csv.gz).
pub fn write_table_csv_gz(table: &Table, path: &Path, natural_sort: bool) -> Result<()> {
    let file = File::create(path)
        .with_context(|| format!("Failed to create compressed CSV: {}", path.display()))?;

    let encoder = GzEncoder::new(BufWriter::new(file), Compression::default());

    let mut writer = WriterBuilder::new()
        .quote_style(csv::QuoteStyle::Always)
        .from_writer(encoder);

    // Write header
    writer.write_record(&table.columns)?;

    // Sort rows by primary key for determinism
    let mut sorted_rows = table.rows.clone();
    if natural_sort {
        sorted_rows.sort_by(|a, b| {
            for (av, bv) in a.pk_values.iter().zip(b.pk_values.iter()) {
                let ord = natural_cmp(av, bv);
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });
    } else {
        sorted_rows.sort_by(|a, b| a.pk_values.cmp(&b.pk_values));
    }

    // Write data rows
    for row in &sorted_rows {
        writer.write_record(&row.values)?;
    }

    writer.flush()?;
    // Finish the gzip stream
    writer.into_inner()
        .map_err(|e| anyhow::anyhow!("CSV flush error: {}", e))?
        .finish()?;

    Ok(())
}

/// Find the data file for a table in a csvdb directory.
/// Checks for `.csv` first, then `.csv.gz`.
pub fn find_table_file(dir: &Path, table_name: &str) -> Option<PathBuf> {
    let csv_path = dir.join(format!("{}.csv", table_name));
    if csv_path.exists() {
        return Some(csv_path);
    }
    let gz_path = dir.join(format!("{}.csv.gz", table_name));
    if gz_path.exists() {
        return Some(gz_path);
    }
    None
}

/// Read a table from a CSV file, automatically detecting gzip compression.
pub fn read_table_csv_auto(path: &Path, schema: &TableSchema) -> Result<Table> {
    if path.extension().map(|e| e == "gz").unwrap_or(false) {
        read_table_csv_gz(path, schema)
    } else {
        read_table_csv(path, schema)
    }
}

/// Read a table from a gzip-compressed CSV file.
fn read_table_csv_gz(path: &Path, schema: &TableSchema) -> Result<Table> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open compressed CSV: {}", path.display()))?;

    let decoder = GzDecoder::new(BufReader::new(file));

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(decoder);

    let headers = reader.headers()?.clone();
    let columns: Vec<String> = headers.iter().map(|s| s.to_string()).collect();

    // Find primary key column indices
    let pk_indices: Vec<usize> = schema.pk_columns
        .iter()
        .filter_map(|pk| columns.iter().position(|c| c == pk))
        .collect();

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result?;
        let values: Vec<String> = record.iter().map(|s| s.to_string()).collect();

        let pk_values: Vec<String> = pk_indices
            .iter()
            .map(|&i| values.get(i).cloned().unwrap_or_default())
            .collect();

        rows.push(Row { pk_values, values });
    }

    Ok(Table {
        name: schema.name.clone(),
        columns,
        pk_columns: schema.pk_columns.clone(),
        rows,
    })
}

/// Read a table from CSV file.
pub fn read_table_csv(path: &Path, schema: &TableSchema) -> Result<Table> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open CSV: {}", path.display()))?;

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(file);

    let headers = reader.headers()?.clone();
    let columns: Vec<String> = headers.iter().map(|s| s.to_string()).collect();

    // Find primary key column indices
    let pk_indices: Vec<usize> = schema.pk_columns
        .iter()
        .filter_map(|pk| columns.iter().position(|c| c == pk))
        .collect();

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result?;
        let values: Vec<String> = record.iter().map(|s| s.to_string()).collect();

        let pk_values: Vec<String> = pk_indices
            .iter()
            .map(|&i| values.get(i).cloned().unwrap_or_default())
            .collect();

        rows.push(Row { pk_values, values });
    }

    Ok(Table {
        name: schema.name.clone(),
        columns,
        pk_columns: schema.pk_columns.clone(),
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::NamedTempFile;

    #[test]
    fn test_csv_empty_table() -> Result<()> {
        let schema = TableSchema {
            name: "empty".to_string(),
            columns: vec![
                crate::core::schema::Column {
                    name: "id".to_string(),
                    col_type: "INTEGER".to_string(),
                    notnull: false,
                    default_value: None,
                    pk: 1,
                },
                crate::core::schema::Column {
                    name: "name".to_string(),
                    col_type: "TEXT".to_string(),
                    notnull: false,
                    default_value: None,
                    pk: 0,
                },
            ],
            pk_columns: vec!["id".to_string()],
            sql: String::new(),
            indexes: vec![],
        };

        let table = Table {
            name: "empty".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            pk_columns: vec!["id".to_string()],
            rows: vec![],
        };

        let temp = NamedTempFile::new()?;
        write_table_csv(&table, temp.path())?;

        let read_back = read_table_csv(temp.path(), &schema)?;
        assert_eq!(read_back.name, "empty");
        assert_eq!(read_back.columns, vec!["id", "name"]);
        assert!(read_back.rows.is_empty());

        Ok(())
    }

    #[test]
    fn test_csv_roundtrip() -> Result<()> {
        let table = Table {
            name: "test".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            pk_columns: vec!["id".to_string()],
            rows: vec![
                Row {
                    pk_values: vec!["2".to_string()],
                    values: vec!["2".to_string(), "Bob".to_string()],
                },
                Row {
                    pk_values: vec!["1".to_string()],
                    values: vec!["1".to_string(), "Alice".to_string()],
                },
            ],
        };

        let temp = NamedTempFile::new()?;
        write_table_csv(&table, temp.path())?;

        // Check file content (should be sorted by PK)
        let mut content = String::new();
        File::open(temp.path())?.read_to_string(&mut content)?;
        assert!(content.contains("\"1\",\"Alice\""));
        assert!(content.contains("\"2\",\"Bob\""));

        // Alice (id=1) should come before Bob (id=2)
        let alice_pos = content.find("Alice").unwrap();
        let bob_pos = content.find("Bob").unwrap();
        assert!(alice_pos < bob_pos);

        Ok(())
    }

    #[test]
    fn test_csv_gz_roundtrip() -> Result<()> {
        let schema = TableSchema {
            name: "test".to_string(),
            columns: vec![
                crate::core::schema::Column {
                    name: "id".to_string(),
                    col_type: "INTEGER".to_string(),
                    notnull: false,
                    default_value: None,
                    pk: 1,
                },
                crate::core::schema::Column {
                    name: "name".to_string(),
                    col_type: "TEXT".to_string(),
                    notnull: false,
                    default_value: None,
                    pk: 0,
                },
            ],
            pk_columns: vec!["id".to_string()],
            sql: String::new(),
            indexes: vec![],
        };

        let table = Table {
            name: "test".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            pk_columns: vec!["id".to_string()],
            rows: vec![
                Row {
                    pk_values: vec!["2".to_string()],
                    values: vec!["2".to_string(), "Bob".to_string()],
                },
                Row {
                    pk_values: vec!["1".to_string()],
                    values: vec!["1".to_string(), "Alice".to_string()],
                },
            ],
        };

        let dir = tempfile::tempdir()?;
        let gz_path = dir.path().join("test.csv.gz");
        write_table_csv_gz(&table, &gz_path, false)?;

        // Verify the file exists and is a gzip file (magic bytes: 1f 8b)
        let mut header = [0u8; 2];
        File::open(&gz_path)?.read_exact(&mut header)?;
        assert_eq!(header, [0x1f, 0x8b], "File should be gzip compressed");

        // Read back using auto-detect
        let read_back = read_table_csv_auto(&gz_path, &schema)?;
        assert_eq!(read_back.name, "test");
        assert_eq!(read_back.columns, vec!["id", "name"]);
        assert_eq!(read_back.rows.len(), 2);

        // Rows should be sorted by PK (Alice first, then Bob)
        assert_eq!(read_back.rows[0].values[1], "Alice");
        assert_eq!(read_back.rows[1].values[1], "Bob");

        Ok(())
    }

    #[test]
    fn test_find_table_file() -> Result<()> {
        let dir = tempfile::tempdir()?;

        // No file -> None
        assert!(find_table_file(dir.path(), "users").is_none());

        // .csv file takes priority
        std::fs::write(dir.path().join("users.csv"), "id\n1\n")?;
        let found = find_table_file(dir.path(), "users");
        assert_eq!(found, Some(dir.path().join("users.csv")));

        // .csv.gz file found when .csv missing
        std::fs::remove_file(dir.path().join("users.csv"))?;
        std::fs::write(dir.path().join("users.csv.gz"), "dummy")?;
        let found = find_table_file(dir.path(), "users");
        assert_eq!(found, Some(dir.path().join("users.csv.gz")));

        // .csv preferred over .csv.gz when both exist
        std::fs::write(dir.path().join("users.csv"), "id\n1\n")?;
        let found = find_table_file(dir.path(), "users");
        assert_eq!(found, Some(dir.path().join("users.csv")));

        Ok(())
    }

    #[test]
    fn test_read_table_csv_auto_plain() -> Result<()> {
        let schema = TableSchema {
            name: "t".to_string(),
            columns: vec![
                crate::core::schema::Column {
                    name: "id".to_string(),
                    col_type: "INTEGER".to_string(),
                    notnull: false,
                    default_value: None,
                    pk: 1,
                },
            ],
            pk_columns: vec!["id".to_string()],
            sql: String::new(),
            indexes: vec![],
        };

        let dir = tempfile::tempdir()?;
        let csv_path = dir.path().join("t.csv");
        std::fs::write(&csv_path, "id\n1\n2\n")?;

        let table = read_table_csv_auto(&csv_path, &schema)?;
        assert_eq!(table.rows.len(), 2);

        Ok(())
    }
}
