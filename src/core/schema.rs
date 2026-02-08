use anyhow::{Context, Result, bail};
use duckdb::Connection as DuckDbConnection;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use crate::OrderMode;

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub col_type: String,
    pub notnull: bool,
    pub default_value: Option<String>,
    pub pk: i32, // 0 if not part of primary key, otherwise position in key
}

#[derive(Debug, Clone)]
pub struct Index {
    pub name: String,
    pub sql: String,
    pub unique: bool,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<Column>,
    pub pk_columns: Vec<String>,
    pub sql: String, // Original CREATE TABLE statement
    pub indexes: Vec<Index>,
}

impl TableSchema {
    pub fn pk_indices(&self) -> Vec<usize> {
        self.pk_columns
            .iter()
            .filter_map(|pk| self.columns.iter().position(|c| &c.name == pk))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct View {
    pub name: String,
    pub sql: String,
}

/// Topologically sort views so that dependencies come before dependents.
///
/// Tokenizes each view's SQL on non-alphanumeric/underscore boundaries and
/// checks tokens against the full set of view names to discover dependencies.
/// Uses Kahn's algorithm (BFS). Returns an error if a cycle is detected.
fn sort_views_by_dependency(views: Vec<View>) -> Result<Vec<View>> {
    if views.len() <= 1 {
        return Ok(views);
    }

    let view_names: HashSet<&str> = views.iter().map(|v| v.name.as_str()).collect();
    let name_to_idx: HashMap<&str, usize> = views.iter().enumerate().map(|(i, v)| (v.name.as_str(), i)).collect();

    // Build adjacency list: deps[i] = set of indices that view i depends on
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); views.len()]; // dependents[dep] = views that depend on dep
    let mut in_degree: Vec<usize> = vec![0; views.len()];

    for (i, view) in views.iter().enumerate() {
        // Tokenize SQL on non-alphanumeric/underscore boundaries
        let tokens: HashSet<&str> = view
            .sql
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| !t.is_empty())
            .collect();

        for token in &tokens {
            if let Some(&dep_idx) = name_to_idx.get(token) {
                if dep_idx != i && view_names.contains(token) {
                    dependents[dep_idx].push(i);
                    in_degree[i] += 1;
                }
            }
        }
    }

    // Kahn's algorithm
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    let mut sorted_indices: Vec<usize> = Vec::with_capacity(views.len());
    while let Some(idx) = queue.pop_front() {
        sorted_indices.push(idx);
        for &dependent in &dependents[idx] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    if sorted_indices.len() != views.len() {
        let cycle_views: Vec<&str> = views
            .iter()
            .enumerate()
            .filter(|(i, _)| in_degree[*i] > 0)
            .map(|(_, v)| v.name.as_str())
            .collect();
        bail!(
            "Circular dependency detected among views: {}",
            cycle_views.join(", ")
        );
    }

    // Rebuild vec in sorted order
    let mut indexed_views: Vec<Option<View>> = views.into_iter().map(Some).collect();
    let sorted: Vec<View> = sorted_indices
        .into_iter()
        .map(|i| indexed_views[i].take().unwrap())
        .collect();

    Ok(sorted)
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub tables: BTreeMap<String, TableSchema>,
    pub views: Vec<View>,
}

impl Schema {
    /// Read schema from SQLite, requiring all tables to have primary keys.
    /// Use `from_sqlite_with_order` for flexible PK handling.
    pub fn from_sqlite(conn: &Connection) -> Result<Self> {
        Self::from_sqlite_with_order(conn, OrderMode::Pk)
    }

    /// Read schema from SQLite with configurable ordering mode.
    ///
    /// - `OrderMode::Pk`: Require all tables to have primary keys (default)
    /// - `OrderMode::AllColumns`: Allow tables without PK, order by all columns
    /// - `OrderMode::AddSyntheticKey`: Allow tables without PK, add synthetic rowid
    pub fn from_sqlite_with_order(conn: &Connection, order_mode: OrderMode) -> Result<Self> {
        let mut tables = BTreeMap::new();
        let mut tables_without_pk = Vec::new();

        // Get all user tables (skip sqlite_ internal tables)
        let mut stmt = conn.prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name"
        )?;

