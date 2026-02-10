# Benchmarks

Comparing csvdb against sqlite3, sqlite-utils, and DuckDB CLI for CSV export and import operations.

**Test setup:** 5 tables, each with 7 columns (INTEGER, TEXT, REAL, nullable). macOS (Apple Silicon), release build.

## Export (SQLite -> CSV)

| Tool | 5K rows | 50K rows | 500K rows |
|------|---------|----------|-----------|
| csvdb to-csvdb | 20ms | 61ms | 468ms |
| sqlite3 .mode csv | 11ms | 48ms | 393ms |
| sqlite-utils rows --csv | 352ms | 423ms | 1.13s |
| duckdb COPY TO csv | 46ms | 64ms | 271ms |

csvdb includes schema extraction, NULL handling, deterministic quoting, and PK-sorted output — work the raw tools don't do. Despite this, csvdb is competitive with sqlite3 and significantly faster than sqlite-utils. DuckDB is fastest at large scale due to its columnar engine.

## Import (CSV -> SQLite)

| Tool | 5K rows | 50K rows | 500K rows |
|------|---------|----------|-----------|
| csvdb to-sqlite | 38ms | 131ms | 1.11s |
| sqlite3 .import | 17ms | 81ms | 733ms |
| sqlite-utils insert --csv | 393ms | 752ms | 4.47s |

csvdb import includes schema creation, type-aware parsing, and NULL restoration. sqlite3 `.import` is faster but requires manual schema setup and doesn't handle NULLs. sqlite-utils is 3-4x slower than csvdb.

## Checksum

| Input | 5K rows | 50K rows | 500K rows |
|-------|---------|----------|-----------|
| SQLite file | 20ms | 75ms | 632ms |
| csvdb directory | 17ms | 51ms | 402ms |

No equivalent in other tools — csvdb provides format-independent checksums for data integrity verification.

## Key Takeaways

- **csvdb vs sqlite3**: ~1.2-1.5x slower, but csvdb produces deterministic, sorted, NULL-aware CSV with full schema — sqlite3 produces raw unsorted CSV with no schema
- **csvdb vs sqlite-utils**: 2-10x faster across all operations
- **csvdb vs DuckDB**: Comparable at small/medium scale; DuckDB faster at 500K+ rows for raw CSV export, but doesn't produce schema or handle NULL conventions

## Running Benchmarks

```bash
cargo build --release -p csvdb
cd benchmarks && uv run python run.py
```

Requires: sqlite3 (system), sqlite-utils (`pip install sqlite-utils`), duckdb CLI (`brew install duckdb`). Missing tools are skipped automatically.
