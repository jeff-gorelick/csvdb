use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

use crate::commands::checksum::normalize_value;
use crate::commands::diff::{load_tables, pk_to_map};
use crate::core::csv::write_table_csv_sorted;
use crate::core::{Row, Schema, Table};
use crate::TableFilter;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MergeResult {
    pub base: String,
    pub left: String,
    pub right: String,
    pub has_conflicts: bool,
    pub tables: Vec<TableMerge>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TableMerge {
    pub name: String,
    pub status: MergeTableStatus,
    pub rows_merged: usize,
    pub conflicts: Vec<MergeConflict>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeTableStatus {
    Identical,
    Merged,
    Conflict,
    Added,
    Removed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MergeConflict {
    pub pk: BTreeMap<String, String>,
    pub kind: ConflictKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<ColumnConflict>>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    ModifyModify,
    ModifyDelete,
    AddAdd,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ColumnConflict {
    pub column: String,
    pub base: String,
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum MergeStrategy {
    #[default]
    Normal,
    Ours,
    Theirs,
}

/// Compare two rows' values using normalized comparison (handles float formatting).
fn values_equal(a: &[String], b: &[String]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(av, bv)| normalize_value(av) == normalize_value(bv))
}

/// Merge rows from three versions of a table.
fn merge_rows(
    base_table: &Table,
    left_table: &Table,
    right_table: &Table,
    strategy: MergeStrategy,
) -> (Vec<Row>, Vec<MergeConflict>) {
    let base_by_pk = base_table.rows_by_pk();
    let left_by_pk = left_table.rows_by_pk();
    let right_by_pk = right_table.rows_by_pk();

    let mut all_pks: Vec<String> = base_by_pk
        .keys()
        .chain(left_by_pk.keys())
        .chain(right_by_pk.keys())
        .cloned()
        .collect();
    all_pks.sort();
    all_pks.dedup();

    let mut merged_rows = Vec::new();
    let mut conflicts = Vec::new();

    for pk in &all_pks {
        let in_base = base_by_pk.get(pk);
        let in_left = left_by_pk.get(pk);
        let in_right = right_by_pk.get(pk);

        match (in_base, in_left, in_right) {
            // In all three
            (Some(base_row), Some(left_row), Some(right_row)) => {
                let left_changed = !values_equal(&base_row.values, &left_row.values);
                let right_changed = !values_equal(&base_row.values, &right_row.values);

                match (left_changed, right_changed) {
                    (false, false) => {
                        // Unchanged on both sides
                        merged_rows.push((*base_row).clone());
                    }
                    (true, false) => {
                        // Changed on left only
                        merged_rows.push((*left_row).clone());
                    }
                    (false, true) => {
                        // Changed on right only
                        merged_rows.push((*right_row).clone());
                    }
                    (true, true) => {
                        // Both changed
                        if values_equal(&left_row.values, &right_row.values) {
                            // Converged to same value
                            merged_rows.push((*left_row).clone());
                        } else {
                            // Conflict: modify/modify
                            match strategy {
                                MergeStrategy::Ours => {
                                    merged_rows.push((*left_row).clone());
                                }
                                MergeStrategy::Theirs => {
                                    merged_rows.push((*right_row).clone());
                                }
                                MergeStrategy::Normal => {
                                    // Report conflict, include left version
                                    merged_rows.push((*left_row).clone());

                                    let col_conflicts: Vec<ColumnConflict> = base_row
                                        .values
                                        .iter()
                                        .zip(left_row.values.iter())
                                        .zip(right_row.values.iter())
                                        .enumerate()
                                        .filter(|(_, ((bv, lv), rv))| {
                                            normalize_value(bv) != normalize_value(lv)
                                                || normalize_value(bv) != normalize_value(rv)
                                        })
                                        .map(|(i, ((bv, lv), rv))| ColumnConflict {
                                            column: left_table
                                                .columns
                                                .get(i)
                                                .cloned()
                                                .unwrap_or_else(|| "?".to_string()),
                                            base: bv.clone(),
                                            left: lv.clone(),
                                            right: rv.clone(),
                                        })
                                        .collect();

                                    conflicts.push(MergeConflict {
                                        pk: pk_to_map(pk, &left_table.pk_columns),
                                        kind: ConflictKind::ModifyModify,
                                        columns: Some(col_conflicts),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // Deleted on left, still in right
            (Some(base_row), None, Some(right_row)) => {
                let right_changed = !values_equal(&base_row.values, &right_row.values);
                if right_changed {
                    // Conflict: left deleted, right modified
                    match strategy {
                        MergeStrategy::Ours => {
                            // Ours deleted it, so don't include
                        }
                        MergeStrategy::Theirs => {
                            merged_rows.push((*right_row).clone());
                        }
                        MergeStrategy::Normal => {
                            conflicts.push(MergeConflict {
                                pk: pk_to_map(pk, &right_table.pk_columns),
                                kind: ConflictKind::ModifyDelete,
                                columns: None,
                            });
                        }
                    }
                }
                // else: left deleted, right unchanged → delete (don't include)
            }

            // Deleted on right, still in left
            (Some(base_row), Some(left_row), None) => {
                let left_changed = !values_equal(&base_row.values, &left_row.values);
                if left_changed {
                    // Conflict: right deleted, left modified
                    match strategy {
                        MergeStrategy::Ours => {
                            merged_rows.push((*left_row).clone());
                        }
                        MergeStrategy::Theirs => {
                            // Theirs deleted it, so don't include
                        }
                        MergeStrategy::Normal => {
                            conflicts.push(MergeConflict {
                                pk: pk_to_map(pk, &left_table.pk_columns),
                                kind: ConflictKind::ModifyDelete,
                                columns: None,
                            });
                        }
                    }
                }
                // else: right deleted, left unchanged → delete (don't include)
            }

            // Deleted on both sides
            (Some(_), None, None) => {
                // Both deleted → don't include
            }

            // Added on left only
            (None, Some(left_row), None) => {
                merged_rows.push((*left_row).clone());
            }

            // Added on right only
            (None, None, Some(right_row)) => {
                merged_rows.push((*right_row).clone());
            }

            // Added on both sides
            (None, Some(left_row), Some(right_row)) => {
                if values_equal(&left_row.values, &right_row.values) {
                    // Same values → dedup
                    merged_rows.push((*left_row).clone());
                } else {
                    // Conflict: add/add with different values
                    match strategy {
                        MergeStrategy::Ours => {
                            merged_rows.push((*left_row).clone());
                        }
                        MergeStrategy::Theirs => {
                            merged_rows.push((*right_row).clone());
                        }
                        MergeStrategy::Normal => {
                            merged_rows.push((*left_row).clone());
                            conflicts.push(MergeConflict {
                                pk: pk_to_map(pk, &left_table.pk_columns),
                                kind: ConflictKind::AddAdd,
                                columns: None,
                            });
                        }
                    }
                }
            }

            // Not in any (shouldn't happen since we collected all PKs)
            (None, None, None) => {}
        }
    }

    (merged_rows, conflicts)
}

/// Collect detailed merge data without writing anything.
pub fn merge_detail(
    base_path: &Path,
    left_path: &Path,
    right_path: &Path,
    strategy: MergeStrategy,
    filter: &TableFilter,
) -> Result<(MergeResult, Schema, BTreeMap<String, Table>)> {
    let (base_schema, base_tables) = load_tables(base_path)
        .with_context(|| format!("Failed to load base: {}", base_path.display()))?;
    let (left_schema, left_tables) = load_tables(left_path)
        .with_context(|| format!("Failed to load left: {}", left_path.display()))?;
    let (_right_schema, right_tables) = load_tables(right_path)
        .with_context(|| format!("Failed to load right: {}", right_path.display()))?;

    let mut all_table_names: Vec<String> = base_schema
        .tables
        .keys()
        .chain(left_schema.tables.keys())
        .chain(_right_schema.tables.keys())
        .cloned()
        .collect();
    all_table_names.sort();
    all_table_names.dedup();
    all_table_names.retain(|t| filter.matches(t));

    let mut has_conflicts = false;
    let mut table_merges = Vec::new();
    let mut merged_tables = BTreeMap::new();

    for table_name in &all_table_names {
        let in_base = base_tables.contains_key(table_name);
        let in_left = left_tables.contains_key(table_name);
        let in_right = right_tables.contains_key(table_name);

        match (in_base, in_left, in_right) {
            // Table in all three → merge rows
            (true, true, true) => {
                let base_table = &base_tables[table_name];
                let left_table = &left_tables[table_name];
                let right_table = &right_tables[table_name];

                let (rows, conflicts) = merge_rows(base_table, left_table, right_table, strategy);

                let status = if !conflicts.is_empty() {
                    has_conflicts = true;
                    MergeTableStatus::Conflict
                } else if rows.len() == base_table.rows.len()
                    && values_equal_tables(base_table, &rows)
                {
                    MergeTableStatus::Identical
                } else {
                    MergeTableStatus::Merged
                };

                let rows_merged = rows.len();
                let merged_table = Table {
                    name: table_name.clone(),
                    columns: left_table.columns.clone(),
                    pk_columns: left_table.pk_columns.clone(),
                    rows,
                };
                merged_tables.insert(table_name.clone(), merged_table);

                table_merges.push(TableMerge {
                    name: table_name.clone(),
                    status,
                    rows_merged,
                    conflicts,
                });
            }

            // New table added on left only
            (false, true, false) => {
                let left_table = &left_tables[table_name];
                let rows_merged = left_table.rows.len();
                merged_tables.insert(table_name.clone(), left_table.clone());
                table_merges.push(TableMerge {
                    name: table_name.clone(),
                    status: MergeTableStatus::Added,
                    rows_merged,
                    conflicts: vec![],
                });
            }

            // New table added on right only
            (false, false, true) => {
                let right_table = &right_tables[table_name];
                let rows_merged = right_table.rows.len();
                merged_tables.insert(table_name.clone(), right_table.clone());
                table_merges.push(TableMerge {
                    name: table_name.clone(),
                    status: MergeTableStatus::Added,
                    rows_merged,
                    conflicts: vec![],
                });
            }

            // New table added on both sides
            (false, true, true) => {
                let left_table = &left_tables[table_name];
                let right_table = &right_tables[table_name];

                // Create an empty base table with same columns for merge
                let empty_base = Table {
                    name: table_name.clone(),
                    columns: left_table.columns.clone(),
                    pk_columns: left_table.pk_columns.clone(),
                    rows: vec![],
                };

                let (rows, conflicts) = merge_rows(&empty_base, left_table, right_table, strategy);

                let status = if !conflicts.is_empty() {
                    has_conflicts = true;
                    MergeTableStatus::Conflict
                } else {
                    MergeTableStatus::Added
                };

                let rows_merged = rows.len();
                let merged_table = Table {
                    name: table_name.clone(),
                    columns: left_table.columns.clone(),
                    pk_columns: left_table.pk_columns.clone(),
                    rows,
                };
                merged_tables.insert(table_name.clone(), merged_table);

                table_merges.push(TableMerge {
                    name: table_name.clone(),
                    status,
                    rows_merged,
                    conflicts,
                });
            }

            // Table removed on left, still on right
            (true, false, true) => {
                let base_table = &base_tables[table_name];
                let right_table = &right_tables[table_name];
                let right_changed = !values_equal_tables(base_table, &right_table.rows);

                if right_changed {
                    // Conflict: left removed, right modified
                    has_conflicts = true;
                    match strategy {
                        MergeStrategy::Ours => {
                            // Ours removed it
                            table_merges.push(TableMerge {
                                name: table_name.clone(),
                                status: MergeTableStatus::Removed,
                                rows_merged: 0,
                                conflicts: vec![],
                            });
                        }
                        MergeStrategy::Theirs => {
                            // Theirs kept it
                            merged_tables.insert(table_name.clone(), right_table.clone());
                            table_merges.push(TableMerge {
                                name: table_name.clone(),
                                status: MergeTableStatus::Merged,
                                rows_merged: right_table.rows.len(),
                                conflicts: vec![],
                            });
                        }
                        MergeStrategy::Normal => {
                            // Report as removed with conflict
                            table_merges.push(TableMerge {
                                name: table_name.clone(),
                                status: MergeTableStatus::Conflict,
                                rows_merged: 0,
                                conflicts: vec![MergeConflict {
                                    pk: BTreeMap::new(),
                                    kind: ConflictKind::ModifyDelete,
                                    columns: None,
                                }],
                            });
                        }
                    }
                } else {
                    // Left removed, right unchanged → remove
                    table_merges.push(TableMerge {
                        name: table_name.clone(),
                        status: MergeTableStatus::Removed,
                        rows_merged: 0,
                        conflicts: vec![],
                    });
                }
            }

            // Table removed on right, still on left
            (true, true, false) => {
                let base_table = &base_tables[table_name];
                let left_table = &left_tables[table_name];
                let left_changed = !values_equal_tables(base_table, &left_table.rows);

                if left_changed {
                    // Conflict: right removed, left modified
                    has_conflicts = true;
                    match strategy {
                        MergeStrategy::Ours => {
                            // Ours kept it
                            merged_tables.insert(table_name.clone(), left_table.clone());
                            table_merges.push(TableMerge {
                                name: table_name.clone(),
                                status: MergeTableStatus::Merged,
                                rows_merged: left_table.rows.len(),
                                conflicts: vec![],
                            });
                        }
                        MergeStrategy::Theirs => {
                            // Theirs removed it
                            table_merges.push(TableMerge {
                                name: table_name.clone(),
                                status: MergeTableStatus::Removed,
                                rows_merged: 0,
                                conflicts: vec![],
                            });
                        }
                        MergeStrategy::Normal => {
                            table_merges.push(TableMerge {
                                name: table_name.clone(),
                                status: MergeTableStatus::Conflict,
                                rows_merged: 0,
                                conflicts: vec![MergeConflict {
                                    pk: BTreeMap::new(),
                                    kind: ConflictKind::ModifyDelete,
                                    columns: None,
                                }],
                            });
                        }
                    }
                } else {
                    // Right removed, left unchanged → remove
                    table_merges.push(TableMerge {
                        name: table_name.clone(),
                        status: MergeTableStatus::Removed,
                        rows_merged: 0,
                        conflicts: vec![],
                    });
                }
            }

            // Both removed
            (true, false, false) => {
                table_merges.push(TableMerge {
                    name: table_name.clone(),
                    status: MergeTableStatus::Removed,
                    rows_merged: 0,
                    conflicts: vec![],
                });
            }

            // Not in any (shouldn't happen)
            (false, false, false) => {}
        }
    }

    let result = MergeResult {
        base: base_path.display().to_string(),
        left: left_path.display().to_string(),
        right: right_path.display().to_string(),
        has_conflicts,
        tables: table_merges,
    };

    // Use left schema as the basis for output
    Ok((result, left_schema, merged_tables))
}

/// Check if a base table's rows match the given merged rows.
fn values_equal_tables(base_table: &Table, rows: &[Row]) -> bool {
    if base_table.rows.len() != rows.len() {
        return false;
    }
    let base_by_pk = base_table.rows_by_pk();
    for row in rows {
        let pk = row.pk_key();
        match base_by_pk.get(&pk) {
            Some(base_row) => {
                if !values_equal(&base_row.values, &row.values) {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

/// Merge three databases and write the result to an output directory.
pub fn merge(
    base_path: &Path,
    left_path: &Path,
    right_path: &Path,
    output_path: &Path,
    strategy: MergeStrategy,
    force: bool,
    filter: &TableFilter,
) -> Result<MergeResult> {
    if output_path.exists() {
        if force {
            std::fs::remove_dir_all(output_path)?;
        } else {
            bail!(
                "Output directory already exists: {}. Use --force to overwrite.",
                output_path.display()
            );
        }
    }

    let (result, schema, merged_tables) =
        merge_detail(base_path, left_path, right_path, strategy, filter)?;

    // Write output directory
    std::fs::create_dir_all(output_path)?;

    // Build a filtered schema containing only merged tables
    let filtered_schema = Schema {
        tables: schema
            .tables
            .iter()
            .filter(|(name, _)| merged_tables.contains_key(*name))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        views: vec![],
    };
    filtered_schema.write_schema_sql(&output_path.join("schema.sql"))?;

    // Write CSV files
    for (name, table) in &merged_tables {
        let csv_path = output_path.join(format!("{name}.csv"));
        write_table_csv_sorted(table, &csv_path, false)?;
    }

    Ok(result)
}

/// Merge three databases and return the result as a JSON string.
pub fn merge_json(
    base_path: &Path,
    left_path: &Path,
    right_path: &Path,
    strategy: MergeStrategy,
    filter: &TableFilter,
) -> Result<String> {
    let (result, _, _) = merge_detail(base_path, left_path, right_path, strategy, filter)?;
    serde_json::to_string_pretty(&result).map_err(Into::into)
}

/// Print a MergeResult in human-readable text format.
pub fn print_text_merge(result: &MergeResult) {
    println!(
        "Merging {} (base) + {} (left) + {} (right)",
        result.base, result.left, result.right
    );
    println!();

    for table in &result.tables {
        match table.status {
            MergeTableStatus::Identical => {
                println!("{}: identical", table.name);
            }
            MergeTableStatus::Merged => {
                println!("{}: merged ({} rows)", table.name, table.rows_merged);
            }
            MergeTableStatus::Added => {
                println!("{}: added ({} rows)", table.name, table.rows_merged);
            }
            MergeTableStatus::Removed => {
                println!("{}: removed", table.name);
            }
            MergeTableStatus::Conflict => {
                println!(
                    "{}: CONFLICT ({} conflict{})",
                    table.name,
                    table.conflicts.len(),
                    if table.conflicts.len() == 1 { "" } else { "s" }
                );
                for conflict in &table.conflicts {
                    let pk_display: String = conflict
                        .pk
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    match &conflict.kind {
                        ConflictKind::ModifyModify => {
                            println!("  ! ({pk_display}) both modified");
                            if let Some(cols) = &conflict.columns {
                                for col in cols {
                                    println!(
                                        "    {}: base=\"{}\" left=\"{}\" right=\"{}\"",
                                        col.column, col.base, col.left, col.right
                                    );
                                }
                            }
                        }
                        ConflictKind::ModifyDelete => {
                            if pk_display.is_empty() {
                                println!("  ! table-level modify/delete conflict");
                            } else {
                                println!("  ! ({pk_display}) modify/delete conflict");
                            }
                        }
                        ConflictKind::AddAdd => {
                            println!("  ! ({pk_display}) both added with different values");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Helper: create a minimal csvdb dir.
    fn make_csvdb(base: &Path, name: &str, schema_sql: &str, csvs: &[(&str, &str)]) -> PathBuf {
        let csvdb_dir = base.join(name);
        fs::create_dir_all(&csvdb_dir).unwrap();
        fs::write(csvdb_dir.join("schema.sql"), schema_sql).unwrap();
        for (table_name, content) in csvs {
            fs::write(csvdb_dir.join(format!("{table_name}.csv")), content).unwrap();
        }
        csvdb_dir
    }

    const SCHEMA: &str =
        "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n";

    fn no_filter() -> TableFilter {
        TableFilter::new(vec![], vec![])
    }

    #[test]
    fn test_merge_identical() -> Result<()> {
        let dir = tempdir()?;
        let csv = "id,name\n1,Alice\n2,Bob\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", csv)]);

        let (result, _, _) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        assert_eq!(result.tables.len(), 1);
        assert!(matches!(
            result.tables[0].status,
            MergeTableStatus::Identical
        ));
        Ok(())
    }

    #[test]
    fn test_merge_left_only_change() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n2,Bob\n";
        let left_csv = "id,name\n1,Alicia\n2,Bob\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", base_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        assert!(matches!(result.tables[0].status, MergeTableStatus::Merged));
        let merged = &tables["t"];
        let by_pk = merged.rows_by_pk();
        assert_eq!(by_pk["1"].values[1], "Alicia");
        Ok(())
    }

    #[test]
    fn test_merge_right_only_change() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n2,Bob\n";
        let right_csv = "id,name\n1,Alice\n2,Bobby\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", base_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        let by_pk = merged.rows_by_pk();
        assert_eq!(by_pk["2"].values[1], "Bobby");
        Ok(())
    }

    #[test]
    fn test_merge_both_same_change() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let changed_csv = "id,name\n1,Alicia\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", changed_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", changed_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        let by_pk = merged.rows_by_pk();
        assert_eq!(by_pk["1"].values[1], "Alicia");
        Ok(())
    }

    #[test]
    fn test_merge_modify_modify_conflict() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let left_csv = "id,name\n1,Alicia\n";
        let right_csv = "id,name\n1,Ally\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, _) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(result.has_conflicts);
        assert!(matches!(
            result.tables[0].status,
            MergeTableStatus::Conflict
        ));
        assert_eq!(result.tables[0].conflicts.len(), 1);
        assert!(matches!(
            result.tables[0].conflicts[0].kind,
            ConflictKind::ModifyModify
        ));
        Ok(())
    }

    #[test]
    fn test_merge_left_add() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let left_csv = "id,name\n1,Alice\n2,Bob\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", base_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        assert_eq!(merged.rows.len(), 2);
        Ok(())
    }

    #[test]
    fn test_merge_right_add() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let right_csv = "id,name\n1,Alice\n3,Charlie\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", base_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        assert_eq!(merged.rows.len(), 2);
        Ok(())
    }

    #[test]
    fn test_merge_both_add_same() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let added_csv = "id,name\n1,Alice\n2,Bob\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", added_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", added_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        assert_eq!(merged.rows.len(), 2);
        Ok(())
    }

    #[test]
    fn test_merge_both_add_different() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let left_csv = "id,name\n1,Alice\n2,Bob\n";
        let right_csv = "id,name\n1,Alice\n2,Bobby\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, _) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(result.has_conflicts);
        assert!(matches!(
            result.tables[0].conflicts[0].kind,
            ConflictKind::AddAdd
        ));
        Ok(())
    }

    #[test]
    fn test_merge_left_delete() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n2,Bob\n";
        let left_csv = "id,name\n1,Alice\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", base_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        assert_eq!(merged.rows.len(), 1);
        Ok(())
    }

    #[test]
    fn test_merge_right_delete() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n2,Bob\n";
        let right_csv = "id,name\n2,Bob\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", base_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        assert_eq!(merged.rows.len(), 1);
        assert_eq!(merged.rows_by_pk()["2"].values[1], "Bob");
        Ok(())
    }

    #[test]
    fn test_merge_modify_delete_conflict() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n2,Bob\n";
        let left_csv = "id,name\n1,Alice\n2,Bobby\n"; // left modified
        let right_csv = "id,name\n1,Alice\n"; // right deleted
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, _) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(result.has_conflicts);
        assert!(matches!(
            result.tables[0].conflicts[0].kind,
            ConflictKind::ModifyDelete
        ));
        Ok(())
    }

    #[test]
    fn test_merge_strategy_ours() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let left_csv = "id,name\n1,Alicia\n";
        let right_csv = "id,name\n1,Ally\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Ours, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        let by_pk = merged.rows_by_pk();
        assert_eq!(by_pk["1"].values[1], "Alicia");
        Ok(())
    }

    #[test]
    fn test_merge_strategy_theirs() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let left_csv = "id,name\n1,Alicia\n";
        let right_csv = "id,name\n1,Ally\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", right_csv)]);

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Theirs, &no_filter())?;
        assert!(!result.has_conflicts);
        let merged = &tables["t"];
        let by_pk = merged.rows_by_pk();
        assert_eq!(by_pk["1"].values[1], "Ally");
        Ok(())
    }

    #[test]
    fn test_merge_writes_output() -> Result<()> {
        let dir = tempdir()?;
        let base_csv = "id,name\n1,Alice\n";
        let left_csv = "id,name\n1,Alicia\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", base_csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", left_csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", base_csv)]);

        let output = dir.path().join("merged.csvdb");
        let result = merge(
            &base,
            &left,
            &right,
            &output,
            MergeStrategy::Normal,
            false,
            &no_filter(),
        )?;
        assert!(!result.has_conflicts);
        assert!(output.join("schema.sql").exists());
        assert!(output.join("t.csv").exists());
        Ok(())
    }

    #[test]
    fn test_merge_multi_table() -> Result<()> {
        let dir = tempdir()?;
        let schema = "CREATE TABLE \"t1\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                       CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let base = make_csvdb(
            dir.path(),
            "base.csvdb",
            schema,
            &[("t1", "id,name\n1,Alice\n"), ("t2", "id,val\n1,x\n")],
        );
        let left = make_csvdb(
            dir.path(),
            "left.csvdb",
            schema,
            &[("t1", "id,name\n1,Alicia\n"), ("t2", "id,val\n1,x\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "right.csvdb",
            schema,
            &[("t1", "id,name\n1,Alice\n"), ("t2", "id,val\n1,y\n")],
        );

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        assert_eq!(result.tables.len(), 2);
        assert_eq!(tables["t1"].rows_by_pk()["1"].values[1], "Alicia");
        assert_eq!(tables["t2"].rows_by_pk()["1"].values[1], "y");
        Ok(())
    }

    #[test]
    fn test_merge_table_added_one_side() -> Result<()> {
        let dir = tempdir()?;
        let base_schema = SCHEMA;
        let left_schema = "CREATE TABLE \"t\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"name\" TEXT\n);\n\
                            CREATE TABLE \"t2\" (\n    \"id\" INTEGER PRIMARY KEY,\n    \"val\" TEXT\n);\n";
        let base = make_csvdb(
            dir.path(),
            "base.csvdb",
            base_schema,
            &[("t", "id,name\n1,Alice\n")],
        );
        let left = make_csvdb(
            dir.path(),
            "left.csvdb",
            left_schema,
            &[("t", "id,name\n1,Alice\n"), ("t2", "id,val\n1,new\n")],
        );
        let right = make_csvdb(
            dir.path(),
            "right.csvdb",
            base_schema,
            &[("t", "id,name\n1,Alice\n")],
        );

        let (result, _, tables) =
            merge_detail(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        assert!(!result.has_conflicts);
        assert!(tables.contains_key("t2"));
        let added = result.tables.iter().find(|t| t.name == "t2").unwrap();
        assert!(matches!(added.status, MergeTableStatus::Added));
        Ok(())
    }

    #[test]
    fn test_merge_json_output() -> Result<()> {
        let dir = tempdir()?;
        let csv = "id,name\n1,Alice\n";
        let base = make_csvdb(dir.path(), "base.csvdb", SCHEMA, &[("t", csv)]);
        let left = make_csvdb(dir.path(), "left.csvdb", SCHEMA, &[("t", csv)]);
        let right = make_csvdb(dir.path(), "right.csvdb", SCHEMA, &[("t", csv)]);

        let json = merge_json(&base, &left, &right, MergeStrategy::Normal, &no_filter())?;
        let parsed: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(parsed["has_conflicts"], false);
        assert!(parsed["tables"].is_array());
        Ok(())
    }
}