        let table_rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;

        for (table_name, create_sql) in table_rows {
            let table_schema = Self::read_table_schema(conn, &table_name, &create_sql)?;

            if table_schema.pk_columns.is_empty() {
                tables_without_pk.push(table_name.clone());
            }

            tables.insert(table_name, table_schema);
        }

        // Check PK requirement based on order mode
        if order_mode == OrderMode::Pk && !tables_without_pk.is_empty() {
            let table_list = tables_without_pk.join(", ");
            bail!(
                "Table(s) without PRIMARY KEY: {}.\n\n\
                 Use --order=all-columns to sort by all columns, or\n\
                 Use --order=add-synthetic-key to add a __csvdb_rowid column.",
                table_list
            );
        }

        // Get all views
        let mut view_stmt = conn.prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type='view' AND name NOT LIKE 'sqlite_%'
             ORDER BY name"
        )?;

        let views: Vec<View> = view_stmt
            .query_map([], |row| {
                Ok(View {
                    name: row.get(0)?,
                    sql: row.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(Schema { tables, views })
    }

    fn read_table_schema(conn: &Connection, table_name: &str, sql: &str) -> Result<TableSchema> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info('{}')", table_name))?;

        let columns: Vec<Column> = stmt
            .query_map([], |row| {
                Ok(Column {
                    name: row.get(1)?,
                    col_type: row.get(2)?,
                    notnull: row.get(3)?,
                    default_value: row.get(4)?,
                    pk: row.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        // Extract primary key columns (pk > 0, ordered by pk value)
        let mut pk_cols: Vec<_> = columns
            .iter()
            .filter(|c| c.pk > 0)
            .map(|c| (c.pk, c.name.clone()))
            .collect();
        pk_cols.sort_by_key(|(pk, _)| *pk);
        let pk_columns: Vec<String> = pk_cols.into_iter().map(|(_, name)| name).collect();

        // Get indexes for this table
        let mut idx_stmt = conn.prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type='index' AND tbl_name=? AND sql IS NOT NULL
             ORDER BY name"
        )?;

        let indexes: Vec<Index> = idx_stmt
            .query_map([table_name], |row| {
                let sql: String = row.get(1)?;
                let (unique, columns) = parse_index_sql(&sql);
                Ok(Index {
                    name: row.get(0)?,
                    sql,
                    unique,
                    columns,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(TableSchema {
            name: table_name.to_string(),
            columns,
            pk_columns,
            sql: sql.to_string(),
            indexes,
        })
    }

    /// Read schema from DuckDB, requiring all tables to have primary keys.
    pub fn from_duckdb(conn: &DuckDbConnection) -> Result<Self> {
        Self::from_duckdb_with_order(conn, OrderMode::Pk)
    }

    /// Read schema from DuckDB with configurable ordering mode.
    pub fn from_duckdb_with_order(conn: &DuckDbConnection, order_mode: OrderMode) -> Result<Self> {
        let mut tables = BTreeMap::new();
        let mut tables_without_pk = Vec::new();

        // Get all user tables from DuckDB
        let mut stmt = conn.prepare(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = 'main' AND table_type = 'BASE TABLE'
             ORDER BY table_name"
        )?;

        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        for table_name in table_names {
            let table_schema = Self::read_duckdb_table_schema(conn, &table_name)?;

            if table_schema.pk_columns.is_empty() {
                tables_without_pk.push(table_name.clone());
            }

            tables.insert(table_name, table_schema);
        }

        // Check PK requirement based on order mode
        if order_mode == OrderMode::Pk && !tables_without_pk.is_empty() {
            let table_list = tables_without_pk.join(", ");
            bail!(
                "Table(s) without PRIMARY KEY: {}.\n\n\
                 Use --order=all-columns to sort by all columns, or\n\
                 Use --order=add-synthetic-key to add a __csvdb_rowid column.",
                table_list
            );
        }

        // Get all views from DuckDB (excluding system views)
        let mut view_stmt = conn.prepare(
            "SELECT view_name, sql FROM duckdb_views()
             WHERE schema_name = 'main'
             ORDER BY view_name"
        )?;

        let views: Vec<View> = view_stmt
            .query_map([], |row| {
                Ok(View {
                    name: row.get(0)?,
                    sql: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                })
            })?
            .filter_map(|r| r.ok())
            .filter(|v| !v.sql.is_empty())
            // Filter out DuckDB system views
            .filter(|v| !v.name.starts_with("duckdb_")
                     && !v.name.starts_with("pragma_")
                     && !v.name.starts_with("sqlite_"))
            .collect();

        Ok(Schema { tables, views })
    }

    fn read_duckdb_table_schema(conn: &DuckDbConnection, table_name: &str) -> Result<TableSchema> {
        // Get column info from information_schema
        let mut col_stmt = conn.prepare(
            "SELECT column_name, data_type, is_nullable, column_default
             FROM information_schema.columns
             WHERE table_schema = 'main' AND table_name = ?
             ORDER BY ordinal_position"
        )?;

        let columns: Vec<Column> = col_stmt
            .query_map([table_name], |row| {
                let is_nullable: String = row.get(2)?;
                Ok(Column {
                    name: row.get(0)?,
                    col_type: row.get(1)?,
                    notnull: is_nullable == "NO",
                    default_value: row.get(3)?,
                    pk: 0, // Will be updated below
                })
            })?
            .collect::<Result<_, _>>()?;

        // Get primary key columns
        let mut pk_stmt = conn.prepare(
            "SELECT kcu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
             WHERE tc.table_schema = 'main'
               AND tc.table_name = ?
               AND tc.constraint_type = 'PRIMARY KEY'
             ORDER BY kcu.ordinal_position"
        )?;

        let pk_columns: Vec<String> = pk_stmt
            .query_map([table_name], |row| row.get(0))?
            .collect::<Result<_, _>>()
            .unwrap_or_default();

        // Update pk field in columns
        let columns: Vec<Column> = columns
            .into_iter()
            .map(|mut c| {
                if let Some(pos) = pk_columns.iter().position(|pk| pk == &c.name) {
                    c.pk = (pos + 1) as i32;
                }
                c
            })
            .collect();

        // Generate CREATE TABLE statement
        let sql = Self::generate_create_table_sql(table_name, &columns, &pk_columns);

        // Get indexes (DuckDB doesn't have a simple index metadata query, so we skip for now)
        let indexes = Vec::new();

        Ok(TableSchema {
            name: table_name.to_string(),
            columns,
            pk_columns,
            sql,
            indexes,
        })
    }

    /// Generate a CREATE TABLE statement from column metadata.
    fn generate_create_table_sql(table_name: &str, columns: &[Column], pk_columns: &[String]) -> String {
        let mut sql = format!("CREATE TABLE \"{}\" (\n", table_name);

        for (i, col) in columns.iter().enumerate() {
            sql.push_str(&format!("    \"{}\" {}", col.name, col.col_type));

            if col.notnull && !pk_columns.contains(&col.name) {
                sql.push_str(" NOT NULL");
            }

            if let Some(ref default) = col.default_value {
                if !default.is_empty() {
                    sql.push_str(&format!(" DEFAULT {}", default));
                }
            }

            if i < columns.len() - 1 || !pk_columns.is_empty() {
                sql.push(',');
            }
            sql.push('\n');
        }

        if !pk_columns.is_empty() {
            let pk_list = pk_columns
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!("    PRIMARY KEY ({})\n", pk_list));
        }

        sql.push(')');
        sql
    }

    pub fn write_schema_sql(&self, path: &Path) -> Result<()> {
        let mut content = String::new();

        // Write tables and their indexes
        for (i, table) in self.tables.values().enumerate() {
            if i > 0 {
                content.push_str("\n");
            }
            content.push_str(&table.sql);
            content.push_str(";\n");

            for index in &table.indexes {
                content.push_str(&index.sql);
                content.push_str(";\n");
            }
        }

        // Write views at the end, sorted so dependencies come first
        let sorted_views = sort_views_by_dependency(self.views.clone())?;
        for view in &sorted_views {
            content.push_str("\n");
            content.push_str(&view.sql);
            content.push_str(";\n");
        }

        fs::write(path, content).context("Failed to write schema.sql")?;
        Ok(())
    }

    /// Read schema from schema.sql file.
    /// This does NOT require primary keys since the CSV data is already ordered.
    pub fn from_schema_sql(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).context("Failed to read schema.sql")?;

        // Create in-memory database to parse schema
        let conn = Connection::open_in_memory()?;

        // Split on semicolons and execute each statement
        for stmt in content.split(';') {
            let stmt = stmt.trim();
            if !stmt.is_empty() {
                conn.execute(stmt, [])
                    .with_context(|| format!("Failed to execute: {}", stmt))?;
            }
        }

        // Don't require PKs when importing from csvdb - data is already ordered
        Self::from_sqlite_with_order(&conn, OrderMode::AllColumns)
    }
}

/// Parse index SQL to extract unique flag and column names.
/// Works with SQLite-style CREATE INDEX statements.
pub fn parse_index_sql(sql: &str) -> (bool, Vec<String>) {
    let sql_upper = sql.to_uppercase();
    let unique = sql_upper.contains("UNIQUE");

    // Extract columns from parentheses: CREATE INDEX ... ON table (col1, col2)
    let columns = if let Some(start) = sql.rfind('(') {
        if let Some(end) = sql.rfind(')') {
            sql[start + 1..end]
                .split(',')
                .map(|s| {
                    // Remove quotes and whitespace
                    s.trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .trim_matches('`')
                        .trim_matches('[')
                        .trim_matches(']')
                        .to_string()
                })
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    (unique, columns)
}

/// Normalize view SQL for cross-database comparison.
/// Collapses whitespace, uppercases, and removes identifier quotes.
pub fn normalize_view_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
        .replace('"', "")
        .replace('\'', "")
        .replace('`', "")
        .replace('[', "")
        .replace(']', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_from_sqlite() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT
            )",
            [],
        )?;

        let schema = Schema::from_sqlite(&conn)?;
        assert!(schema.tables.contains_key("users"));

        let users = &schema.tables["users"];
        assert_eq!(users.pk_columns, vec!["id"]);
        assert_eq!(users.columns.len(), 3);

        Ok(())
    }

    #[test]
    fn test_missing_pk_error_with_pk_mode() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE no_pk (name TEXT, value INTEGER)", []).unwrap();

        let result = Schema::from_sqlite(&conn);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("no_pk"));
        assert!(err_msg.contains("--order=all-columns"));
    }

    #[test]
    fn test_missing_pk_allowed_with_all_columns_mode() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE no_pk (name TEXT, value INTEGER)", []).unwrap();

        let result = Schema::from_sqlite_with_order(&conn, OrderMode::AllColumns);
        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_pk_allowed_with_synthetic_key_mode() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE no_pk (name TEXT, value INTEGER)", []).unwrap();

        let result = Schema::from_sqlite_with_order(&conn, OrderMode::AddSyntheticKey);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_index_sql() {
        // Regular index
        let (unique, cols) = parse_index_sql("CREATE INDEX idx_name ON users (name)");
        assert!(!unique);
        assert_eq!(cols, vec!["name"]);

        // Unique index
        let (unique, cols) = parse_index_sql("CREATE UNIQUE INDEX idx_email ON users (email)");
        assert!(unique);
        assert_eq!(cols, vec!["email"]);

        // Multi-column index
        let (unique, cols) = parse_index_sql("CREATE INDEX idx_multi ON orders (user_id, product_id)");
        assert!(!unique);
        assert_eq!(cols, vec!["user_id", "product_id"]);

        // Quoted column names
        let (unique, cols) = parse_index_sql("CREATE INDEX idx_q ON t (\"col1\", \"col2\")");
        assert!(!unique);
        assert_eq!(cols, vec!["col1", "col2"]);
    }

    #[test]
    fn test_parse_index_sql_edge_cases() {
        // No parens returns empty columns
        let (_, cols) = parse_index_sql("CREATE INDEX idx_name ON users");
        assert!(cols.is_empty());

        // Empty SQL
        let (unique, cols) = parse_index_sql("");
        assert!(!unique);
        assert!(cols.is_empty());
    }

    #[test]
    fn test_normalize_view_sql() {
        // Collapses whitespace
        let result = normalize_view_sql("SELECT  id,  name   FROM   users");
        assert_eq!(result, "SELECT ID, NAME FROM USERS");

        // Uppercases
        let result = normalize_view_sql("select id from users");
        assert_eq!(result, "SELECT ID FROM USERS");

        // Strips all quote styles
        let result = normalize_view_sql("SELECT \"id\", 'name', `value`, [col] FROM t");
        assert_eq!(result, "SELECT ID, NAME, VALUE, COL FROM T");

        // Newlines/tabs collapsed
        let result = normalize_view_sql("SELECT\n  id\nFROM\n  users");
        assert_eq!(result, "SELECT ID FROM USERS");
    }

    #[test]
    fn test_sort_views_by_dependency() {
        // a_derived depends on z_base → z_base should come first
        let views = vec![
            View {
                name: "a_derived".to_string(),
                sql: "CREATE VIEW a_derived AS SELECT * FROM z_base WHERE val > 10".to_string(),
            },
            View {
                name: "z_base".to_string(),
                sql: "CREATE VIEW z_base AS SELECT * FROM some_table".to_string(),
            },
        ];
        let sorted = sort_views_by_dependency(views).unwrap();
        assert_eq!(sorted[0].name, "z_base");
        assert_eq!(sorted[1].name, "a_derived");
    }

    #[test]
    fn test_sort_views_deep_chain() {
        // c depends on b, b depends on a → a, b, c
        let views = vec![
            View {
                name: "c_view".to_string(),
                sql: "CREATE VIEW c_view AS SELECT * FROM b_view".to_string(),
            },
            View {
                name: "a_view".to_string(),
                sql: "CREATE VIEW a_view AS SELECT * FROM some_table".to_string(),
            },
            View {
                name: "b_view".to_string(),
                sql: "CREATE VIEW b_view AS SELECT * FROM a_view".to_string(),
            },
        ];
        let sorted = sort_views_by_dependency(views).unwrap();
        assert_eq!(sorted[0].name, "a_view");
        assert_eq!(sorted[1].name, "b_view");
        assert_eq!(sorted[2].name, "c_view");
    }

    #[test]
    fn test_sort_views_circular() {
        // a depends on b, b depends on a → error
        let views = vec![
            View {
                name: "view_a".to_string(),
                sql: "CREATE VIEW view_a AS SELECT * FROM view_b".to_string(),
            },
            View {
                name: "view_b".to_string(),
                sql: "CREATE VIEW view_b AS SELECT * FROM view_a".to_string(),
            },
        ];
        let result = sort_views_by_dependency(views);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Circular dependency"));
        assert!(err.contains("view_a"));
        assert!(err.contains("view_b"));
    }

    #[test]
    fn test_sort_views_no_deps() {
        // Independent views — order should be preserved (stable)
        let views = vec![
            View {
                name: "alpha".to_string(),
                sql: "CREATE VIEW alpha AS SELECT * FROM table1".to_string(),
            },
            View {
                name: "beta".to_string(),
                sql: "CREATE VIEW beta AS SELECT * FROM table2".to_string(),
            },
            View {
                name: "gamma".to_string(),
                sql: "CREATE VIEW gamma AS SELECT * FROM table3".to_string(),
            },
        ];
        let sorted = sort_views_by_dependency(views).unwrap();
        assert_eq!(sorted[0].name, "alpha");
        assert_eq!(sorted[1].name, "beta");
        assert_eq!(sorted[2].name, "gamma");
    }
}
