use anyhow::{Context, Result};
use duckdb::Connection as DuckDbConnection;
use rusqlite::Connection;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

use super::schema::TableSchema;
use crate::{OrderMode, NullMode, NULL_MARKER};

/// Name of the synthetic rowid column added in AddSyntheticKey mode.
pub const SYNTHETIC_KEY_COLUMN: &str = "__csvdb_rowid";

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub pk_values: Vec<String>,
    pub values: Vec<String>,
}

impl Row {
    /// Compute a hash of the row's values for quick comparison.
    pub fn content_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.values.hash(&mut hasher);
        hasher.finish()
    }

    /// Get primary key as a string for use as map key.
    pub fn pk_key(&self) -> String {
        self.pk_values.join("\x00")
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub columns: Vec<String>,
    pub pk_columns: Vec<String>,
    pub rows: Vec<Row>,
}

/// Result of reading a table, including any warnings.
pub struct TableReadResult {
    pub table: Table,
    pub warnings: Vec<String>,
}

impl Table {
    /// Read a table from SQLite database using PK ordering (default).
    pub fn from_sqlite(conn: &Connection, schema: &TableSchema) -> Result<Self> {
        let result = Self::from_sqlite_with_order(conn, schema, OrderMode::Pk, NullMode::Marker)?;
        Ok(result.table)
    }

    /// Read a table from SQLite database with configurable ordering.
    ///
    /// Returns the table and any warnings (e.g., duplicate row warnings for AllColumns mode).
    pub fn from_sqlite_with_order(
        conn: &Connection,
        schema: &TableSchema,
        order_mode: OrderMode,
        null_mode: NullMode,
    ) -> Result<TableReadResult> {
        let mut warnings = Vec::new();

        // Determine columns and ordering based on mode
        let (columns, order_by, include_rowid) = match order_mode {
            OrderMode::Pk => {
                let cols: Vec<String> = schema.columns.iter().map(|c| c.name.clone()).collect();
                let order = if schema.pk_columns.is_empty() {
                    // Fallback to all columns if no PK (shouldn't happen in Pk mode)
                    cols.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ")
                } else {
                    schema.pk_columns.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ")
                };
                (cols, order, false)
            }
            OrderMode::AllColumns => {
                let cols: Vec<String> = schema.columns.iter().map(|c| c.name.clone()).collect();
                let order = cols.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ");
                (cols, order, false)
            }
            OrderMode::AddSyntheticKey => {
                let mut cols: Vec<String> = vec![SYNTHETIC_KEY_COLUMN.to_string()];
                cols.extend(schema.columns.iter().map(|c| c.name.clone()));
                let order = format!("\"{}\"", SYNTHETIC_KEY_COLUMN);
                (cols, order, true)
            }
        };

        // Build query
        let select_cols = if include_rowid {
            let mut parts = vec![format!("rowid AS \"{}\"", SYNTHETIC_KEY_COLUMN)];
            parts.extend(schema.columns.iter().map(|c| format!("\"{}\"", c.name)));
            parts.join(", ")
        } else {
            columns.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ")
        };

        let query = format!(
            "SELECT {} FROM \"{}\" ORDER BY {}",
            select_cols,
            schema.name,
            order_by
        );

        let mut stmt = conn.prepare(&query)?;

        // Determine PK columns for the result
        let (pk_columns, pk_indices) = match order_mode {
            OrderMode::Pk => {
                (schema.pk_columns.clone(), schema.pk_indices())
            }
            OrderMode::AllColumns => {
                // Use all columns as a pseudo-PK for ordering
                let all_indices: Vec<usize> = (0..columns.len()).collect();
                (columns.clone(), all_indices)
            }
            OrderMode::AddSyntheticKey => {
                // The synthetic key is at index 0
                (vec![SYNTHETIC_KEY_COLUMN.to_string()], vec![0])
            }
        };

        let rows: Vec<Row> = stmt
            .query_map([], |row| {
                let values: Vec<String> = (0..columns.len())
                    .map(|i| {
                        let value: rusqlite::types::Value = row.get(i)?;
                        Ok(value_to_string(&value, null_mode))
                    })
                    .collect::<Result<_, rusqlite::Error>>()?;

                let pk_values: Vec<String> = pk_indices
                    .iter()
                    .map(|&i| values[i].clone())
                    .collect();

                Ok(Row { pk_values, values })
            })?
            .collect::<Result<_, _>>()
            .context("Failed to read rows")?;

        // Check for duplicates in AllColumns mode
        if order_mode == OrderMode::AllColumns && !rows.is_empty() {
            let mut seen: HashSet<String> = HashSet::new();
            let mut duplicate_count = 0;
            for row in &rows {
                let key = row.values.join("\x00");
                if !seen.insert(key) {
                    duplicate_count += 1;
                }
            }
            if duplicate_count > 0 {
                warnings.push(format!(
                    "Table '{}' has {} duplicate row(s). \
                     Diffs and merges may be ambiguous. \
                     Consider using --order=add-synthetic-key for event/log tables.",
                    schema.name,
                    duplicate_count
                ));
            }
        }

        let table = Table {
            name: schema.name.clone(),
            columns,
            pk_columns,
            rows,
        };

        Ok(TableReadResult { table, warnings })
    }

