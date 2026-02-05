# csvdb Directory Format (format_version "1")

This document specifies the `.csvdb` directory format at `format_version = "1"`.

## Directory Structure

A `.csvdb` directory contains exactly these files:

| File | Role |
|------|------|
| `csvdb.toml` | Format version, export settings, table filtering |
| `schema.sql` | DDL: table definitions, indexes, views |
| `<table>.csv` | One CSV file per table, containing all row data |

No other files are expected. The directory name conventionally ends in `.csvdb` but this is not enforced.

## csvdb.toml

TOML file recording the format version and the settings used to produce the export.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `format_version` | string | Yes | Must be `"1"` |
| `created_by` | string | No | Tool and version that wrote this directory (e.g. `"csvdb 0.3.0"`) |
| `order` | string | No | Row ordering mode. One of: `"pk"` (default), `"all-columns"`, `"add-synthetic-key"` |
| `null_mode` | string | No | NULL representation in CSV. One of: `"marker"` (default), `"empty"`, `"literal"` |
| `tables` | array of strings | No | If present, only these tables were exported (include filter) |
| `exclude` | array of strings | No | If present, these tables were excluded from export |

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

- `CREATE TABLE` -- with column types, NOT NULL, DEFAULT, and PRIMARY KEY constraints
- `CREATE INDEX` / `CREATE UNIQUE INDEX`
- `CREATE VIEW`

Tables are written first (alphabetical order), each followed by its indexes. Views are written last (they may reference tables). Statements are separated by `;\n`. Identifiers are double-quoted (`"column_name"`).

The file is parsed by executing each semicolon-delimited statement into an in-memory SQLite database, so the SQL must be valid SQLite syntax.

## CSV Files

### Naming

Each table gets one file named `<tablename>.csv`. The table name is taken directly from the schema -- no escaping or case-folding is applied.

### Dialect

- Header row: present, column names match the schema
- Quoting: all fields are quoted (`QuoteStyle::Always`), using double-quote (`"`) as the quote character
- Delimiter: comma (`,`)
- Line endings: LF (`\n`)
- Encoding: UTF-8

### Column Order

Columns appear in the order defined in the `CREATE TABLE` statement. When `order` is `"add-synthetic-key"`, a `__csvdb_rowid` column is prepended as the first column.

### Row Order

| `order` value | Behavior |
|---------------|----------|
| `"pk"` | Rows sorted by primary key columns (ascending, lexicographic). All tables must have a PRIMARY KEY. |
| `"all-columns"` | Rows sorted by all columns (ascending, lexicographic). Tables without a PK are allowed. Duplicate rows may exist. |
| `"add-synthetic-key"` | A `__csvdb_rowid` INTEGER column is added (derived from SQLite/DuckDB rowid). Rows sorted by this column. |

### NULL Representation

| `null_mode` value | NULL written as | Lossless? |
|-------------------|----------------|-----------|
| `"marker"` | `\N` (two characters: backslash + N) | Yes -- empty string `""` is preserved as a distinct value |
| `"empty"` | empty string (zero characters between quotes) | No -- NULL and empty string are conflated |
| `"literal"` | `NULL` (four characters) | No -- NULL and the string `"NULL"` are conflated |

On import, the `\N` marker in a field is converted to SQL NULL. All other field values are inserted as-is.

## Migration

If a future format version is needed, csvdb will provide a migrate command. Tools should warn on unknown `format_version` values.
