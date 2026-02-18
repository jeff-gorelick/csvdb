use anyhow::{bail, Context, Result};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::sql::arrow_value_to_string;
use crate::core::config::{created_by_string, CsvdbConfig, CURRENT_FORMAT_VERSION};
use crate::NULL_MARKER;

/// Write Arrow tables to a csvdb directory.
///
/// Creates schema.sql from Arrow schemas, writes CSV files with \N null markers,
/// and generates csvdb.toml configuration.
pub fn write_csvdb_from_arrow(
    tables: Vec<(String, Vec<RecordBatch>)>,
    output: &Path,
    force: bool,
) -> Result<PathBuf> {
    if output.exists() {
        if !force {
            bail!(
                "Output directory already exists: {}\nUse force=True to overwrite.",
                output.display()
            );
        }
        fs::remove_dir_all(output)?;
    }
    fs::create_dir_all(output)?;

    // Generate schema.sql
    let mut schema_sql = String::new();
    for (name, batches) in &tables {
        if batches.is_empty() {
            continue;
        }
        let arrow_schema = batches[0].schema();
        let create_table = arrow_schema_to_create_table(name, &arrow_schema);
        schema_sql.push_str(&create_table);
    }
    fs::write(output.join("schema.sql"), &schema_sql)?;

    // Write CSV files
    for (name, batches) in &tables {
        let csv_path = output.join(format!("{name}.csv"));
        write_arrow_to_csv(&csv_path, batches)
            .with_context(|| format!("Failed to write table: {name}"))?;
    }

    // Write config
    let config = CsvdbConfig {
        format_version: Some(CURRENT_FORMAT_VERSION.to_string()),
        created_by: Some(created_by_string()),
        order: Some("pk".to_string()),
        null_mode: Some("marker".to_string()),
        ..Default::default()
    };
    config.write(output)?;

    Ok(output.to_path_buf())
}

fn arrow_type_to_sql(dt: &DataType) -> &'static str {
    match dt {
        DataType::Boolean => "INTEGER",
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => "INTEGER",
        DataType::Float16 | DataType::Float32 | DataType::Float64 => "REAL",
        _ => "TEXT",
    }
}

fn arrow_schema_to_create_table(name: &str, schema: &arrow::datatypes::Schema) -> String {
    let mut sql = format!("CREATE TABLE \"{name}\" (\n");

    let fields = schema.fields();
    let pk_col = detect_pk(name, schema);

    let col_defs: Vec<String> = fields
        .iter()
        .map(|field| {
            let sql_type = arrow_type_to_sql(field.data_type());
            let mut constraints = String::new();
            if pk_col.as_deref() == Some(field.name()) {
                constraints.push_str(" PRIMARY KEY");
            }
            if !field.is_nullable() && pk_col.as_deref() != Some(field.name()) {
                constraints.push_str(" NOT NULL");
            }
            format!("    \"{}\" {}{}", field.name(), sql_type, constraints)
        })
        .collect();

    sql.push_str(&col_defs.join(",\n"));
    sql.push_str("\n);\n");
    sql
}

/// Simple PK detection: look for a column named "id" or "<table>_id".
fn detect_pk(table_name: &str, schema: &arrow::datatypes::Schema) -> Option<String> {
    let fields = schema.fields();

    // Check for "id" column
    if fields.iter().any(|f| f.name() == "id") {
        return Some("id".to_string());
    }

    // Check for "<table>_id" column (singular form)
    let singular = table_name.strip_suffix('s').unwrap_or(table_name);
    let expected = format!("{singular}_id");
    if fields.iter().any(|f| f.name() == &expected) {
        return Some(expected);
    }

    None
}

