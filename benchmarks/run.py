#!/usr/bin/env python3
"""Benchmark csvdb against sqlite3, sqlite-utils, and DuckDB CLI.

Measures export (DB -> CSV), import (CSV -> DB), and checksum operations
at multiple scales.

Usage:
    cd benchmarks && uv run python run.py

Requires: sqlite-utils, duckdb CLI, csvdb binary (cargo build --release)
"""

import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SCALES = [
    {"label": "Small (5 tables x 1K rows)", "tables": 5, "rows": 1_000},
    {"label": "Medium (5 tables x 10K rows)", "tables": 5, "rows": 10_000},
    {"label": "Large (5 tables x 100K rows)", "tables": 5, "rows": 100_000},
]

CSVDB_BIN = None  # resolved at startup


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def find_csvdb():
    """Find the csvdb binary."""
    candidates = [
        os.path.join(os.path.dirname(__file__), "..", "target", "release", "csvdb"),
        shutil.which("csvdb"),
    ]
    for c in candidates:
        if c and os.path.isfile(c) and os.access(c, os.X_OK):
            return os.path.abspath(c)
    return None


def run(cmd, **kwargs):
    """Run a command, return elapsed seconds."""
    start = time.perf_counter()
    result = subprocess.run(
        cmd, capture_output=True, text=True,
        timeout=300, **kwargs,
    )
    elapsed = time.perf_counter() - start
    if result.returncode != 0:
        print(f"  FAILED: {' '.join(cmd)}", file=sys.stderr)
        print(f"  stderr: {result.stderr[:500]}", file=sys.stderr)
        return None
    return elapsed


def create_test_db(path, num_tables, rows_per_table):
    """Create a SQLite database with test data."""
    conn = sqlite3.connect(path)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=OFF")

    for t in range(num_tables):
        name = f"table_{t:03d}"
        conn.execute(f"""
            CREATE TABLE "{name}" (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                category TEXT,
                value REAL,
                count INTEGER,
                active INTEGER NOT NULL DEFAULT 1,
                notes TEXT
            )
        """)
        rows = [
            (
                i,
                f"item_{i:06d}",
                f"cat_{i % 20}",
                round(i * 1.23, 2),
                i % 1000,
                1 if i % 7 != 0 else 0,
                f"Notes for item {i}" if i % 3 == 0 else None,
            )
            for i in range(rows_per_table)
        ]
        conn.executemany(
            f'INSERT INTO "{name}" VALUES (?, ?, ?, ?, ?, ?, ?)', rows
        )

    conn.commit()
    conn.close()


def fmt(seconds):
    """Format elapsed time."""
    if seconds is None:
        return "FAIL"
    if seconds < 0.01:
        return f"{seconds * 1000:.1f}ms"
    if seconds < 1:
        return f"{seconds * 1000:.0f}ms"
    return f"{seconds:.2f}s"


def file_size_mb(path):
    """Get file or directory size in MB."""
    if os.path.isdir(path):
        total = sum(
            os.path.getsize(os.path.join(dp, f))
            for dp, _, fns in os.walk(path)
            for f in fns
        )
    elif os.path.isfile(path):
        total = os.path.getsize(path)
    else:
        return 0
    return total / (1024 * 1024)


# ---------------------------------------------------------------------------
# Benchmark: Export (SQLite -> CSV)
# ---------------------------------------------------------------------------

def bench_export_csvdb(db_path, tmpdir):
    """csvdb to-csvdb"""
    out = os.path.join(tmpdir, "csvdb_export.csvdb")
    return run([CSVDB_BIN, "to-csvdb", "--force", "-o", out, db_path])


def bench_export_sqlite3(db_path, tmpdir):
    """sqlite3 .mode csv + .output per table"""
    conn = sqlite3.connect(db_path)
    tables = [r[0] for r in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    ).fetchall()]
    conn.close()

    out_dir = os.path.join(tmpdir, "sqlite3_export")
    os.makedirs(out_dir, exist_ok=True)

    commands = ""
    for table in tables:
        csv_path = os.path.join(out_dir, f"{table}.csv")
        commands += f".headers on\n.mode csv\n.output {csv_path}\nSELECT * FROM \"{table}\";\n"
    commands += ".quit\n"

    return run(["sqlite3", db_path], input=commands)