    /// Read a table from DuckDB database with configurable ordering.
    pub fn from_duckdb_with_order(
        conn: &DuckDbConnection,
        schema: &TableSchema,
        order_mode: OrderMode,
        null_mode: NullMode,
    ) -> Result<TableReadResult> {
        let mut warnings = Vec::new();

        // Determine columns and ordering based on mode
        let (columns, order_by, include_rowid) = match order_mode {
            OrderMode::Pk => {
                let cols: Vec<String> = schema.columns.iter().map(|c| c.name.clone()).collect();
                let order = if schema.pk_columns.is_empty() {
                    cols.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ")
                } else {
                    schema.pk_columns.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ")
                };
                (cols, order, false)
            }
            OrderMode::AllColumns => {
                let cols: Vec<String> = schema.columns.iter().map(|c| c.name.clone()).collect();
                let order = cols.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ");
                (cols, order, false)
            }
            OrderMode::AddSyntheticKey => {
                let mut cols: Vec<String> = vec![SYNTHETIC_KEY_COLUMN.to_string()];
                cols.extend(schema.columns.iter().map(|c| c.name.clone()));
                let order = format!("\"{}\"", SYNTHETIC_KEY_COLUMN);
                (cols, order, true)
            }
        };

        // Build query - DuckDB uses rowid for synthetic key
        let select_cols = if include_rowid {
            let mut parts = vec![format!("rowid AS \"{}\"", SYNTHETIC_KEY_COLUMN)];
            parts.extend(schema.columns.iter().map(|c| format!("\"{}\"", c.name)));
            parts.join(", ")
        } else {
            columns.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", ")
        };

        let query = format!(
            "SELECT {} FROM \"{}\" ORDER BY {}",
            select_cols,
            schema.name,
            order_by
        );

        let mut stmt = conn.prepare(&query)?;

        // Determine PK columns for the result
        let (pk_columns, pk_indices) = match order_mode {
            OrderMode::Pk => {
                (schema.pk_columns.clone(), schema.pk_indices())
            }
            OrderMode::AllColumns => {
                let all_indices: Vec<usize> = (0..columns.len()).collect();
                (columns.clone(), all_indices)
            }
            OrderMode::AddSyntheticKey => {
                (vec![SYNTHETIC_KEY_COLUMN.to_string()], vec![0])
            }
        };

        let rows: Vec<Row> = stmt
            .query_map([], |row| {
                let values: Vec<String> = (0..columns.len())
                    .map(|i| {
                        let value: duckdb::types::Value = row.get(i)?;
                        Ok(duckdb_value_to_string(&value, null_mode))
                    })
                    .collect::<Result<_, duckdb::Error>>()?;

                let pk_values: Vec<String> = pk_indices
                    .iter()
                    .map(|&i| values[i].clone())
                    .collect();

                Ok(Row { pk_values, values })
            })?
            .collect::<Result<_, _>>()
            .context("Failed to read rows")?;

        // Check for duplicates in AllColumns mode
        if order_mode == OrderMode::AllColumns && !rows.is_empty() {
            let mut seen: HashSet<String> = HashSet::new();
            let mut duplicate_count = 0;
            for row in &rows {
                let key = row.values.join("\x00");
                if !seen.insert(key) {
                    duplicate_count += 1;
                }
            }
            if duplicate_count > 0 {
                warnings.push(format!(
                    "Table '{}' has {} duplicate row(s). \
                     Diffs and merges may be ambiguous. \
                     Consider using --order=add-synthetic-key for event/log tables.",
                    schema.name,
                    duplicate_count
                ));
            }
        }

        let table = Table {
            name: schema.name.clone(),
            columns,
            pk_columns,
            rows,
        };