fn write_arrow_to_csv(path: &Path, batches: &[RecordBatch]) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;

    if batches.is_empty() {
        return Ok(());
    }

    // Write header
    let schema = batches[0].schema();
    let header: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    wtr.write_record(&header)?;

    // Write rows
    for batch in batches {
        for row_idx in 0..batch.num_rows() {
            let mut row = Vec::with_capacity(batch.num_columns());
            for col_idx in 0..batch.num_columns() {
                let col = batch.column(col_idx);
                if col.is_null(row_idx) {
                    row.push(NULL_MARKER.to_string());
                } else {
                    row.push(arrow_value_to_string(col.as_ref(), row_idx));
                }
            }
            wtr.write_record(&row)?;
        }
    }

    wtr.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{Field, Schema};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn make_test_batches() -> Vec<(String, Vec<RecordBatch>)> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("score", DataType::Int32, true),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["Alice", "Bob", "Charlie"])),
                Arc::new(Int32Array::from(vec![Some(95), Some(87), None])),
            ],
        )
        .unwrap();

        vec![("users".to_string(), vec![batch])]
    }

    #[test]
    fn test_write_csvdb_from_arrow() -> Result<()> {
        let dir = tempdir()?;
        let output = dir.path().join("test.csvdb");
        let tables = make_test_batches();

        let result = write_csvdb_from_arrow(tables, &output, false)?;
        assert_eq!(result, output);
        assert!(output.join("schema.sql").exists());
        assert!(output.join("users.csv").exists());
        assert!(output.join("csvdb.toml").exists());

        // Check schema.sql
        let schema = fs::read_to_string(output.join("schema.sql"))?;
        assert!(schema.contains("CREATE TABLE \"users\""));
        assert!(schema.contains("\"id\" INTEGER PRIMARY KEY"));
        assert!(schema.contains("\"name\" TEXT NOT NULL"));
        assert!(schema.contains("\"score\" INTEGER"));

        // Check CSV
        let csv = fs::read_to_string(output.join("users.csv"))?;
        assert!(csv.starts_with("id,name,score\n"));
        assert!(csv.contains("1,Alice,95"));
        assert!(csv.contains("3,Charlie,\\N"));

        Ok(())
    }

    #[test]
    fn test_write_csvdb_from_arrow_force() -> Result<()> {
        let dir = tempdir()?;
        let output = dir.path().join("test.csvdb");
        let tables = make_test_batches();

        // First write
        write_csvdb_from_arrow(tables.clone(), &output, false)?;

        // Second write without force should fail
        let err = write_csvdb_from_arrow(tables.clone(), &output, false);
        assert!(err.is_err());

        // With force should succeed
        write_csvdb_from_arrow(tables, &output, true)?;
        assert!(output.join("schema.sql").exists());

        Ok(())
    }

    #[test]
    fn test_arrow_type_to_sql() {
        assert_eq!(arrow_type_to_sql(&DataType::Int32), "INTEGER");
        assert_eq!(arrow_type_to_sql(&DataType::Int64), "INTEGER");
        assert_eq!(arrow_type_to_sql(&DataType::Float64), "REAL");
        assert_eq!(arrow_type_to_sql(&DataType::Utf8), "TEXT");
        assert_eq!(arrow_type_to_sql(&DataType::Boolean), "INTEGER");
        assert_eq!(arrow_type_to_sql(&DataType::Date32), "TEXT");
    }

    #[test]
    fn test_detect_pk() {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
        ]);
        assert_eq!(detect_pk("users", &schema), Some("id".to_string()));

        let schema = Schema::new(vec![
            Field::new("user_id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
        ]);
        assert_eq!(detect_pk("users", &schema), Some("user_id".to_string()));

        let schema = Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Int32, false),
        ]);
        assert_eq!(detect_pk("data", &schema), None);
    }

    #[test]
    fn test_roundtrip_arrow_to_csvdb_to_arrow() -> Result<()> {
        let dir = tempdir()?;
        let output = dir.path().join("roundtrip.csvdb");
        let tables = make_test_batches();

        // Write
        write_csvdb_from_arrow(tables, &output, false)?;

        // Read back
        use crate::commands::read::read_table_arrow;
        let batches = read_table_arrow(&output, "users")?;

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);

        // Check schema fields
        let schema = batches[0].schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec!["id", "name", "score"]);

        Ok(())
    }
}
