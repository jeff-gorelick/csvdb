# csvdb

Version-control your relational data like code.

> **Note:** This is beta software. The API and file format may change. Use with caution in production.

SQLite and DuckDB files are binary — git can't diff them, reviewers can't read them, and merges are impossible. csvdb converts your database into a diffable directory of CSV files + `schema.sql`, lossless through any roundtrip. Convert back to SQLite, DuckDB, or Parquet when you need query performance.

```diff
 # git diff myapp.csvdb/rates.csv
 "date","rate"
 "2024-01-01","4.50"
-"2024-04-01","4.25"
+"2024-04-01","3.75"
+"2024-07-01","3.50"
```

Every change is a readable, reviewable line in a PR. No binary blobs, no "file changed" with no context.

**Use cases:**
- Seed data and test fixtures committed alongside code
- Config and lookup tables reviewed in PRs before deploy
- CI integrity checks: `csvdb checksum data.csvdb/ | grep $EXPECTED`
- Migrating between SQLite, DuckDB, and Parquet without ETL scripts
- Manual edits in a spreadsheet or text editor, rebuild with one command
- Audit trail: `git blame` on any CSV row shows who changed it and when

## Directory Layouts

A `.csvdb` directory contains:
```
mydb.csvdb/
  csvdb.toml    # format version, export settings
  schema.sql    # CREATE TABLE, CREATE INDEX, CREATE VIEW
  users.csv     # one file per table
  orders.csv
```

A `.parquetdb` directory uses the same layout with Parquet files:
```
mydb.parquetdb/
  csvdb.toml       # format version, export settings
  schema.sql       # CREATE TABLE, CREATE INDEX, CREATE VIEW
  users.parquet    # one file per table
  orders.parquet
```

The schema defines the structure; the data files hold the rows. `csvdb.toml` records the format version and export settings.

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
# Rust (via cargo)
cargo install csvdb

# Python library (import csvdb)
pip install csvdb-py

# Standalone binary (via pip/pipx/uvx)
uvx csvdb-cli
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
# From a directory of CSV files
csvdb init ./raw_csvs/

# From a single CSV file
csvdb init data.csv
```

Creates a `.csvdb` directory by:
- Inferring schema from CSV headers and data types
- Detecting primary keys (columns named `id` or `<table>_id`)
- Detecting foreign keys (columns like `user_id` referencing `users.id`)
- Copying CSV files

Options:
- `--no-pk-detection` - Disable automatic primary key detection
- `--no-fk-detection` - Disable automatic foreign key detection

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

Accepted input formats:
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

Accepted input formats:
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
- **Format-independent**: same data produces the same hash whether stored as SQLite, DuckDB, csvdb, or Parquet
- **Deterministic**: same data always produces the same hash
- **Content-based**: covers schema structure and all row data

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

Every table must have a primary key. Rows sort by primary key on export, so identical data always produces identical CSV files and git diffs show only real changes.

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

CSV has no native NULL. By default, csvdb uses `\N` (the PostgreSQL convention) to preserve the distinction between NULL and empty string:

```csv
"id","name","value"
"1","\N","42"      # name is NULL
"2","","42"        # name is empty string
"3","hello","\N"   # value is NULL
```

Roundtrip behavior:
- **SQLite**: NULL and empty string fully preserved
- **DuckDB**: NULL preserved; empty strings may become NULL (Rust driver limitation)

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

Mostly RFC 4180 compliant, with one deliberate deviation: LF line endings instead of CRLF, for cleaner git diffs. Newlines within field values are preserved inside quoted fields.

See [FORMAT.md](FORMAT.md) for the full format specification.

## Gotchas

Things that may surprise you on day one:

- **String-based sorting.** PK sort is lexicographic. `"10"` sorts before `"2"`. Use zero-padded strings or INTEGER primary keys for numeric order.

- **Schema inference is limited.** `csvdb init` infers three types: `INTEGER`, `REAL`, `TEXT`. Edit `schema.sql` after init for dates, booleans, or blobs.

- **PK detection stops at 100k values.** During `init`, uniqueness tracking for primary key candidates stops after 100,000 values. A column unique to that point still becomes the PK.

- **Float precision in checksums.** Values are normalized to 10 decimal places. `42.0` becomes `42`. Small precision differences across databases are absorbed.

- **DuckDB empty strings.** Empty strings in TEXT columns may become NULL when round-tripping through DuckDB (Rust driver limitation).

- **BLOBs are hex in CSV.** BLOB data is stored as lowercase hex (e.g. `cafe`). Roundtrips correctly through SQLite and DuckDB.

- **Duplicate PKs are not caught on read.** Duplicate primary keys in CSV files cause errors at INSERT time, not at read time.

- **DuckDB indexes are lost.** DuckDB does not expose index metadata. Indexes in `schema.sql` survive csvdb-to-SQLite conversion but not DuckDB-to-csvdb.

- **Views are alphabetically ordered.** If view A depends on view B, you may need to reorder them in `schema.sql`.

- **`__csvdb_rowid` is reserved.** The `add-synthetic-key` order mode uses this column name.

## Examples

The [`examples/`](examples/) directory contains:

- **`examples/store.csvdb/`** — Two tables, an index, a view, and NULL values
- **`examples/raw-csvs/`** — Plain CSV files for `csvdb init`

See [`examples/README.md`](examples/README.md) for details.

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

Track schema and data in git; export to SQLite for deployment:

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

## Python Bindings

Native Python bindings via PyO3. Call csvdb functions from Python with no subprocess overhead.

### Install

```bash
pip install csvdb-py