        Ok(TableReadResult { table, warnings })
    }

    /// Build a map from primary key to row for efficient lookups.
    pub fn rows_by_pk(&self) -> HashMap<String, &Row> {
        self.rows.iter().map(|r| (r.pk_key(), r)).collect()
    }

    /// Write this table's data to a SQLite database using batch inserts.
    pub fn write_to_sqlite(&self, conn: &Connection) -> Result<()> {
        if self.rows.is_empty() {
            return Ok(());
        }

        let columns_str = self.columns
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect::<Vec<_>>()
            .join(", ");

        // SQLite has a limit on variables per query (default 999)
        // Calculate max rows per batch: 999 / columns, but cap at 500 for safety
        let max_rows_per_batch = (999 / self.columns.len()).min(500).max(1);
        let single_row_placeholders = format!("({})", vec!["?"; self.columns.len()].join(", "));

        for chunk in self.rows.chunks(max_rows_per_batch) {
            // Build multi-row INSERT: INSERT INTO t (cols) VALUES (...), (...), (...)
            let values_placeholders = vec![single_row_placeholders.as_str(); chunk.len()].join(", ");
            let sql = format!(
                "INSERT INTO \"{}\" ({}) VALUES {}",
                self.name, columns_str, values_placeholders
            );

            let mut stmt = conn.prepare_cached(&sql)?;

            // Flatten all values into a single params vector, converting \N to NULL
            let all_values: Vec<Option<&str>> = chunk
                .iter()
                .flat_map(|row| row.values.iter().map(|s| {
                    if s == NULL_MARKER {
                        None
                    } else {
                        Some(s.as_str())
                    }
                }))
                .collect();

            let params: Vec<&dyn rusqlite::ToSql> = all_values
                .iter()
                .map(|v| v as &dyn rusqlite::ToSql)
                .collect();

            stmt.execute(params.as_slice())?;
        }

        Ok(())
    }

    /// Write this table's data to a DuckDB database.
    pub fn write_to_duckdb(&self, conn: &DuckDbConnection) -> Result<()> {
        if self.rows.is_empty() {
            return Ok(());
        }

        // Query column types from DuckDB to handle NULL conversion properly
        let type_query = format!(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_name = '{}' ORDER BY ordinal_position",
            self.name
        );
        let mut type_stmt = conn.prepare(&type_query)?;
        let col_types: Vec<(String, String)> = type_stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;

        // Build a map of column name to type
        let type_map: HashMap<String, String> = col_types.into_iter().collect();

        let placeholders = vec!["?"; self.columns.len()].join(", ");
        let columns_str = self.columns
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "INSERT INTO \"{}\" ({}) VALUES ({})",
            self.name, columns_str, placeholders
        );

        let mut stmt = conn.prepare(&sql)?;

        for row in &self.rows {
            // Convert values based on column types, treating empty strings as NULL for numeric types
            let converted: Vec<DuckDbValue> = self.columns
                .iter()
                .zip(row.values.iter())
                .map(|(col_name, val)| {
                    let col_type = type_map.get(col_name).map(|s| s.as_str()).unwrap_or("TEXT");
                    convert_for_duckdb(val, col_type)
                })
                .collect();

            let params: Vec<&dyn duckdb::ToSql> = converted
                .iter()
                .map(|v| v as &dyn duckdb::ToSql)
                .collect();
            stmt.execute(params.as_slice())?;
        }

        Ok(())
    }
}

/// A wrapper enum to handle DuckDB value conversion with proper NULL handling
enum DuckDbValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

impl duckdb::ToSql for DuckDbValue {
    fn to_sql(&self) -> duckdb::Result<duckdb::types::ToSqlOutput<'_>> {
        use duckdb::types::{ToSqlOutput, Value};
        match self {
            DuckDbValue::Null => Ok(ToSqlOutput::Owned(Value::Null)),
            DuckDbValue::Integer(i) => Ok(ToSqlOutput::Owned(Value::BigInt(*i))),
            DuckDbValue::Real(f) => Ok(ToSqlOutput::Owned(Value::Double(*f))),
            DuckDbValue::Text(s) => Ok(ToSqlOutput::Owned(Value::Text(s.clone()))),
        }
    }
}

