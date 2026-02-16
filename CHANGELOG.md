# Changelog

## 0.2.10 (2026-02-16)

- Idempotent releases: re-running the release workflow no longer fails if packages are already published

## 0.2.9 (2026-02-15)

- 86% line coverage from combined unit + functional tests using cargo-llvm-cov
- CI coverage job runs unit tests and functional tests against instrumented binary
- Workspace versioning: single version source, set automatically from git tag at release time
- CI runs on all branches, main requires PR with passing checks
- Fix DuckDB parquet extension autoload on macOS ARM

## 0.2.8 (2026-02-14)

- DataFrame bindings: read csvdb into pandas, polars, or pyarrow DataFrames with zero-copy Arrow interchange
- Write DataFrames back to csvdb format
- SQL queries returning DataFrames: `sql_arrow()`, `sql_pandas()`, `sql_polars()`
- Type stubs (`.pyi`) for IDE autocomplete and type checking
- Install extras: `pip install csvdb-py[pandas]`, `csvdb-py[polars]`, `csvdb-py[all]`

## 0.2.7 (2026-02-13)

- `--output` flag on `init`, `to-sqlite`, and `to-duckdb` for custom output paths
- `--force` flag on `init` to overwrite existing output directories
- `--tables`/`--exclude` flags on `init` and `diff` for filtering by table name
- All flags available across CLI, Python, C FFI, and Perl bindings

## 0.2.6 (2026-02-12)

- Rename PyPI package from `csvdb-tool` to `csvdb-py`
- Clean up Python API surface
- `csvdb init` accepts single CSV files
- Auto-publish to crates.io on release

## 0.2.4 (2026-02-11)

- `csvdb init` accepts a single CSV file in addition to directories

## 0.2.3 (2026-02-11)

- Rename Python module: `import csvdb` instead of `import csvdb_python`
- Add `csvdb-cli` PyPI package: install via `uvx csvdb-cli` or `pipx install csvdb-cli`

## 0.2.1 (2026-02-11)

- FK inference, rayon parallelism, git hooks, PyPI publishing

## 0.2.0 (2026-02-07)

- Parquet support: `to-parquetdb` command and `.parquetdb` input across all commands
- Single `.parquet` file import
- REAL to DOUBLE precision fix for DuckDB
- Float normalization in diff

## 0.1.0 (2026-02-05)

- Initial release
- `to-csvdb`, `to-sqlite`, `to-duckdb`, `init`, `checksum`, `diff`, `validate`
- Deterministic output with rows sorted by primary key
- Lossless NULL handling using `\N` marker
- Cross-format checksums