def bench_export_sqlite_utils(db_path, tmpdir):
    """sqlite-utils rows --csv per table"""
    conn = sqlite3.connect(db_path)
    tables = [r[0] for r in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    ).fetchall()]
    conn.close()

    out_dir = os.path.join(tmpdir, "sqlite_utils_export")
    os.makedirs(out_dir, exist_ok=True)

    start = time.perf_counter()
    for table in tables:
        csv_path = os.path.join(out_dir, f"{table}.csv")
        result = subprocess.run(
            ["sqlite-utils", "rows", db_path, table, "--csv"],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        with open(csv_path, "w") as f:
            f.write(result.stdout)
    return time.perf_counter() - start


def bench_export_duckdb(db_path, tmpdir):
    """DuckDB COPY ... TO (csv)"""
    conn = sqlite3.connect(db_path)
    tables = [r[0] for r in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    ).fetchall()]
    conn.close()

    out_dir = os.path.join(tmpdir, "duckdb_export")
    os.makedirs(out_dir, exist_ok=True)

    commands = f"INSTALL sqlite; LOAD sqlite; ATTACH '{db_path}' AS src (TYPE sqlite);\n"
    for table in tables:
        csv_path = os.path.join(out_dir, f"{table}.csv").replace("\\", "/")
        commands += f"COPY src.\"{table}\" TO '{csv_path}' (HEADER, DELIMITER ',');\n"

    return run(["duckdb", "-c", commands])


# ---------------------------------------------------------------------------
# Benchmark: Import (CSV -> SQLite)
# ---------------------------------------------------------------------------

def bench_import_csvdb(csvdb_dir, tmpdir):
    """csvdb to-sqlite"""
    return run([CSVDB_BIN, "to-sqlite", "--force", csvdb_dir])


def bench_import_sqlite3(db_path, tmpdir):
    """sqlite3 .import per table"""
    conn = sqlite3.connect(db_path)
    tables = [r[0] for r in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    ).fetchall()]
    schema_sql = "\n".join(
        conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=? ", (t,)
        ).fetchone()[0] + ";"
        for t in tables
    )
    conn.close()

    csv_dir = os.path.join(tmpdir, "sqlite3_export")
    out_db = os.path.join(tmpdir, "sqlite3_imported.db")
    if os.path.exists(out_db):
        os.remove(out_db)

    commands = schema_sql + "\n"
    for table in tables:
        csv_path = os.path.join(csv_dir, f"{table}.csv")
        if os.path.exists(csv_path):
            commands += f".mode csv\n.import {csv_path} {table}\n"
    commands += ".quit\n"

    return run(["sqlite3", out_db], input=commands)


