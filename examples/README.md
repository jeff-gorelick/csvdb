# csvdb Examples

## store.csvdb — A hand-written csvdb directory

A small store database with two tables, an index, and a view. Demonstrates NULL values using the `\N` marker (Bob has no email, some orders have no notes).

```bash
# Build a SQLite database from the example
csvdb to-sqlite examples/store.csvdb/

# Build a DuckDB database
csvdb to-duckdb examples/store.csvdb/

# Compute a checksum
csvdb checksum examples/store.csvdb/

# Validate the structure
csvdb validate examples/store.csvdb/
```

### Converting to Parquet

Parquet files are binary and should not be committed to git. Generate them on the fly:

```bash
csvdb to-parquetdb examples/store.csvdb/
# Creates examples/store.parquetdb/ with .parquet files
```

## raw-csvs — Input for `csvdb init`

Two plain CSV files (products, categories) without any schema. Use `csvdb init` to infer a schema and create a csvdb directory:

```bash
csvdb init examples/raw-csvs/
# Creates examples/raw-csvs.csvdb/ with inferred schema
```

The `init` command will:
- Detect `id` columns as primary keys
- Infer column types (INTEGER, REAL, TEXT)
- Generate `schema.sql` and `csvdb.toml`
- Copy and reformat the CSV files
