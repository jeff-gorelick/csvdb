# csvdb Format Specification

**Format version:** `1`

This document is the normative specification for the csvdb on-disk format. It covers both `.csvdb` (CSV-based) and `.parquetdb` (Parquet-based) directory layouts.

## Directory Structure

### `.csvdb` directory

```
mydb.csvdb/
  csvdb.toml       # required — format metadata
  schema.sql       # required — CREATE TABLE, CREATE INDEX, CREATE VIEW
  customers.csv    # required — one CSV file per table
  orders.csv
```

### `.parquetdb` directory

```
mydb.parquetdb/
  csvdb.toml          # required — format metadata
  schema.sql          # required — CREATE TABLE, CREATE INDEX, CREATE VIEW
  customers.parquet   # required — one Parquet file per table
  orders.parquet
```

Both layouts share the same `csvdb.toml` and `schema.sql` formats. The only difference is whether data files are CSV or Parquet.

No other files are expected. The directory name conventionally ends in `.csvdb` or `.parquetdb` but this is not enforced.

## csvdb.toml

TOML file recording the format version and the settings used to produce the export.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `format_version` | string | Yes | Must be `"1"`. |
| `created_by` | string | No | Tool and version that wrote this directory (e.g. `"csvdb 0.3.0"`). |
| `order` | string | No | Row ordering mode: `"pk"` (default), `"all-columns"`, or `"add-synthetic-key"`. |
| `null_mode` | string | No | NULL representation in CSV: `"marker"` (default), `"empty"`, or `"literal"`. |
| `tables` | array of strings | No | If present, only these tables were exported (include filter). |
| `exclude` | array of strings | No | If present, these tables were excluded from export. |

When `order` or `null_mode` are absent, consumers should assume the defaults (`"pk"` and `"marker"`).

Example:

```toml
format_version = "1"
created_by = "csvdb 0.3.0"
order = "pk"
null_mode = "marker"
```

## schema.sql

A plain-text file containing semicolon-terminated SQL DDL statements. Supported statement types:

- `CREATE TABLE` — with column types, NOT NULL, DEFAULT, and PRIMARY KEY constraints
- `CREATE INDEX` / `CREATE UNIQUE INDEX`
- `CREATE VIEW`

### Ordering rules

1. `CREATE TABLE` statements — alphabetical by table name
2. `CREATE INDEX` statements — immediately after the table they belong to
3. `CREATE VIEW` statements — after all tables, alphabetical by view name

Statements are separated by `;\n`. Identifiers may be double-quoted (`"column_name"`).

The file is parsed by executing each semicolon-delimited statement into an in-memory SQLite database, so the SQL must be valid SQLite syntax.

## CSV Files

### Naming

Each table gets one file named `<tablename>.csv`. The table name is taken directly from the schema — no escaping or case-folding is applied.

### Dialect

| Property | Value |
|----------|-------|
| Encoding | UTF-8 |
| Delimiter | `,` (comma) |
| Quote character | `"` (double quote) |
| Quoting | Always — every field is quoted, including headers |
| Quote escaping | Doubled (`""`) per RFC 4180 |
| Record terminator | `\n` (LF only, not CRLF) |
| Header row | Always present as the first row |

This is RFC 4180 compliant with one deliberate deviation: line endings use LF instead of CRLF. This produces cleaner git diffs and avoids mixed-endings issues on Unix systems.

Newlines embedded within field values are preserved inside quoted fields.

### Column order

Columns appear in the order defined in the `CREATE TABLE` statement. When `order` is `"add-synthetic-key"`, a `__csvdb_rowid` column is prepended as the first column.

### Row ordering

Rows are sorted to guarantee that identical data always produces identical files.

| `order` value | Sort key | Behavior |
|---------------|----------|----------|
| `"pk"` (default) | Primary key columns | Lexicographic sort on string PK values. All tables must have a PRIMARY KEY. |
| `"all-columns"` | All columns | Lexicographic sort across all column values. Tables without a PK are allowed. |
| `"add-synthetic-key"` | `__csvdb_rowid` column | Adds a synthetic integer column preserving the original row order. |

Sort is **lexicographic on string values**, not numeric (`"10"` sorts before `"2"`). For composite primary keys, rows are sorted by the first PK column, then the second, etc. (element-wise lexicographic comparison).

## NULL Encoding

Three modes control how SQL NULL values are represented in CSV:

