use anyhow::{Context, Result};
use csv::ReaderBuilder;
use std::fs::File;
use std::path::Path;

use crate::core::Schema;
use crate::core::config::{CsvdbConfig, CURRENT_FORMAT_VERSION};

/// Result of validating a .csvdb directory.
pub struct ValidateResult {
    pub table_count: usize,
    pub view_count: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// Validate a .csvdb directory for structural integrity.
pub fn validate(csvdb_dir: &Path) -> Result<ValidateResult> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    println!("Validating {}/", csvdb_dir.display());

    // 1. Check schema.sql exists and parses
    let schema_path = csvdb_dir.join("schema.sql");
    let schema = match Schema::from_schema_sql(&schema_path) {
        Ok(s) => {
            println!(
                "  schema.sql .............. OK ({} tables, {} views)",
                s.tables.len(),
                s.views.len()
            );
            Some(s)
        }
        Err(e) => {
            let msg = format!("schema.sql: {}", e);
            println!("  schema.sql .............. ERROR: {}", e);
            errors.push(msg);
            None
        }
    };

    let mut table_count = 0;
    let view_count;

    if let Some(ref schema) = schema {
        table_count = schema.tables.len();
        view_count = schema.views.len();

        // 2-4. Check each table's CSV file
        for (table_name, table_schema) in &schema.tables {
            let csv_path = csvdb_dir.join(format!("{}.csv", table_name));

            if !csv_path.exists() {
                let msg = format!("{}: missing CSV file", table_name);
                println!("  {}.csv .............. WARN: missing CSV file", table_name);
                warnings.push(msg);
                continue;
            }

            // Try to read and validate
            match validate_csv(&csv_path, table_schema) {
                Ok(row_count) => {
                    println!("  {}.csv .............. OK ({} rows)", table_name, row_count);
                }
                Err(e) => {
                    let msg = format!("{}.csv: {}", table_name, e);
                    println!("  {}.csv .............. WARN: {}", table_name, e);
                    warnings.push(msg);
                }
            }
        }
    } else {
        view_count = 0;
    }

    // 5. Check csvdb.toml if present
    let toml_path = csvdb_dir.join("csvdb.toml");
    if toml_path.exists() {
        match CsvdbConfig::load(csvdb_dir) {
            Ok(config) => {
                println!("  csvdb.toml .............. OK");
                if let Some(ref v) = config.format_version {
                    if v != CURRENT_FORMAT_VERSION {
                        let msg = format!(
                            "csvdb.toml: unknown format_version '{}' (expected '{}')",
                            v, CURRENT_FORMAT_VERSION
                        );
                        println!("  csvdb.toml .............. WARN: {}", msg);
                        warnings.push(msg);
                    }
                }
            }
            Err(e) => {
                let msg = format!("csvdb.toml: {}", e);
                println!("  csvdb.toml .............. WARN: {}", e);
                warnings.push(msg);
            }
        }
    }

    // 6. Summary
    println!();
    if errors.is_empty() && warnings.is_empty() {
        println!("{} tables validated, 0 warnings", table_count);
    } else if errors.is_empty() {
        println!(
            "{} tables validated, {} warning{}",
            table_count,
            warnings.len(),
            if warnings.len() == 1 { "" } else { "s" }
        );
    } else {
        println!(
            "{} error{}, {} warning{}",
            errors.len(),
            if errors.len() == 1 { "" } else { "s" },
            warnings.len(),
            if warnings.len() == 1 { "" } else { "s" }
        );
    }

    Ok(ValidateResult {
        table_count,
        view_count,
        warnings,
        errors,
    })
}

/// Validate a single CSV file against its schema.
fn validate_csv(
    csv_path: &Path,
    schema: &crate::core::TableSchema,
) -> Result<usize> {
    let file = File::open(csv_path)
        .with_context(|| format!("Failed to open {}", csv_path.display()))?;

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(file);

    // Check header matches schema columns
    let headers = reader.headers()?.clone();
    let header_names: Vec<&str> = headers.iter().collect();
    let schema_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();

    if header_names != schema_names {
        anyhow::bail!(
            "column mismatch: CSV has [{}], schema expects [{}]",
            header_names.join(", "),
            schema_names.join(", ")
        );
    }

    // Check all rows parse and have correct column count
    let expected_cols = schema.columns.len();
    let mut row_count = 0;
    for (i, result) in reader.records().enumerate() {
        let record = result
            .with_context(|| format!("parse error at row {}", i + 1))?;
        if record.len() != expected_cols {
            anyhow::bail!(
                "row {} has {} columns, expected {}",
                i + 1,
                record.len(),
                expected_cols
            );
        }
        row_count += 1;
    }

    Ok(row_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_validate_valid_csvdb() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"users\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        fs::write(csvdb_dir.join("users.csv"), "id,name\n1,Alice\n2,Bob\n")?;

        let result = validate(&csvdb_dir)?;
        assert_eq!(result.table_count, 1);
        assert_eq!(result.view_count, 0);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn test_validate_missing_schema() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("bad.csvdb");
        fs::create_dir(&csvdb_dir)?;
        // No schema.sql

        let result = validate(&csvdb_dir)?;
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].contains("schema.sql"));
        Ok(())
    }

    #[test]
    fn test_validate_missing_csv() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"users\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        // No users.csv

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("missing CSV"));
        Ok(())
    }

    #[test]
    fn test_validate_column_mismatch() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "id,wrong_col\n1,bad\n")?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("column mismatch"));
        Ok(())
    }

    #[test]
    fn test_validate_row_column_count() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        // Row has 3 fields but schema expects 2
        fs::write(csvdb_dir.join("t.csv"), "id,name\n1,Alice,extra\n")?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn test_validate_with_views() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"users\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
             CREATE VIEW \"active_users\" AS SELECT * FROM \"users\";\n",
        )?;
        fs::write(csvdb_dir.join("users.csv"), "id,name\n1,Alice\n")?;

        let result = validate(&csvdb_dir)?;
        assert_eq!(result.table_count, 1);
        assert_eq!(result.view_count, 1);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        Ok(())
    }
}