# With DataFrame support
pip install csvdb-py[pandas]    # pandas + pyarrow
pip install csvdb-py[polars]    # polars + pyarrow
pip install csvdb-py[all]       # pandas + polars + pyarrow
```

### API

```python
import csvdb

# Convert between formats
csvdb.to_csvdb("mydb.sqlite", force=True)
csvdb.to_sqlite("mydb.csvdb", force=True)
csvdb.to_duckdb("mydb.csvdb", force=True)
csvdb.to_parquetdb("mydb.csvdb", force=True)

# Incremental export (only re-exports changed tables)
result = csvdb.to_csvdb_incremental("mydb.sqlite")
# result: {"path": "...", "added": [...], "updated": [...], "unchanged": [...], "removed": [...]}

# Checksum (format-independent, deterministic)
hash = csvdb.checksum("mydb.csvdb")

# SQL queries (read-only, returns list of dicts)
rows = csvdb.sql("mydb.csvdb", "SELECT name, COUNT(*) AS n FROM users GROUP BY name")

# Diff two databases
has_diff = csvdb.diff("v1.csvdb", "v2.csvdb")

# Validate structure
info = csvdb.validate("mydb.csvdb")

# Initialize csvdb from raw CSV files
result = csvdb.init("./raw_csvs/")

# Selective export
csvdb.to_csvdb("mydb.sqlite", tables=["users", "orders"], force=True)
csvdb.to_csvdb("mydb.sqlite", exclude=["logs"], force=True)
```

### DataFrame Support

Read csvdb data into pandas, polars, or pyarrow DataFrames through zero-copy Arrow.

```python
import csvdb

# Read as pyarrow Tables
table = csvdb.to_arrow("mydb.csvdb", "users")        # single table -> pa.Table
tables = csvdb.to_arrow("mydb.csvdb")                 # all tables -> dict[str, pa.Table]

# Read as pandas DataFrames
df = csvdb.to_pandas("mydb.csvdb", "users")           # single table -> pd.DataFrame
dfs = csvdb.to_pandas("mydb.csvdb")                   # all tables -> dict[str, pd.DataFrame]

# Read as polars DataFrames
df = csvdb.to_polars("mydb.csvdb", "users")           # single table -> pl.DataFrame
dfs = csvdb.to_polars("mydb.csvdb")                   # all tables -> dict[str, pl.DataFrame]

