#!/usr/bin/env python3
"""Basic usage examples for csvdb Python bindings.

Run from repo root:
    cd csvdb-python && uv run maturin develop --release && uv run python examples/basic_usage.py
"""

import tempfile
import os
import sqlite3
from pathlib import Path

import csvdb


def create_sample_db(path):
    """Create a sample SQLite database for demos."""
    conn = sqlite3.connect(path)
    conn.execute("""
        CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT
        )
    """)
    conn.execute("""
        CREATE TABLE orders (
            id INTEGER PRIMARY KEY,
            user_id INTEGER NOT NULL REFERENCES users(id),
            product TEXT NOT NULL,
            amount REAL NOT NULL
        )
    """)
    conn.executemany(
        "INSERT INTO users VALUES (?, ?, ?)",
        [
            (1, "Alice", "alice@example.com"),
            (2, "Bob", "bob@example.com"),
            (3, "Charlie", None),
        ],
    )
    conn.executemany(
        "INSERT INTO orders VALUES (?, ?, ?, ?)",
        [
            (1, 1, "Widget", 9.99),
            (2, 1, "Gadget", 24.99),
            (3, 2, "Widget", 9.99),
            (4, 3, "Gizmo", 49.99),
        ],
    )
    conn.commit()
    conn.close()


def main():
    print(f"csvdb version: {csvdb.version()}\n")

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "shop.sqlite")
        create_sample_db(db_path)

        # --- Convert SQLite to .csvdb ---
        csvdb_path = csvdb.to_csvdb(db_path, force=True)
        print(f"Created csvdb: {csvdb_path}")
        print(f"  Files: {os.listdir(csvdb_path)}")

        # --- Checksum ---
        h = csvdb.checksum(csvdb_path)
        print(f"\nChecksum: {h}")

        # --- Validate ---
        info = csvdb.validate(csvdb_path)
        print(f"\nValidation: {info['table_count']} tables, "
              f"{len(info['errors'])} errors, "
              f"{len(info['warnings'])} warnings")

        # --- SQL query ---
        rows = csvdb.sql(csvdb_path, """
            SELECT u.name, SUM(o.amount) AS total
            FROM users u
            JOIN orders o ON u.id = o.user_id
            GROUP BY u.name
            ORDER BY total DESC
        """)
        print("\nSpending by customer:")
        for row in rows:
            print(f"  {row['name']}: ${row['total']}")

        # --- Diff (identical) ---
        has_diff = csvdb.diff(csvdb_path, csvdb_path)
        print(f"\nDiff against itself: {'differences found' if has_diff else 'identical'}")

        # --- Convert to other formats ---
        sqlite_path = csvdb.to_sqlite(csvdb_path, force=True)
        print(f"\nConverted to SQLite: {sqlite_path}")

        duckdb_path = csvdb.to_duckdb(csvdb_path, force=True)
        print(f"Converted to DuckDB: {duckdb_path}")

        # --- Query the DuckDB file directly ---
        rows = csvdb.sql(duckdb_path, "SELECT COUNT(*) AS n FROM orders")
        print(f"Orders in DuckDB: {rows[0]['n']}")

        # --- Selective export (only users table) ---
        partial = csvdb.to_csvdb(
            db_path, output=os.path.join(tmpdir, "users_only.csvdb"),
            tables=["users"], force=True,
        )
        print(f"\nPartial export (users only): {os.listdir(partial)}")

        # --- NULL handling ---
        rows = csvdb.sql(csvdb_path, "SELECT name, email FROM users ORDER BY id")
        print("\nNULL handling:")
        for row in rows:
            print(f"  {row['name']}: email={row['email']!r}")

        # --- Incremental export ---
        result = csvdb.to_csvdb_incremental(db_path)
        print(f"\nIncremental export: {result['updated']} updated, "
              f"{result['unchanged']} unchanged")


if __name__ == "__main__":
    main()