| `null_mode` value | NULL written as | Empty string written as | Lossless? |
|-------------------|----------------|------------------------|-----------|
| `"marker"` (default) | `\N` (two characters: backslash + N) | `""` (empty quoted field) | Yes |
| `"empty"` | `""` (empty quoted field) | `""` (empty quoted field) | No |
| `"literal"` | `NULL` (four characters) | `""` (empty quoted field) | No |

On import, the `\N` marker in a field is converted to SQL NULL. All other field values are inserted as-is.

## Blob Encoding

BLOB values are encoded as lowercase hexadecimal strings in CSV. Each byte becomes two hex characters. For example, the bytes `[0xCA, 0xFE]` are stored as `"cafe"`.

## Type Normalization

The checksum algorithm normalizes SQL types to canonical forms for cross-database consistency:

| Input type (case-insensitive substring match) | Normalized type |
|-----------------------------------------------|-----------------|
| Contains "INT" (INT, INTEGER, BIGINT, SMALLINT, TINYINT, ...) | `INTEGER` |
| Contains "FLOAT" or "DOUBLE", or equals "REAL" | `REAL` |
| Contains "CHAR", "TEXT", "STRING", "VARCHAR", or "CLOB" | `TEXT` |
| Contains "BLOB", "BINARY", or "BYTEA" | `BLOB` |
| Contains "DECIMAL" or "NUMERIC" | `NUMERIC` |
| Contains "BOOL" | `INTEGER` |
| Contains "DATE", "TIME", or "TIMESTAMP" | `TEXT` |
| Anything else | `TEXT` |

## Checksum Algorithm

The checksum produces a SHA-256 hex digest of the database content. It is format-independent: the same data produces the same hash whether stored as SQLite, DuckDB, csvdb, or parquetdb.

### Hash protocol

The hash is computed by feeding bytes into a SHA-256 hasher in the following order:

**For each table** (in alphabetical order by name):

1. Schema:
   - `TABLE:` + table name bytes + `\x00`
   - For each column (in schema order, skipping `__csvdb_rowid`):
     - `COL:` + column name bytes + `:` + normalized type bytes + `\x00`
   - If the table has a primary key (excluding `__csvdb_rowid`):
     - `PK:` + comma-joined PK column names + `\x00`
   - `\x01` (end-of-schema marker)

2. Row data:
   - `DATA:` + table name bytes + `\x00`
   - For each row (in table order):
     - For each value (in column order, skipping `__csvdb_rowid`):
       - Normalized value bytes + `\x00`
     - `\x01` (row separator)
   - `\x02` (end-of-table-data marker)

**After all tables**, views:

- For each view (in alphabetical order by name):
  - `VIEW:` + view name bytes + `\x00`
- `\x03` (end-of-views marker)

### What is excluded from the checksum

- Index definitions (they vary across databases)
- NOT NULL constraints (they vary across databases)
- Default values
- View SQL (only view names are hashed; SQL syntax varies across databases)

### Value normalization

Before hashing, values are normalized for cross-database consistency:

| Value | Normalized form |
|-------|----------------|
| Empty string | Empty string (no change) |
| Integer-valued float (e.g. `42.0`, `100.0`) | Integer string (`"42"`, `"100"`) |
| Float values | Rounded to 10 decimal places, trailing zeros stripped (e.g. `3.14159265358979` -> `"3.1415926536"`) |
| All other values | Used as-is |

## Parquet Output

Parquet files are produced by DuckDB's `COPY ... TO ... (FORMAT PARQUET)` with default settings:

- Compression: Snappy (DuckDB default)
- No explicit encoding, row group size, or page size overrides

## Schema Inference (init)

The `init` command infers schemas from raw CSV files:

- **Types inferred**: `INTEGER`, `REAL`, `TEXT` only
- **PK detection**: Columns named `id` or `<table_name>_id` are candidates. Uniqueness is tracked up to **100,000 values**; after that, tracking stops and the column is used as PK if it was unique up to that point.
- PK detection can be disabled with `--no-pk-detection`

## Versioning Policy

The `format_version` field tracks breaking changes to the on-disk format. A breaking change is any change that would cause an older reader to misinterpret data produced by a newer writer, or vice versa. Examples of breaking changes:

- Changing the CSV quoting rules
- Changing the NULL encoding convention
- Changing the checksum algorithm
- Adding required fields to `csvdb.toml`
- Changing the schema.sql ordering rules

Non-breaking changes (do not bump the version):

- Adding new optional fields to `csvdb.toml`
- Adding new commands
- Bug fixes that don't change the on-disk format

If a future format version is needed, csvdb will provide a migration path. Tools should warn on unknown `format_version` values.
