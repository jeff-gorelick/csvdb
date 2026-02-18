use anyhow::{Context, Result};
use csv::ReaderBuilder;
use flate2::read::GzDecoder;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::core::config::{CsvdbConfig, CURRENT_FORMAT_VERSION};
use crate::core::csv::find_table_file;
use crate::core::Schema;

/// Result of validating a .csvdb or .parquetdb directory.
pub struct ValidateResult {
    pub table_count: usize,
    pub view_count: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// Validate a .csvdb or .parquetdb directory for structural integrity.
pub fn validate(dir: &Path) -> Result<ValidateResult> {
    // Detect format based on extension
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let is_parquetdb = dir_name.ends_with(".parquetdb");

    if is_parquetdb {
        validate_parquetdb(dir)
    } else {
        validate_csvdb(dir)
    }
}

fn validate_csvdb(csvdb_dir: &Path) -> Result<ValidateResult> {
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
            let msg = format!("schema.sql: {e}");
            println!("  schema.sql .............. ERROR: {e}");
            errors.push(msg);
            None
        }
    };

    let mut table_count = 0;
    let view_count;

    if let Some(ref schema) = schema {
        table_count = schema.tables.len();
        view_count = schema.views.len();

        // 2-4. Check each table's CSV file (parallel validation)
        let table_entries: Vec<_> = schema.tables.iter().collect();
        let validation_results: Vec<_> = table_entries
            .par_iter()
            .map(|(table_name, table_schema)| {
                let csv_path = find_table_file(csvdb_dir, table_name);
                (
                    (*table_name).clone(),
                    csv_path.clone(),
                    csv_path.map(|p| validate_csv(&p, table_schema)),
                )
            })
            .collect();

        for (table_name, csv_path, result) in validation_results {
            let csv_path = match csv_path {
                Some(p) => p,
                None => {
                    let msg = format!("{table_name}: missing CSV file");
                    println!("  {table_name}.csv .............. WARN: missing CSV file");
                    warnings.push(msg);
                    continue;
                }
            };

            let display_name = csv_path.file_name().unwrap_or_default().to_string_lossy();

            match result.unwrap() {
                Ok(csv_result) => {
                    println!(
                        "  {} .............. OK ({} rows)",
                        display_name, csv_result.row_count
                    );
                    if !csv_result.duplicate_pks.is_empty() {
                        let count = csv_result.duplicate_pks.len();
                        let examples: Vec<&str> = csv_result
                            .duplicate_pks
                            .iter()
                            .take(3)
                            .map(|s| s.as_str())
                            .collect();
                        let msg = format!(
                            "{}: {} duplicate primary key(s) found (e.g. {})",
                            display_name,
                            count,
                            examples.join("; ")
                        );
                        println!("  {display_name} .............. WARN: {count} duplicate PK(s)");
                        warnings.push(msg);
                    }
                }
                Err(e) => {
                    let msg = format!("{display_name}: {e}");
                    println!("  {display_name} .............. WARN: {e}");
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
                            "csvdb.toml: unknown format_version '{v}' (expected '{CURRENT_FORMAT_VERSION}')"
                        );
                        println!("  csvdb.toml .............. WARN: {msg}");
                        warnings.push(msg);
                    }
                }
            }
            Err(e) => {
                let msg = format!("csvdb.toml: {e}");
                println!("  csvdb.toml .............. WARN: {e}");
                warnings.push(msg);
            }
        }
    }

    // 6. Summary
    println!();
    if errors.is_empty() && warnings.is_empty() {
        println!("{table_count} tables validated, 0 warnings");
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

/// Result of validating a single CSV file.
struct CsvValidateResult {
    row_count: usize,
    duplicate_pks: Vec<String>,
}

/// Validate a single CSV file against its schema.
/// Supports both .csv and .csv.gz files.
fn validate_csv(csv_path: &Path, schema: &crate::core::TableSchema) -> Result<CsvValidateResult> {
    let file =
        File::open(csv_path).with_context(|| format!("Failed to open {}", csv_path.display()))?;

    let is_gz = csv_path.extension().map(|e| e == "gz").unwrap_or(false);

    let reader_box: Box<dyn Read> = if is_gz {
        Box::new(GzDecoder::new(BufReader::new(file)))
    } else {
        Box::new(file)
    };

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(reader_box);

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

    // Determine PK column indices for duplicate detection
    let pk_indices: Vec<usize> = schema
        .pk_columns
        .iter()
        .filter_map(|pk| header_names.iter().position(|h| h == pk))
        .collect();
    let check_pks = !pk_indices.is_empty();

    let mut seen_pks: HashSet<Vec<String>> = HashSet::new();
    let mut duplicate_pks: Vec<String> = Vec::new();

    // Check all rows parse and have correct column count
    let expected_cols = schema.columns.len();
    let mut row_count = 0;
    for (i, result) in reader.records().enumerate() {
        let record = result.with_context(|| format!("parse error at row {}", i + 1))?;
        if record.len() != expected_cols {
            anyhow::bail!(
                "row {} has {} columns, expected {}",
                i + 1,
                record.len(),
                expected_cols
            );
        }

        // Check for duplicate PKs
        if check_pks {
            let pk_tuple: Vec<String> = pk_indices
                .iter()
                .map(|&idx| record.get(idx).unwrap_or("").to_string())
                .collect();
            if !seen_pks.insert(pk_tuple.clone()) {
                duplicate_pks.push(pk_tuple.join(", "));
            }
        }

        row_count += 1;
    }

    Ok(CsvValidateResult {
        row_count,
        duplicate_pks,
    })
}

fn validate_parquetdb(parquetdb_dir: &Path) -> Result<ValidateResult> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    println!("Validating {}/", parquetdb_dir.display());

    // 1. Check schema.sql exists and parses
    let schema_path = parquetdb_dir.join("schema.sql");
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
            let msg = format!("schema.sql: {e}");
            println!("  schema.sql .............. ERROR: {e}");
            errors.push(msg);
            None
        }
    };

    let mut table_count = 0;
    let view_count;

    if let Some(ref schema) = schema {
        table_count = schema.tables.len();
        view_count = schema.views.len();

        // 2. Check each table's Parquet file (parallel validation)
        let parquet_entries: Vec<_> = schema.tables.iter().collect();
        let parquet_results: Vec<_> = parquet_entries
            .par_iter()
            .map(|(table_name, table_schema)| {
                let parquet_path = parquetdb_dir.join(format!("{table_name}.parquet"));
                let result = if parquet_path.exists() {
                    Some(validate_parquet(&parquet_path, table_schema))
                } else {
                    None
                };
                ((*table_name).clone(), result)
            })
            .collect();

        for (table_name, result) in parquet_results {
            match result {
                None => {
                    let msg = format!("{table_name}: missing Parquet file");
                    println!("  {table_name}.parquet .............. WARN: missing Parquet file");
                    warnings.push(msg);
                }
                Some(Ok(row_count)) => {
                    println!("  {table_name}.parquet .............. OK ({row_count} rows)");
                }
                Some(Err(e)) => {
                    let msg = format!("{table_name}.parquet: {e}");
                    println!("  {table_name}.parquet .............. WARN: {e}");
                    warnings.push(msg);
                }
            }
        }
    } else {
        view_count = 0;
    }

    // 3. Check csvdb.toml if present
    let toml_path = parquetdb_dir.join("csvdb.toml");
    if toml_path.exists() {
        match CsvdbConfig::load(parquetdb_dir) {
            Ok(config) => {
                println!("  csvdb.toml .............. OK");
                if let Some(ref v) = config.format_version {
                    if v != CURRENT_FORMAT_VERSION {
                        let msg = format!(
                            "csvdb.toml: unknown format_version '{v}' (expected '{CURRENT_FORMAT_VERSION}')"
                        );
                        println!("  csvdb.toml .............. WARN: {msg}");
                        warnings.push(msg);
                    }
                }
            }
            Err(e) => {
                let msg = format!("csvdb.toml: {e}");
                println!("  csvdb.toml .............. WARN: {e}");
                warnings.push(msg);
            }
        }
    }

    // 4. Summary
    println!();
    if errors.is_empty() && warnings.is_empty() {
        println!("{table_count} tables validated, 0 warnings");
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

/// Validate a single Parquet file against its schema using DuckDB.
fn validate_parquet(parquet_path: &Path, schema: &crate::core::TableSchema) -> Result<usize> {
    let conn = duckdb::Connection::open_in_memory()
        .context("Failed to create in-memory DuckDB connection")?;

    let abs_path = parquet_path
        .canonicalize()
        .with_context(|| format!("Failed to get absolute path: {}", parquet_path.display()))?;
    let path_str = abs_path.to_string_lossy().replace('\\', "/");
    let path_str = path_str.strip_prefix("//?/").unwrap_or(&path_str);

    // Create a table from the parquet file to check its structure
    let table_name = parquet_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("parquet_table");

    conn.execute(
        &format!(
            "CREATE TABLE \"{table_name}\" AS SELECT * FROM read_parquet('{path_str}') LIMIT 0"
        ),
        [],
    )
    .with_context(|| format!("Failed to read parquet file: {}", parquet_path.display()))?;

    // Get column count from the created table
    let parquet_cols: usize = conn.query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table_name}')"),
        [],
        |row| row.get(0),
    )?;

    let schema_cols = schema.columns.len();

    if parquet_cols != schema_cols {
        anyhow::bail!(
            "column count mismatch: Parquet has {parquet_cols} columns, schema expects {schema_cols}"
        );
    }

    // Count rows
    let row_count: usize = conn.query_row(
        &format!("SELECT COUNT(*) FROM read_parquet('{path_str}')"),
        [],
        |row| row.get(0),
    )?;

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
    fn test_validate_duplicate_pk_warning() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("dup.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        // Duplicate PK: id=1 appears twice
        fs::write(
            csvdb_dir.join("t.csv"),
            "id,name\n1,Alice\n1,Bob\n2,Charlie\n",
        )?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty());
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("duplicate primary key")));
        Ok(())
    }

    #[test]
    fn test_validate_no_pk_skips_duplicate_check() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("nopk.csvdb");
        fs::create_dir(&csvdb_dir)?;

        // Table without PK
        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"a\" TEXT,\n    \"b\" TEXT\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "a,b\nfoo,bar\nfoo,bar\n")?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        // No duplicate PK warnings because there's no PK
        assert!(result.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn test_validate_unique_pks_clean() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("clean.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;
        fs::write(
            csvdb_dir.join("t.csv"),
            "id,name\n1,Alice\n2,Bob\n3,Charlie\n",
        )?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
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

    #[test]
    fn test_validate_parquetdb() -> Result<()> {
        use rusqlite::Connection;

        let dir = tempdir()?;
        let db_path = dir.path().join("test.sqlite");

        {
            let conn = Connection::open(&db_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])?;
            conn.execute("INSERT INTO t VALUES (1, 'hello')", [])?;
            conn.execute("INSERT INTO t VALUES (2, 'world')", [])?;
        }

        // Create parquetdb
        let parquetdb = crate::commands::to_parquetdb::to_parquetdb(
            &db_path,
            crate::OrderMode::Pk,
            crate::NullMode::Marker,
            None,
            None,
            true,
            &crate::TableFilter::new(vec![], vec![]),
        )?;

        let result = validate(&parquetdb)?;
        assert_eq!(result.table_count, 1);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn test_validate_parquetdb_missing_parquet() -> Result<()> {
        let dir = tempdir()?;
        let parquetdb_dir = dir.path().join("test.parquetdb");
        fs::create_dir(&parquetdb_dir)?;

        fs::write(
            parquetdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n",
        )?;
        // No t.parquet file

        let result = validate(&parquetdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("missing Parquet"));
        Ok(())
    }

    #[test]
    fn test_validate_parquetdb_missing_schema() -> Result<()> {
        let dir = tempdir()?;
        let parquetdb_dir = dir.path().join("bad.parquetdb");
        fs::create_dir(&parquetdb_dir)?;
        // No schema.sql

        let result = validate(&parquetdb_dir)?;
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].contains("schema.sql"));
        Ok(())
    }

    #[test]
    fn test_validate_compressed_csv() -> Result<()> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n",
        )?;

        // Write compressed CSV
        let gz_path = csvdb_dir.join("t.csv.gz");
        let f = File::create(&gz_path)?;
        let mut encoder = GzEncoder::new(f, Compression::default());
        encoder.write_all(b"id,name\n1,Alice\n2,Bob\n")?;
        encoder.finish()?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn test_validate_with_csvdb_toml() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;
        fs::write(
            csvdb_dir.join("csvdb.toml"),
            "format_version = \"1\"\norder = \"pk\"\n",
        )?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn test_validate_with_bad_format_version() -> Result<()> {
        let dir = tempdir()?;
        let csvdb_dir = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb_dir)?;

        fs::write(
            csvdb_dir.join("schema.sql"),
            "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY\n);\n",
        )?;
        fs::write(csvdb_dir.join("t.csv"), "id\n1\n")?;
        fs::write(csvdb_dir.join("csvdb.toml"), "format_version = \"999\"\n")?;

        let result = validate(&csvdb_dir)?;
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("format_version")));
        Ok(())
    }
}
