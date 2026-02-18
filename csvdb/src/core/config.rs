use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::{NullMode, OrderMode};

pub const CURRENT_FORMAT_VERSION: &str = "1";

pub fn created_by_string() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

/// Configuration stored in csvdb.toml alongside a .csvdb directory.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CsvdbConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub null_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub natural_sort: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compressed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tables: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_checksums: Option<BTreeMap<String, String>>,
}

impl CsvdbConfig {
    /// Load config from a csvdb.toml file. Returns default if file doesn't exist.
    pub fn load(csvdb_dir: &Path) -> Result<Self> {
        let config_path = csvdb_dir.join("csvdb.toml");
        if !config_path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        let config: CsvdbConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?;
        Ok(config)
    }

    /// Parse the order field as an OrderMode.
    pub fn order_mode(&self) -> Option<OrderMode> {
        self.order
            .as_ref()
            .and_then(|s| OrderMode::from_str(s, true).ok())
    }

    /// Parse the null_mode field as a NullMode.
    pub fn null_mode(&self) -> Option<NullMode> {
        self.null_mode
            .as_ref()
            .and_then(|s| NullMode::from_str(s, true).ok())
    }

    /// Write config to a csvdb.toml file.
    pub fn write(&self, csvdb_dir: &Path) -> Result<()> {
        let config_path = csvdb_dir.join("csvdb.toml");
        let content = toml::to_string_pretty(self).context("Failed to serialize csvdb.toml")?;
        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_load_missing_config() -> Result<()> {
        let dir = tempdir()?;
        let config = CsvdbConfig::load(dir.path())?;
        assert!(config.order.is_none());
        assert!(config.null_mode.is_none());
        Ok(())
    }

    #[test]
    fn test_config_order_mode() -> Result<()> {
        let config = CsvdbConfig {
            order: Some("pk".to_string()),
            null_mode: Some("empty".to_string()),
            ..Default::default()
        };
        assert_eq!(config.order_mode(), Some(OrderMode::Pk));
        assert_eq!(config.null_mode(), Some(NullMode::Empty));

        let config2 = CsvdbConfig {
            order: Some("all-columns".to_string()),
            null_mode: Some("marker".to_string()),
            ..Default::default()
        };
        assert_eq!(config2.order_mode(), Some(OrderMode::AllColumns));
        assert_eq!(config2.null_mode(), Some(NullMode::Marker));

        let empty = CsvdbConfig::default();
        assert_eq!(empty.order_mode(), None);
        assert_eq!(empty.null_mode(), None);
        Ok(())
    }

    #[test]
    fn test_roundtrip_config() -> Result<()> {
        let dir = tempdir()?;
        let config = CsvdbConfig {
            format_version: Some(CURRENT_FORMAT_VERSION.to_string()),
            created_by: Some(created_by_string()),
            order: Some("pk".to_string()),
            null_mode: Some("marker".to_string()),
            natural_sort: None,
            order_by: None,
            compressed: None,
            tables: None,
            exclude: None,
            table_checksums: None,
        };
        config.write(dir.path())?;

        let loaded = CsvdbConfig::load(dir.path())?;
        assert_eq!(loaded.format_version.as_deref(), Some("1"));
        assert_eq!(loaded.created_by.as_deref(), Some(&*created_by_string()));
        assert_eq!(loaded.order.as_deref(), Some("pk"));
        assert_eq!(loaded.null_mode.as_deref(), Some("marker"));
        Ok(())
    }
}
