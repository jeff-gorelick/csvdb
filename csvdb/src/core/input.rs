use anyhow::{bail, Result};
use std::path::Path;

/// Detected input format type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    /// SQLite database (.sqlite, .sqlite3, .db)
    Sqlite,
    /// DuckDB database (.duckdb)
    DuckDb,
    /// csvdb directory (.csvdb/)
    Csvdb,
    /// parquetdb directory (.parquetdb/)
    Parquetdb,
    /// Single parquet file (.parquet)
    Parquet,
}

impl InputFormat {
    /// Detect input format by examining file extension and path structure.
    pub fn from_path(path: &Path) -> Result<Self> {
        // Check if it's a directory
        if path.is_dir() {
            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            // Check extension-based detection first
            if dir_name.ends_with(".parquetdb") {
                return Ok(InputFormat::Parquetdb);
            }
            if dir_name.ends_with(".csvdb") {
                return Ok(InputFormat::Csvdb);
            }

            // No extension - check contents
            if path.join("schema.sql").exists() {
                // Has schema.sql - check if it has parquet files or csv files
                let has_parquet = std::fs::read_dir(path)
                    .map(|entries| {
                        entries.filter_map(|e| e.ok()).any(|e| {
                            e.path().extension().and_then(|ext| ext.to_str()) == Some("parquet")
                        })
                    })
                    .unwrap_or(false);

                if has_parquet {
                    return Ok(InputFormat::Parquetdb);
                }
                return Ok(InputFormat::Csvdb);
            }

            bail!(
                "Cannot detect input format for directory: {}. \
                 Expected .csvdb or .parquetdb directory.",
                path.display()
            );
        }

        // It's a file - check extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        match ext.as_deref() {
            Some("sqlite") | Some("sqlite3") | Some("db") => Ok(InputFormat::Sqlite),
            Some("duckdb") => Ok(InputFormat::DuckDb),
            Some("parquet") => Ok(InputFormat::Parquet),
            _ => bail!(
                "Cannot detect input format for: {}. \
                 Supported: SQLite (.sqlite, .db), DuckDB (.duckdb), Parquet (.parquet), \
                 or csvdb/parquetdb directories.",
                path.display()
            ),
        }
    }

    /// Returns the default output extension for this format.
    pub fn default_output_extension(&self) -> &'static str {
        match self {
            InputFormat::Sqlite => "sqlite",
            InputFormat::DuckDb => "duckdb",
            InputFormat::Csvdb => "csvdb",
            InputFormat::Parquetdb => "parquetdb",
            InputFormat::Parquet => "parquet",
        }
    }

    /// Returns true if this is a directory-based format.
    pub fn is_directory(&self) -> bool {
        matches!(self, InputFormat::Csvdb | InputFormat::Parquetdb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_detect_sqlite() {
        assert_eq!(InputFormat::from_path(Path::new("test.sqlite")).unwrap(), InputFormat::Sqlite);
        assert_eq!(InputFormat::from_path(Path::new("test.sqlite3")).unwrap(), InputFormat::Sqlite);
        assert_eq!(InputFormat::from_path(Path::new("test.db")).unwrap(), InputFormat::Sqlite);
    }

    #[test]
    fn test_detect_duckdb() {
        assert_eq!(InputFormat::from_path(Path::new("test.duckdb")).unwrap(), InputFormat::DuckDb);
    }

    #[test]
    fn test_detect_parquet() {
        assert_eq!(InputFormat::from_path(Path::new("test.parquet")).unwrap(), InputFormat::Parquet);
    }

    #[test]
    fn test_detect_csvdb_dir() -> Result<()> {
        let dir = tempdir()?;
        let csvdb = dir.path().join("test.csvdb");
        fs::create_dir(&csvdb)?;
        fs::write(csvdb.join("schema.sql"), "CREATE TABLE t (id INTEGER);")?;

        assert_eq!(InputFormat::from_path(&csvdb)?, InputFormat::Csvdb);
        Ok(())
    }

    #[test]
    fn test_detect_parquetdb_dir() -> Result<()> {
        let dir = tempdir()?;
        let parquetdb = dir.path().join("test.parquetdb");
        fs::create_dir(&parquetdb)?;
        fs::write(parquetdb.join("schema.sql"), "CREATE TABLE t (id INTEGER);")?;
        fs::write(parquetdb.join("t.parquet"), "")?; // Empty file just for detection

        assert_eq!(InputFormat::from_path(&parquetdb)?, InputFormat::Parquetdb);
        Ok(())
    }

    #[test]
    fn test_unknown_format() {
        assert!(InputFormat::from_path(Path::new("test.unknown")).is_err());
    }
}
