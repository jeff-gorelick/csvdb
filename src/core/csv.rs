use anyhow::{Context, Result};
use csv::{ReaderBuilder, WriterBuilder};
use std::fs::File;
use std::path::Path;

use super::table::{Row, Table};
use super::schema::TableSchema;

/// Write a table to CSV with deterministic formatting.
/// - Header row with column names
/// - Data rows sorted by primary key
/// - Consistent quoting (quote all fields)
pub fn write_table_csv(table: &Table, path: &Path) -> Result<()> {
    let file = File::create(path)
        .with_context(|| format!("Failed to create CSV: {}", path.display()))?;

    let mut writer = WriterBuilder::new()
        .quote_style(csv::QuoteStyle::Always)
        .from_writer(file);

    // Write header
    writer.write_record(&table.columns)?;

    // Sort rows by primary key for determinism
    let mut sorted_rows = table.rows.clone();
    sorted_rows.sort_by(|a, b| a.pk_values.cmp(&b.pk_values));

    // Write data rows
    for row in &sorted_rows {
        writer.write_record(&row.values)?;
    }

    writer.flush()?;
    Ok(())
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
}