def bench_import_sqlite_utils(db_path, tmpdir):
    """sqlite-utils insert --csv per table"""
    csv_dir = os.path.join(tmpdir, "sqlite_utils_export")
    out_db = os.path.join(tmpdir, "sqlite_utils_imported.db")
    if os.path.exists(out_db):
        os.remove(out_db)

    conn = sqlite3.connect(db_path)
    tables = [r[0] for r in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    ).fetchall()]
    conn.close()

    start = time.perf_counter()
    for table in tables:
        csv_path = os.path.join(csv_dir, f"{table}.csv")
        if not os.path.exists(csv_path):
            continue
        result = subprocess.run(
            ["sqlite-utils", "insert", out_db, table, csv_path, "--csv"],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
    return time.perf_counter() - start


# ---------------------------------------------------------------------------
# Benchmark: Checksum
# ---------------------------------------------------------------------------

def bench_checksum_csvdb(db_path):
    """csvdb checksum"""
    return run([CSVDB_BIN, "checksum", db_path])


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def run_scale(scale, available_tools):
    """Run all benchmarks at a given scale."""
    label = scale["label"]
    num_tables = scale["tables"]
    rows = scale["rows"]
    total_rows = num_tables * rows

    print(f"\n{'=' * 60}")
    print(f"  {label} ({total_rows:,} total rows)")
    print(f"{'=' * 60}")

    with tempfile.TemporaryDirectory() as tmpdir:
        # Create test database
        db_path = os.path.join(tmpdir, "bench.sqlite")
        print("\n  Creating test database...", end=" ", flush=True)
        create_test_db(db_path, num_tables, rows)
        db_size = file_size_mb(db_path)
        print(f"{db_size:.1f} MB")

        # --- Export benchmarks ---
        print(f"\n  {'Export (SQLite -> CSV)':<35} {'Time':>10}")
        print(f"  {'-' * 45}")

        t = bench_export_csvdb(db_path, tmpdir)
        print(f"  {'csvdb to-csvdb':<35} {fmt(t):>10}")

        if "sqlite3" in available_tools:
            t = bench_export_sqlite3(db_path, tmpdir)
            print(f"  {'sqlite3 .mode csv':<35} {fmt(t):>10}")

        if "sqlite-utils" in available_tools:
            t = bench_export_sqlite_utils(db_path, tmpdir)
            print(f"  {'sqlite-utils rows --csv':<35} {fmt(t):>10}")

        if "duckdb" in available_tools:
            t = bench_export_duckdb(db_path, tmpdir)
            print(f"  {'duckdb COPY TO csv':<35} {fmt(t):>10}")

        # Prepare csvdb dir for import benchmark
        csvdb_dir = os.path.join(tmpdir, "csvdb_export.csvdb")
        if not os.path.isdir(csvdb_dir):
            run([CSVDB_BIN, "to-csvdb", "--force", "-o", csvdb_dir, db_path])

        # --- Import benchmarks ---
        print(f"\n  {'Import (CSV -> SQLite)':<35} {'Time':>10}")
        print(f"  {'-' * 45}")

        t = bench_import_csvdb(csvdb_dir, tmpdir)
        print(f"  {'csvdb to-sqlite':<35} {fmt(t):>10}")

        if "sqlite3" in available_tools:
            t = bench_import_sqlite3(db_path, tmpdir)
            print(f"  {'sqlite3 .import':<35} {fmt(t):>10}")

        if "sqlite-utils" in available_tools:
            t = bench_import_sqlite_utils(db_path, tmpdir)
            print(f"  {'sqlite-utils insert --csv':<35} {fmt(t):>10}")

        # --- Checksum ---
        print(f"\n  {'Checksum':<35} {'Time':>10}")
        print(f"  {'-' * 45}")

        t = bench_checksum_csvdb(db_path)
        print(f"  {'csvdb checksum (SQLite)':<35} {fmt(t):>10}")

        t = bench_checksum_csvdb(csvdb_dir)
        print(f"  {'csvdb checksum (csvdb dir)':<35} {fmt(t):>10}")


def main():
    global CSVDB_BIN
    CSVDB_BIN = find_csvdb()
    if not CSVDB_BIN:
        print("ERROR: csvdb binary not found. Run: cargo build --release -p csvdb",
              file=sys.stderr)
        sys.exit(1)

    print(f"csvdb binary: {CSVDB_BIN}")

    # Check external tools
    available_tools = set()
    for tool, cmd in [("sqlite3", ["sqlite3", "--version"]),
                      ("sqlite-utils", ["sqlite-utils", "--version"]),
                      ("duckdb", ["duckdb", "--version"])]:
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
            if result.returncode == 0:
                version = result.stdout.strip().split("\n")[0]
                available_tools.add(tool)
            else:
                version = "NOT FOUND"
        except FileNotFoundError:
            version = "NOT FOUND"
        print(f"{tool}: {version}")

    csvdb_version = subprocess.run(
        [CSVDB_BIN, "--version"], capture_output=True, text=True
    ).stdout.strip()
    print(f"csvdb: {csvdb_version}")

    for scale in SCALES:
        run_scale(scale, available_tools)

    print(f"\n{'=' * 60}")
    print("  Done.")
    print(f"{'=' * 60}")


if __name__ == "__main__":
    main()
