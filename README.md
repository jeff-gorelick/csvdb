# csvdb

> **Note:** This is beta software. The API and file format may change. Use with caution in production.

A deterministic, git-friendly way to store relational data as CSV and convert it to SQLite, DuckDB, or Parquet when you need fast queries.

## Directory Layouts

A `.csvdb` directory contains:
```
mydb.csvdb/
  csvdb.toml    # format version, export settings
  schema.sql    # CREATE TABLE, CREATE INDEX, CREATE VIEW
  users.csv     # one file per table
  orders.csv
```

A `.parquetdb` directory has the same structure with Parquet files instead of CSVs:
```
mydb.parquetdb/
  csvdb.toml       # format version, export settings
  schema.sql       # CREATE TABLE, CREATE INDEX, CREATE VIEW
  users.parquet    # one file per table
  orders.parquet
```

The schema defines the structure. The data files hold the data. `csvdb.toml` records the format version and the settings used to produce the export.

## Why csvdb

**CSV format** works with standard tools:
- Edit with any text editor or spreadsheet
- Diff and merge with git
- Process with awk, pandas, Excel

**SQLite/DuckDB format** provides fast access:
- Indexed lookups without scanning entire files
- Views for complex joins and computed columns
- Full SQL query support
- Single-file distribution

**Parquet format** provides columnar storage:
- Efficient compression and encoding
- Fast analytical queries
- Wide ecosystem support (Spark, pandas, DuckDB, etc.)
- Per-table `.parquet` files in a `.parquetdb` directory

csvdb lets you store data as CSV (human-readable, git-friendly) and convert to SQLite, DuckDB, or Parquet when you need query performance.

## Installation

```bash
cargo install --path .
```

## Quick Start

```bash
# Convert an existing SQLite database to csvdb
csvdb to-csvdb mydb.sqlite
git add mydb.csvdb/
git commit -m "Track data in csvdb format"

# Edit data
vim mydb.csvdb/users.csv

# Rebuild database
csvdb to-sqlite mydb.csvdb/

# Or export to Parquet
csvdb to-parquetdb mydb.csvdb/
```

## Commands

### init — Create csvdb from raw CSV files

```bash
csvdb init ./raw_csvs/
```

Creates `raw_csvs.csvdb/` by:
- Inferring schema from CSV headers and data types
- Detecting primary keys (columns named `id` or `<table>_id`)
- Copying CSV files

Options:
- `--no-pk-detection` - Disable automatic primary key detection

### to-csvdb — Export database to csvdb

```bash
# From SQLite
csvdb to-csvdb mydb.sqlite

# From DuckDB
csvdb to-csvdb mydb.duckdb

# From Parquet
csvdb to-csvdb mydb.parquetdb/
csvdb to-csvdb single_table.parquet
```

Creates `mydb.csvdb/` containing:
- `schema.sql` - table definitions, indexes, views
- `*.csv` - one file per table, sorted by primary key

Supports multiple input formats:
- **SQLite** (`.sqlite`, `.sqlite3`, `.db`)
- **DuckDB** (`.duckdb`)
- **parquetdb** (`.parquetdb` directory)
- **Parquet** (`.parquet` single file)

Options:
- `-o, --output <dir>` - Custom output directory
- `--order <mode>` - Row ordering mode (see below)
- `--null-mode <mode>` - NULL representation in CSV (see below)
- `--pipe` - Write to temp directory, output only path (for piping)

### to-sqlite — Build SQLite database

```bash
csvdb to-sqlite mydb.csvdb/
csvdb to-sqlite mydb.parquetdb/
```

Creates `mydb.sqlite` from a csvdb or parquetdb directory.

Options:
- `--force` - Overwrite existing output file
- `--tables <list>` - Only include these tables (comma-separated)
- `--exclude <list>` - Exclude these tables (comma-separated)

### to-duckdb — Build DuckDB database

```bash
csvdb to-duckdb mydb.csvdb/
csvdb to-duckdb mydb.parquetdb/
```

Creates `mydb.duckdb` from a csvdb or parquetdb directory.

Options:
- `--force` - Overwrite existing output file
- `--tables <list>` - Only include these tables (comma-separated)
- `--exclude <list>` - Exclude these tables (comma-separated)

### to-parquetdb — Convert any format to Parquet

```bash
# From SQLite
csvdb to-parquetdb mydb.sqlite

# From DuckDB
csvdb to-parquetdb mydb.duckdb

# From csvdb
csvdb to-parquetdb mydb.csvdb/

# From a single Parquet file
csvdb to-parquetdb users.parquet
```

Creates `mydb.parquetdb/` containing:
- `schema.sql` - table definitions, indexes, views
- `csvdb.toml` - format version and export settings
- `*.parquet` - one Parquet file per table