# SQL queries returning DataFrames
table = csvdb.sql_arrow("mydb.csvdb", "SELECT * FROM users WHERE score > 90")
df = csvdb.sql_pandas("mydb.csvdb", "SELECT * FROM users WHERE score > 90")
df = csvdb.sql_polars("mydb.csvdb", "SELECT * FROM users WHERE score > 90")
```

Write DataFrames back to csvdb format:

```python
import pandas as pd

df = pd.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
csvdb.to_csvdb({"users": df}, output="mydb.csvdb")

# Works with any DataFrame type (pandas, polars, pyarrow)
import polars as pl
df = pl.DataFrame({"id": [1, 2], "name": ["Alice", "Bob"]})
csvdb.to_csvdb({"users": df}, output="mydb.csvdb", force=True)
```

### Development

```bash
cd csvdb-python
uv sync
uv run maturin develop --release
uv run pytest
```

## Perl Bindings

Perl bindings via a C FFI shared library and `FFI::Platypus`.

### Setup

```bash
# Build the shared library
cargo build --release -p csvdb-ffi

# Install dependencies (macOS)
brew install cpanminus libffi
LDFLAGS="-L/opt/homebrew/opt/libffi/lib" \
CPPFLAGS="-I/opt/homebrew/opt/libffi/include" \
cpanm FFI::Platypus

# Install dependencies (Linux)
sudo apt-get install cpanminus libffi-dev
cpanm FFI::Platypus
```

### Running Examples

```bash
perl -Iperl/lib perl/examples/basic_usage.pl
```

### API

```perl
use Csvdb;

print Csvdb::version(), "\n";

# Convert between formats
my $csvdb_path  = Csvdb::to_csvdb(input => "mydb.sqlite", force => 1);
my $sqlite_path = Csvdb::to_sqlite(input => "mydb.csvdb", force => 1);
my $duckdb_path = Csvdb::to_duckdb(input => "mydb.csvdb", force => 1);

# Checksum
my $hash = Csvdb::checksum(input => "mydb.csvdb");

# SQL query (returns CSV text)
my $csv = Csvdb::sql(path => "mydb.csvdb", query => "SELECT * FROM users");

# Diff (returns 0=identical, 1=differences)
my $rc = Csvdb::diff(left => "v1.csvdb", right => "v2.csvdb");

# Validate (returns 0=valid, 1=errors)
my $rc = Csvdb::validate(input => "mydb.csvdb");
```

### Running Tests

```bash
cargo build --release -p csvdb-ffi
prove perl/t/
```

## Project Structure

```
csvdb/                    # Core library + CLI binary
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
      sql.rs             # Read-only SQL queries
      read.rs            # Read tables as Arrow RecordBatches
      write.rs           # Write Arrow tables to csvdb
    core/
      schema.rs          # Parse/emit schema.sql, type normalization
      table.rs           # Row operations, PK handling
      csv.rs             # Deterministic CSV I/O
      input.rs           # Input format detection
csvdb-python/             # Python bindings (PyO3)
  src/lib.rs
  csvdb.pyi                # Type stubs
  examples/
    basic_usage.py
    advanced_usage.py
csvdb-ffi/                # C FFI for Perl and other languages
  src/lib.rs
perl/                     # Perl module (FFI::Platypus)
  lib/Csvdb.pm
  examples/basic_usage.pl
tests/functional/         # CLI functional tests
  conftest.py
  test_*.py
  pyproject.toml
```

## Development

```bash
cargo build -p csvdb
cargo run -p csvdb -- init ./raw_csvs/
cargo run -p csvdb -- to-csvdb mydb.sqlite
cargo run -p csvdb -- to-sqlite mydb.csvdb/
cargo run -p csvdb -- to-duckdb mydb.csvdb/
cargo run -p csvdb -- to-parquetdb mydb.sqlite
cargo run -p csvdb -- checksum mydb.sqlite
```

## Testing

```bash
# Rust unit tests
cargo test

# Functional tests
cd tests/functional
uv run pytest

# Cross-platform (avoids .venv collision)
uv run --isolated pytest
```

## License

MIT
