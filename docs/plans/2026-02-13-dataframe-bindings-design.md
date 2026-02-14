# DataFrame Bindings for csvdb Python Package

## Goal

Add pandas, polars, and pyarrow DataFrame support to the `csvdb-py` Python package. Users should be able to read csvdb directories into DataFrames and write DataFrames back to csvdb directories, with zero-copy Arrow as the interchange format.

## API

### Reading (csvdb to memory)

```python
import csvdb

# Arrow (base layer, requires pyarrow)
csvdb.to_arrow("mydb.csvdb", "users")           # -> pa.Table (single table)
csvdb.to_arrow("mydb.csvdb")                     # -> dict[str, pa.Table] (all tables)

# Pandas (requires pandas + pyarrow)
csvdb.to_pandas("mydb.csvdb", "users")           # -> pd.DataFrame
csvdb.to_pandas("mydb.csvdb")                    # -> dict[str, pd.DataFrame]

# Polars (requires polars + pyarrow)
csvdb.to_polars("mydb.csvdb", "users")           # -> pl.DataFrame
csvdb.to_polars("mydb.csvdb")                    # -> dict[str, pl.DataFrame]

# SQL variants
csvdb.sql_arrow("mydb.csvdb", "SELECT ...")       # -> pa.Table
csvdb.sql_pandas("mydb.csvdb", "SELECT ...")      # -> pd.DataFrame
csvdb.sql_polars("mydb.csvdb", "SELECT ...")      # -> pl.DataFrame
```

All read functions accept `tables=[]` and `exclude=[]` kwargs for filtering. The table name parameter is optional — omit it to get all tables as a dict.

### Writing (memory to csvdb)

```python
# Accepts dict of pd.DataFrame, pl.DataFrame, or pa.Table (auto-detected)
csvdb.to_csvdb({"users": df_users, "orders": df_orders}, output="out.csvdb")
```

The existing `to_csvdb(path_string)` signature is unchanged. When a dict is passed instead of a string, the function dispatches to the DataFrame write path.

### Existing API

All existing functions are unchanged: `to_csvdb(path)`, `to_sqlite()`, `to_duckdb()`, `to_parquetdb()`, `sql()`, `checksum()`, `diff()`, `validate()`, `init()`.

## Architecture

### Why Arrow

The `sql.rs` module already queries through DuckDB and receives Arrow RecordBatches internally, then discards them by stringifying into `Vec<Vec<String>>`. The core change is to stop stringifying and pass Arrow batches across the FFI boundary. Both pandas (via pyarrow) and polars (natively) consume Arrow, so writing the bridge once covers both.

Arrow is already a dependency (`arrow = "56"` in the core crate Cargo.toml).

### Rust Side

Two new functions in the core crate:

**`sql::sql_query_arrow(path, query) -> Result<Vec<RecordBatch>>`**

Reuses the existing DuckDB query path in `sql.rs` but returns Arrow RecordBatches directly instead of stringifying them. The existing `sql_query()` function can be refactored to call `sql_query_arrow()` internally, then stringify.

**`read::read_tables_arrow(csvdb_path, filter) -> Result<Vec<(String, Vec<RecordBatch>)>>`**

Loads a csvdb directory into DuckDB (reusing the pattern from `diff.rs`'s `load_tables`), then runs `SELECT * FROM "table_name"` for each table and returns the Arrow batches. A single-table variant `read_table_arrow()` is a thin wrapper.

### PyO3 Arrow Bridge

Uses the Arrow C Data Interface (FFI) to pass RecordBatches from Rust to Python's pyarrow with zero copy. A single helper function shared by all new PyO3 functions:

```rust
fn arrow_batches_to_pyarrow(py: Python<'_>, batches: &[RecordBatch]) -> PyResult<PyObject> {
    // Export Rust RecordBatch via arrow::ffi
    // Import on Python side via pyarrow C Data Interface
}
```

New PyO3 functions: `to_arrow`, `read_tables_arrow`, `sql_arrow`. These return pyarrow Tables.

### Python Convenience Layer

Thin Python wrappers over the Rust Arrow functions:

- `to_pandas()` calls `to_arrow()` then `arrow_table.to_pandas(types_mapper=...)` with nullable dtypes (`pd.Int64Dtype()`, `pd.StringDtype()`, etc.)
- `to_polars()` calls `to_arrow()` then `pl.from_arrow(arrow_table)` (zero-copy)
- `sql_pandas()` and `sql_polars()` follow the same pattern

pandas and polars are lazy-imported so they are not required dependencies.

### Write Path

`to_csvdb(dict_of_dataframes, output=...)` works in three steps:

1. **Python normalizes to Arrow** — detect input type, convert to `dict[str, pa.Table]`. pandas: `pa.Table.from_pandas(df)`. polars: `df.to_arrow()`. pyarrow: pass through.

2. **Rust receives Arrow tables via FFI** — same C Data Interface in reverse. A new PyO3 function `to_csvdb_from_arrow` receives the tables.

3. **Rust writes the csvdb directory** — infer `schema.sql` from Arrow schemas, then write CSVs using the existing deterministic CSV writer with `\N` null markers and PK-sorted rows.

Type mapping from Arrow to SQL: `int64` -> `INTEGER`, `float64` -> `REAL`, `utf8` -> `TEXT`, `bool` -> `INTEGER`, `date32` -> `TEXT`. Primary keys can be inferred the same way `init` does it, or the user can pass explicit PK info.

### Packaging

```toml
[project.optional-dependencies]
arrow = ["pyarrow>=12"]
pandas = ["pandas>=1.5", "pyarrow>=12"]
polars = ["polars>=0.19", "pyarrow>=12"]
all = ["pandas>=1.5", "polars>=0.19", "pyarrow>=12"]
```

Base `pip install csvdb-py` stays lightweight. `pip install csvdb-py[pandas]` pulls in what's needed. Functions that require pyarrow raise a clear `ImportError` if it's not installed.

## Implementation Phases

### Phase 1: Rust Core (Read Path)

- Add `sql_query_arrow()` to `sql.rs`
- Add `read_tables_arrow()` in a new `read.rs` module
- Refactor existing `sql_query()` to call `sql_query_arrow()` internally
- Unit tests

### Phase 2: PyO3 Arrow Bridge

- Add Arrow FFI helper (`arrow_batches_to_pyarrow`)
- Add `to_arrow`, `sql_arrow` as PyO3 functions
- Add pyarrow to test dependencies
- Tests

### Phase 3: Python Convenience Layer

- `to_pandas`, `to_polars`, `sql_pandas`, `sql_polars` as thin Python wrappers
- Lazy imports for pandas/polars
- Optional dependency groups in pyproject.toml
- Tests

### Phase 4: Write Path

- Python-side normalization (DataFrame to Arrow)
- PyO3 function to receive Arrow tables via FFI
- Rust function to infer schema.sql from Arrow schema and write CSVs
- Roundtrip tests: `df -> to_csvdb -> to_polars -> assert equal`

### Phase 5: Polish

- Type stubs (`_core.pyi`)
- Update README with DataFrame examples
- Release

Phases 1-3 are the high-value work and cover the read path (most use cases). Phase 4 (write) can ship separately. Each phase is independently releasable.