Supports multiple input formats:
- **SQLite** (`.sqlite`, `.sqlite3`, `.db`)
- **DuckDB** (`.duckdb`)
- **csvdb** (`.csvdb` directory)
- **parquetdb** (`.parquetdb` directory)
- **Parquet** (`.parquet` single file)

Options:
- `-o, --output <dir>` - Custom output directory
- `--order <mode>` - Row ordering mode (see below)
- `--null-mode <mode>` - NULL representation (see below)
- `--pipe` - Write to temp directory, output only path (for piping)
- `--force` - Overwrite existing output directory
- `--tables <list>` - Only include these tables (comma-separated)
- `--exclude <list>` - Exclude these tables (comma-separated)

### checksum — Verify data integrity

```bash
csvdb checksum mydb.sqlite
csvdb checksum mydb.csvdb/
csvdb checksum mydb.duckdb
csvdb checksum mydb.parquetdb/
csvdb checksum users.parquet
```

Computes a SHA-256 checksum of the database content. The checksum is:
- **Format-independent**: Same data produces same hash regardless of format
- **Deterministic**: Same data always produces same hash
- **Content-based**: Includes schema structure and all row data

Use checksums to verify roundtrip conversions:
```bash
csvdb checksum original.sqlite       # a1b2c3...
csvdb to-csvdb original.sqlite
csvdb to-duckdb original.csvdb/
csvdb checksum original.duckdb       # a1b2c3... (same!)
csvdb to-parquetdb original.csvdb/
csvdb checksum original.parquetdb/   # a1b2c3... (same!)
```

## Primary Key Requirement

By default, every table must have an explicit primary key. Rows are sorted by primary key when exporting to CSV. By enforcing a stable row order, csvdb guarantees that identical data always produces identical CSV files, making git diffs meaningful and noise-free.

### Tables Without Primary Keys

For tables without a primary key (event logs, append-only tables), use the `--order` option:

```bash
# Order by all columns (deterministic but may have issues with duplicates)
csvdb to-csvdb mydb.sqlite --order=all-columns

# Add a synthetic __csvdb_rowid column (best for event/log tables)
csvdb to-csvdb mydb.sqlite --order=add-synthetic-key
```

#### Order Modes

| Mode | Description | Best For |
|------|-------------|----------|
| `pk` (default) | Order by primary key | Tables with natural keys |
| `all-columns` | Order by all columns | Reference tables without PK |
| `add-synthetic-key` | Add `__csvdb_rowid` column | Event logs, append-only data |

## NULL Handling

CSV has no native NULL concept. csvdb uses explicit conventions to preserve NULLs across database roundtrips.

By default, CSV files use `\N` (PostgreSQL convention) to represent NULL values:

```csv
"id","name","value"
"1","\N","42"      # name is NULL
"2","","42"        # name is empty string
"3","hello","\N"   # value is NULL
```

This preserves the distinction between NULL and empty string through roundtrips:
- **SQLite roundtrip**: NULL and empty string are fully preserved
- **DuckDB roundtrip**: NULL is preserved. **DuckDB limitation**: empty strings may become NULL due to a Rust driver limitation.

### --null-mode

| Mode | NULL representation | Lossless? | Use case |
|------|-------------------|-----------|----------|
| `marker` (default) | `\N` | Yes | Roundtrip-safe, distinguishes NULL from empty string |
| `empty` | empty string | No | Simpler CSV, but cannot distinguish NULL from `""` |
| `literal` | `NULL` | No | Human-readable, but cannot distinguish NULL from the string `"NULL"` |

```bash
csvdb to-csvdb mydb.sqlite                      # default: \N marker
csvdb to-csvdb mydb.sqlite --null-mode=empty     # empty string for NULL
csvdb to-csvdb mydb.sqlite --null-mode=literal   # literal "NULL" string
```

Lossy modes print a warning to stderr. Use `--pipe` to suppress warnings.

## CSV Dialect

csvdb produces a strict, deterministic CSV dialect:

| Property | Value |
|----------|-------|
| Encoding | UTF-8 |
| Delimiter | `,` (comma) |
| Quote character | `"` (double quote) |
| Quoting | Always — every field is quoted, including headers |
| Quote escaping | Doubled (`""`) per RFC 4180 |
| Record terminator | `\n` (LF), not CRLF |
| Header row | Always present as the first row |
| Row ordering | Sorted by primary key (deterministic) |
| NULL representation | Configurable via `--null-mode` (see above) |

This is mostly RFC 4180 compliant, with one deliberate deviation: line endings use LF instead of CRLF. This produces cleaner git diffs and avoids mixed-endings issues on Unix systems.

Newlines embedded within field values are preserved as-is inside quoted fields. The Rust `csv` crate handles quoting and escaping automatically.

## Workflows

### Git-Tracked Data

Store data in git, rebuild databases as needed:

```bash
# Initial setup: export existing database
csvdb to-csvdb production.sqlite
git add production.csvdb/
git commit -m "Initial data import"

# Daily workflow: edit CSVs, commit, rebuild
vim production.csvdb/users.csv
git add -p production.csvdb/
git commit -m "Update user records"
csvdb to-sqlite production.csvdb/
```

### Deploy to Production

Use csvdb as the source of truth. Track schema and data in git, export to SQLite for deployment:

```bash
# Define your schema and seed data in csvdb format
mkdir -p myapp.csvdb
cat > myapp.csvdb/schema.sql <<'EOF'
CREATE TABLE config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE rates (
    date TEXT NOT NULL,
    rate REAL NOT NULL,
    PRIMARY KEY (date)
);
EOF

# Edit data directly as CSV
cat > myapp.csvdb/config.csv <<'EOF'
key,value
app_name,MyApp
version,2.1
EOF

# Commit to git — schema and data are versioned together
git add myapp.csvdb/
git commit -m "Add rate config for Q1"

# Build SQLite for deployment
csvdb to-sqlite myapp.csvdb/
scp myapp.sqlite prod-server:/opt/myapp/data/
```

Changes go through normal code review. `git diff` shows exactly which rows changed. Rollback is `git revert`.

### Data Review via Pull Request

Treat data changes like code changes:

```bash
git checkout -b update-q2-rates
# Edit the CSV
vim myapp.csvdb/rates.csv
git add myapp.csvdb/rates.csv
git commit -m "Update Q2 rates"
git push origin update-q2-rates
# Open PR — reviewers see the exact row-level diff
```

Because CSVs are sorted by primary key, the diff contains only actual changes — no noise from row reordering.

### Piping Commands

Use `--pipe` for one-liner conversions:

```bash
# SQLite → DuckDB via pipe
csvdb to-csvdb mydb.sqlite --pipe | xargs csvdb to-duckdb

# SQLite → Parquet via pipe
csvdb to-parquetdb mydb.sqlite --pipe | xargs csvdb to-duckdb
```

The `--pipe` flag:
- Writes to system temp directory
- Outputs only the path (no "Created:" prefix)
- Uses forward slashes for cross-platform compatibility

### Database Migration

Convert between database formats:

```bash
# SQLite to DuckDB
csvdb to-csvdb legacy.sqlite
csvdb to-duckdb legacy.csvdb/

# DuckDB to SQLite
csvdb to-csvdb analytics.duckdb
csvdb to-sqlite analytics.csvdb/

# SQLite to Parquet
csvdb to-parquetdb legacy.sqlite

# Parquet to SQLite
csvdb to-sqlite legacy.parquetdb/

# Verify no data loss
csvdb checksum legacy.sqlite
csvdb checksum legacy.duckdb
csvdb checksum legacy.parquetdb/
# Checksums match = data preserved
```

### Diff and Review Changes

Use git to review data changes:

```bash
# See what changed
git diff production.csvdb/

# See changes to specific table
git diff production.csvdb/orders.csv

# Blame: who changed what
git blame production.csvdb/users.csv
```

### CI/CD Integration

Verify data integrity in CI:

```bash
#!/bin/bash
set -e

# Rebuild from csvdb source
csvdb to-sqlite data.csvdb/

# Verify checksum matches expected
EXPECTED="a1b2c3d4..."
ACTUAL=$(csvdb checksum data.sqlite)
[ "$EXPECTED" = "$ACTUAL" ] || exit 1
```

## Project Structure

```
src/
  main.rs              # CLI (clap)
  lib.rs
  commands/
    init.rs            # CSV files -> csvdb (schema inference)
    to_csv.rs          # any format -> csvdb
    to_sqlite.rs       # any format -> SQLite
    to_duckdb.rs       # any format -> DuckDB
    to_parquetdb.rs    # any format -> parquetdb (Parquet)
    checksum.rs        # Format-independent checksums
    validate.rs        # Structural integrity checks
    diff.rs            # Compare two databases
  core/
    schema.rs          # Parse/emit schema.sql, type normalization
    table.rs           # Row operations, PK handling
    csv.rs             # Deterministic CSV I/O
    input.rs           # Input format detection

tests/
  functional/
    conftest.py        # Pytest fixtures
    test_commands.py   # Functional tests
    pyproject.toml     # Python dependencies (uv)
```

## Development

```bash
cargo build
cargo run -- init ./raw_csvs/
cargo run -- to-csvdb mydb.sqlite
cargo run -- to-sqlite mydb.csvdb/
cargo run -- to-duckdb mydb.csvdb/
cargo run -- to-parquetdb mydb.sqlite
cargo run -- checksum mydb.sqlite
```

## Testing

```bash
# Rust unit tests
cargo test

# Python functional tests (151 tests)
cd tests/functional
uv run pytest

# Cross-platform (avoids .venv collision)
uv run --isolated pytest
```

## License

MIT