/// Convert a string value to appropriate DuckDB type based on column type.
/// Converts CSV value to DuckDB value.
/// - `\N` is always interpreted as NULL
/// - Empty strings are preserved as empty strings for text types
fn convert_for_duckdb(value: &str, col_type: &str) -> DuckDbValue {
    // Check for NULL marker first
    if value == NULL_MARKER {
        return DuckDbValue::Null;
    }

    let col_type_upper = col_type.to_uppercase();

    // Check if it's a numeric type
    let is_numeric = col_type_upper.contains("INT")
        || col_type_upper.contains("FLOAT")
        || col_type_upper.contains("DOUBLE")
        || col_type_upper.contains("REAL")
        || col_type_upper.contains("DECIMAL")
        || col_type_upper.contains("NUMERIC");

    if value.is_empty() {
        if is_numeric {
            // Empty string in numeric column is NULL (for backwards compatibility)
            return DuckDbValue::Null;
        } else {
            // For text types, empty string stays as empty string
            return DuckDbValue::Text(String::new());
        }
    }

    // Try to convert based on type
    if col_type_upper.contains("INT") {
        if let Ok(i) = value.parse::<i64>() {
            return DuckDbValue::Integer(i);
        }
    }

    if is_numeric {
        if let Ok(f) = value.parse::<f64>() {
            return DuckDbValue::Real(f);
        }
        // If we can't parse it but it's supposed to be numeric, treat as NULL
        return DuckDbValue::Null;
    }

    // Default to text
    DuckDbValue::Text(value.to_string())
}

fn value_to_string(value: &rusqlite::types::Value, null_mode: NullMode) -> String {
    use rusqlite::types::Value;
    match value {
        Value::Null => null_mode.null_string().to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Text(s) => s.clone(),
        Value::Blob(b) => {
            // Encode blob as hex
            b.iter().map(|byte| format!("{:02x}", byte)).collect()
        }
    }
}

