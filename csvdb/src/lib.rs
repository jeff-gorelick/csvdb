#![allow(clippy::too_many_arguments)]

pub mod core;
pub mod commands;

pub use commands::{init, to_csv, to_duckdb, to_sqlite};
pub use core::config::CsvdbConfig;
pub use core::input::InputFormat;

use clap::ValueEnum;

/// Controls row ordering when exporting tables to CSV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum OrderMode {
    /// Order by primary key (default). Requires all tables to have a PK.
    #[default]
    Pk,
    /// Order by all columns. Works without PK but may have issues with duplicate rows.
    AllColumns,
    /// Add a synthetic `__csvdb_rowid` column for ordering. Best for event/log tables.
    AddSyntheticKey,
}

/// Controls how NULL values are represented in CSV files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum NullMode {
    /// Use \N to represent NULL (PostgreSQL convention). Default.
    /// Lossless: empty string "" is preserved as empty string.
    #[default]
    #[value(name = "marker", alias = "postgres", alias = "mysql")]
    Marker,
    /// Use empty string for NULL. LOSSY: cannot distinguish NULL from empty string.
    #[value(name = "empty", alias = "excel")]
    Empty,
    /// Use literal string "NULL" for NULL. LOSSY: cannot distinguish NULL from the string "NULL".
    #[value(name = "literal")]
    Literal,
}

impl NullMode {
    /// Returns true if this mode is lossy (cannot roundtrip all data).
    pub fn is_lossy(&self) -> bool {
        matches!(self, NullMode::Empty | NullMode::Literal)
    }

    /// Returns the string representation for NULL in this mode.
    pub fn null_string(&self) -> &'static str {
        match self {
            NullMode::Marker => NULL_MARKER,
            NullMode::Empty => "",
            NullMode::Literal => "NULL",
        }
    }

    /// Returns true if the given value represents NULL in this mode.
    pub fn is_null(&self, value: &str) -> bool {
        match self {
            NullMode::Marker => value == NULL_MARKER,
            NullMode::Empty => value.is_empty(),
            NullMode::Literal => value == "NULL",
        }
    }
}

/// The marker string used to represent NULL values in CSV files (default mode).
pub const NULL_MARKER: &str = "\\N";

/// Filter for selecting a subset of tables.
pub struct TableFilter {
    pub tables: Vec<String>,   // include list (empty = all)
    pub exclude: Vec<String>,  // exclude list
}

impl TableFilter {
    pub fn new(tables: Vec<String>, exclude: Vec<String>) -> Self {
        Self { tables, exclude }
    }

    pub fn matches(&self, table_name: &str) -> bool {
        if !self.tables.is_empty() && !self.tables.iter().any(|t| t == table_name) {
            return false;
        }
        !self.exclude.iter().any(|t| t == table_name)
    }

    pub fn is_active(&self) -> bool {
        !self.tables.is_empty() || !self.exclude.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_mode_is_lossy() {
        assert!(!NullMode::Marker.is_lossy());
        assert!(NullMode::Empty.is_lossy());
        assert!(NullMode::Literal.is_lossy());
    }

    #[test]
    fn test_null_mode_null_string() {
        assert_eq!(NullMode::Marker.null_string(), "\\N");
        assert_eq!(NullMode::Empty.null_string(), "");
        assert_eq!(NullMode::Literal.null_string(), "NULL");
    }

    #[test]
    fn test_null_mode_is_null() {
        assert!(NullMode::Marker.is_null("\\N"));
        assert!(NullMode::Empty.is_null(""));
        assert!(NullMode::Literal.is_null("NULL"));
    }

    #[test]
    fn test_null_mode_alias_parsing() {
        use clap::ValueEnum;
        // "postgres" and "mysql" should resolve to Marker
        assert_eq!(NullMode::from_str("postgres", true).unwrap(), NullMode::Marker);
        assert_eq!(NullMode::from_str("mysql", true).unwrap(), NullMode::Marker);
        // "excel" should resolve to Empty
        assert_eq!(NullMode::from_str("excel", true).unwrap(), NullMode::Empty);
        // canonical names still work
        assert_eq!(NullMode::from_str("marker", true).unwrap(), NullMode::Marker);
        assert_eq!(NullMode::from_str("empty", true).unwrap(), NullMode::Empty);
        assert_eq!(NullMode::from_str("literal", true).unwrap(), NullMode::Literal);
    }

    #[test]
    fn test_null_mode_is_null_edge_cases() {
        // Marker doesn't treat empty as null
        assert!(!NullMode::Marker.is_null(""));
        // Marker doesn't treat "NULL" as null
        assert!(!NullMode::Marker.is_null("NULL"));

        // Empty doesn't treat \N as null
        assert!(!NullMode::Empty.is_null("\\N"));
        // Empty doesn't treat "NULL" as null
        assert!(!NullMode::Empty.is_null("NULL"));

        // Literal doesn't treat empty as null
        assert!(!NullMode::Literal.is_null(""));
        // Literal doesn't treat \N as null
        assert!(!NullMode::Literal.is_null("\\N"));
    }

    #[test]
    fn test_table_filter_empty_matches_all() {
        let filter = TableFilter::new(vec![], vec![]);
        assert!(filter.matches("users"));
        assert!(filter.matches("orders"));
        assert!(filter.matches("anything"));
    }

    #[test]
    fn test_table_filter_include_list() {
        let filter = TableFilter::new(vec!["users".to_string(), "orders".to_string()], vec![]);
        assert!(filter.matches("users"));
        assert!(filter.matches("orders"));
        assert!(!filter.matches("products"));
        assert!(!filter.matches("other"));
    }

    #[test]
    fn test_table_filter_exclude_list() {
        let filter = TableFilter::new(vec![], vec!["users".to_string()]);
        assert!(!filter.matches("users"));
        assert!(filter.matches("orders"));
        assert!(filter.matches("products"));
    }

    #[test]
    fn test_table_filter_is_active() {
        let empty = TableFilter::new(vec![], vec![]);
        assert!(!empty.is_active());

        let with_include = TableFilter::new(vec!["users".to_string()], vec![]);
        assert!(with_include.is_active());

        let with_exclude = TableFilter::new(vec![], vec!["users".to_string()]);
        assert!(with_exclude.is_active());

        let with_both = TableFilter::new(vec!["orders".to_string()], vec!["users".to_string()]);
        assert!(with_both.is_active());
    }
}