fn duckdb_value_to_string(value: &duckdb::types::Value, null_mode: NullMode) -> String {
    use duckdb::types::Value;
    match value {
        Value::Null => null_mode.null_string().to_string(),
        Value::Boolean(b) => if *b { "1".to_string() } else { "0".to_string() },
        Value::TinyInt(i) => i.to_string(),
        Value::SmallInt(i) => i.to_string(),
        Value::Int(i) => i.to_string(),
        Value::BigInt(i) => i.to_string(),
        Value::HugeInt(i) => i.to_string(),
        Value::UTinyInt(i) => i.to_string(),
        Value::USmallInt(i) => i.to_string(),
        Value::UInt(i) => i.to_string(),
        Value::UBigInt(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Text(s) => s.clone(),
        Value::Blob(b) => {
            // Encode blob as hex
            b.iter().map(|byte| format!("{:02x}", byte)).collect()
        }
        // Handle other types by converting to string representation
        _ => format!("{:?}", value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Schema;

    #[test]
    fn test_table_from_sqlite() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
            [],
        )?;
        conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])?;
        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])?;

        let schema = Schema::from_sqlite(&conn)?;
        let table = Table::from_sqlite(&conn, &schema.tables["users"])?;

        assert_eq!(table.rows.len(), 2);
        // Should be sorted by PK
        assert_eq!(table.rows[0].pk_values, vec!["1"]);
        assert_eq!(table.rows[1].pk_values, vec!["2"]);

        Ok(())
    }

    #[test]
    fn test_rows_by_pk() -> Result<()> {
        let table = Table {
            name: "test".to_string(),
            columns: vec!["id".to_string(), "value".to_string()],
            pk_columns: vec!["id".to_string()],
            rows: vec![
                Row {
                    pk_values: vec!["1".to_string()],
                    values: vec!["1".to_string(), "one".to_string()],
                },
                Row {
                    pk_values: vec!["2".to_string()],
                    values: vec!["2".to_string(), "two".to_string()],
                },
            ],
        };

        let by_pk = table.rows_by_pk();
        assert_eq!(by_pk.get("1").unwrap().values[1], "one");
        assert_eq!(by_pk.get("2").unwrap().values[1], "two");

        Ok(())
    }

    #[test]
    fn test_table_all_columns_ordering() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute("CREATE TABLE events (timestamp TEXT, message TEXT)", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-02', 'second')", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'first')", [])?;

        let schema = Schema::from_sqlite_with_order(&conn, OrderMode::AllColumns)?;
        let result = Table::from_sqlite_with_order(
            &conn,
            &schema.tables["events"],
            OrderMode::AllColumns,
            NullMode::Marker,
        )?;

        assert_eq!(result.table.rows.len(), 2);
        // Should be sorted by all columns (timestamp, then message)
        assert_eq!(result.table.rows[0].values[0], "2024-01-01");
        assert_eq!(result.table.rows[1].values[0], "2024-01-02");
        assert!(result.warnings.is_empty());

        Ok(())
    }

    #[test]
    fn test_table_all_columns_duplicate_warning() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute("CREATE TABLE events (timestamp TEXT, message TEXT)", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'same')", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'same')", [])?;

        let schema = Schema::from_sqlite_with_order(&conn, OrderMode::AllColumns)?;
        let result = Table::from_sqlite_with_order(
            &conn,
            &schema.tables["events"],
            OrderMode::AllColumns,
            NullMode::Marker,
        )?;

        assert_eq!(result.table.rows.len(), 2);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("duplicate"));

        Ok(())
    }

    #[test]
    fn test_table_synthetic_key() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute("CREATE TABLE events (timestamp TEXT, message TEXT)", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-02', 'second')", [])?;
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'first')", [])?;

        let schema = Schema::from_sqlite_with_order(&conn, OrderMode::AddSyntheticKey)?;
        let result = Table::from_sqlite_with_order(
            &conn,
            &schema.tables["events"],
            OrderMode::AddSyntheticKey,
            NullMode::Marker,
        )?;

        assert_eq!(result.table.rows.len(), 2);
        // Should have synthetic key as first column
        assert_eq!(result.table.columns[0], SYNTHETIC_KEY_COLUMN);
        assert_eq!(result.table.pk_columns, vec![SYNTHETIC_KEY_COLUMN]);
        // Rows ordered by insertion order (rowid)
        assert_eq!(result.table.rows[0].values[1], "2024-01-02"); // first inserted
        assert_eq!(result.table.rows[1].values[1], "2024-01-01"); // second inserted

        Ok(())
    }

    #[test]
    fn test_null_marker_export() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
            [],
        )?;
        conn.execute("INSERT INTO test VALUES (1, NULL, NULL)", [])?;
        conn.execute("INSERT INTO test VALUES (2, '', 42)", [])?;
        conn.execute("INSERT INTO test VALUES (3, 'hello', NULL)", [])?;

        let schema = Schema::from_sqlite(&conn)?;
        let table = Table::from_sqlite(&conn, &schema.tables["test"])?;

        // NULL should be exported as \N
        assert_eq!(table.rows[0].values, vec!["1", "\\N", "\\N"]);
        // Empty string should remain empty
        assert_eq!(table.rows[1].values, vec!["2", "", "42"]);
        // Mixed
        assert_eq!(table.rows[2].values, vec!["3", "hello", "\\N"]);

        Ok(())
    }

    #[test]
    fn test_row_content_hash() {
        let row_a = Row {
            pk_values: vec!["1".to_string()],
            values: vec!["1".to_string(), "hello".to_string()],
        };
        let row_a2 = Row {
            pk_values: vec!["1".to_string()],
            values: vec!["1".to_string(), "hello".to_string()],
        };
        let row_b = Row {
            pk_values: vec!["2".to_string()],
            values: vec!["2".to_string(), "world".to_string()],
        };

        // Identical rows produce same hash
        assert_eq!(row_a.content_hash(), row_a2.content_hash());
        // Different rows produce different hashes
        assert_ne!(row_a.content_hash(), row_b.content_hash());
    }

    #[test]
    fn test_row_pk_key() {
        // Single PK
        let row = Row {
            pk_values: vec!["42".to_string()],
            values: vec!["42".to_string(), "data".to_string()],
        };
        assert_eq!(row.pk_key(), "42");

        // Composite PK (joined with \0 separator)
        let row = Row {
            pk_values: vec!["A".to_string(), "B".to_string()],
            values: vec!["A".to_string(), "B".to_string(), "data".to_string()],
        };
        assert_eq!(row.pk_key(), "A\x00B");
    }

    #[test]
    fn test_null_marker_import() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
            [],
        )?;

        // Create a table with \N markers (simulating CSV import)
        let table = Table {
            name: "test".to_string(),
            columns: vec!["id".to_string(), "name".to_string(), "value".to_string()],
            pk_columns: vec!["id".to_string()],
            rows: vec![
                Row {
                    pk_values: vec!["1".to_string()],
                    values: vec!["1".to_string(), "\\N".to_string(), "\\N".to_string()],
                },
                Row {
                    pk_values: vec!["2".to_string()],
                    values: vec!["2".to_string(), "".to_string(), "42".to_string()],
                },
            ],
        };

        table.write_to_sqlite(&conn)?;

        // Verify NULL values were imported correctly
        let mut stmt = conn.prepare("SELECT id, name, value FROM test ORDER BY id")?;
        let rows: Vec<(i64, Option<String>, Option<i64>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Row 1: \N -> NULL
        assert_eq!(rows[0].0, 1);
        assert_eq!(rows[0].1, None); // NULL
        assert_eq!(rows[0].2, None); // NULL

        // Row 2: empty string -> empty string (not NULL)
        assert_eq!(rows[1].0, 2);
        assert_eq!(rows[1].1, Some("".to_string())); // empty string preserved
        assert_eq!(rows[1].2, Some(42));

        Ok(())
    }
}
