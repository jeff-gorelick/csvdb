#!/usr/bin/env python3
"""Advanced usage examples for csvdb Python bindings.

Demonstrates format pipelines, change detection, schema initialization
from raw CSVs, cross-format queries, and data integrity verification.

Run from repo root:
    cd csvdb-python && uv run maturin develop --release && uv run python examples/advanced_usage.py
"""

import tempfile
import os
import sqlite3

import csvdb


def create_multi_table_db(path):
    """Create a database with multiple related tables and views."""
    conn = sqlite3.connect(path)
    conn.execute("PRAGMA foreign_keys = ON")
    conn.execute("""
        CREATE TABLE departments (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            budget REAL
        )
    """)
    conn.execute("""
        CREATE TABLE employees (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            dept_id INTEGER REFERENCES departments(id),
            salary REAL NOT NULL,
            hire_date TEXT NOT NULL
        )
    """)
    conn.execute("""
        CREATE TABLE projects (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            lead_id INTEGER REFERENCES employees(id),
            budget REAL,
            status TEXT NOT NULL DEFAULT 'active'
        )
    """)
    conn.execute("""
        CREATE VIEW dept_summary AS
        SELECT d.name AS department,
               COUNT(e.id) AS headcount,
               ROUND(AVG(e.salary), 2) AS avg_salary,
               d.budget
        FROM departments d
        LEFT JOIN employees e ON e.dept_id = d.id
        GROUP BY d.id, d.name, d.budget
    """)
    conn.executemany("INSERT INTO departments VALUES (?, ?, ?)", [
        (1, "Engineering", 500000),
        (2, "Sales", 300000),
        (3, "Research", 400000),
    ])
    conn.executemany("INSERT INTO employees VALUES (?, ?, ?, ?, ?)", [
        (1, "Alice",   1, 120000, "2020-01-15"),
        (2, "Bob",     1, 110000, "2020-06-01"),
        (3, "Charlie", 2,  95000, "2021-03-10"),
        (4, "Diana",   1, 130000, "2019-11-20"),
        (5, "Eve",     3, 115000, "2021-09-01"),
        (6, "Frank",   2,  90000, "2022-01-15"),
        (7, "Grace",   3, 125000, "2020-04-01"),
        (8, "Hank",    None, 100000, "2023-06-01"),
    ])
    conn.executemany("INSERT INTO projects VALUES (?, ?, ?, ?, ?)", [
        (1, "Platform v2",    1, 200000, "active"),
        (2, "Sales Portal",   3,  80000, "active"),
        (3, "ML Pipeline",    5, 150000, "active"),
        (4, "Legacy Cleanup", 4,  30000, "completed"),
    ])
    conn.commit()
    conn.close()


def section(title):
    print(f"\n{'=' * 60}")
    print(f"  {title}")
    print(f"{'=' * 60}\n")


def main():
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = os.path.join(tmpdir, "company.sqlite")
        create_multi_table_db(db_path)

        # ---------------------------------------------------------
        section("1. Format Pipeline: SQLite -> csvdb -> DuckDB -> Parquetdb")
        # ---------------------------------------------------------

        # Each format has different strengths:
        #   csvdb:     git-friendly, human-readable diffs
        #   DuckDB:    fast analytical queries
        #   parquetdb: columnar storage, efficient compression

        csvdb_path = csvdb.to_csvdb(db_path, force=True)
        duckdb_path = csvdb.to_duckdb(csvdb_path, force=True)
        parquet_path = csvdb.to_parquetdb(csvdb_path, force=True)

        print("Format pipeline complete:")
        print(f"  SQLite:    {os.path.basename(db_path)}")
        print(f"  csvdb:     {os.path.basename(csvdb_path)}/")
        print(f"  DuckDB:    {os.path.basename(duckdb_path)}")
        print(f"  Parquetdb: {os.path.basename(parquet_path)}/")

        # Checksums should match across all formats
        checksums = {}
        for label, path in [("csvdb", csvdb_path), ("DuckDB", duckdb_path),
                            ("Parquetdb", parquet_path), ("SQLite", db_path)]:
            checksums[label] = csvdb.checksum(path)

        all_match = len(set(checksums.values())) == 1
        print(f"\nChecksums {'match' if all_match else 'DIFFER'} across all formats")
        for label, h in checksums.items():
            print(f"  {label:10s}: {h[:16]}...")

        # ---------------------------------------------------------
        section("2. Cross-Format SQL Queries")
        # ---------------------------------------------------------

        # sql_query works on any supported format
        query = """
            SELECT d.name AS dept, COUNT(e.id) AS headcount,
                   ROUND(AVG(e.salary), 2) AS avg_salary
            FROM departments d
            LEFT JOIN employees e ON e.dept_id = d.id
            GROUP BY d.id, d.name
            ORDER BY avg_salary DESC
        """

        for label, path in [("csvdb", csvdb_path), ("DuckDB", duckdb_path),
                            ("Parquetdb", parquet_path)]:
            rows = csvdb.sql(path, query)
            print(f"{label} results:")
            for row in rows:
                print(f"  {row['dept']:15s}  {row['headcount']} people  "
                      f"avg ${row['avg_salary']}")
            print()

        # ---------------------------------------------------------
        section("3. Change Detection with Diff")
        # ---------------------------------------------------------

        # Make a change: add an employee and re-export
        conn = sqlite3.connect(db_path)
        conn.execute(
            "INSERT INTO employees VALUES (9, 'Ivy', 1, 140000, '2024-01-10')"
        )
        conn.commit()
        conn.close()

        csvdb_v2 = csvdb.to_csvdb(
            db_path, output=os.path.join(tmpdir, "company_v2.csvdb"), force=True,
        )

        has_diff = csvdb.diff(csvdb_path, csvdb_v2)
        print(f"v1 vs v2: {'differences found' if has_diff else 'identical'}")

        # Show what changed via SQL
        v1_count = csvdb.sql(csvdb_path, "SELECT COUNT(*) AS n FROM employees")
        v2_count = csvdb.sql(csvdb_v2, "SELECT COUNT(*) AS n FROM employees")
        print(f"  v1 employees: {v1_count[0]['n']}")
        print(f"  v2 employees: {v2_count[0]['n']}")

        # Checksum comparison
        h1 = csvdb.checksum(csvdb_path)
        h2 = csvdb.checksum(csvdb_v2)
        print(f"  v1 checksum: {h1[:16]}...")
        print(f"  v2 checksum: {h2[:16]}...")

        # ---------------------------------------------------------
        section("4. Incremental Export")
        # ---------------------------------------------------------

        # First export creates everything
        result = csvdb.to_csvdb_incremental(db_path)
        print(f"First incremental: path={os.path.basename(result['path'])}")
        print(f"  added={result['added']}, updated={result['updated']}, "
              f"unchanged={result['unchanged']}, removed={result['removed']}")

        # Second export with no changes - tables are unchanged
        result = csvdb.to_csvdb_incremental(db_path)
        print(f"\nSecond incremental (no changes):")
        print(f"  added={result['added']}, updated={result['updated']}, "
              f"unchanged={result['unchanged']}, removed={result['removed']}")

        # ---------------------------------------------------------
        section("5. Schema Init from Raw CSVs")
        # ---------------------------------------------------------

        # Create a directory with raw CSV files (no schema)
        raw_dir = os.path.join(tmpdir, "raw_csvs")
        os.makedirs(raw_dir)

        with open(os.path.join(raw_dir, "products.csv"), "w") as f:
            f.write("id,name,price,in_stock\n")
            f.write("1,Widget,9.99,true\n")
            f.write("2,Gadget,24.99,true\n")
            f.write("3,Discontinued,0.00,false\n")

        with open(os.path.join(raw_dir, "reviews.csv"), "w") as f:
            f.write("id,product_id,rating,comment\n")
            f.write("1,1,5,Great product\n")
            f.write("2,1,4,Good value\n")
            f.write('3,2,3,"OK, but pricey"\n')

        result = csvdb.init(raw_dir)
        print(f"Initialized: {os.path.basename(result['output_dir'])}")
        for table in result["tables"]:
            print(f"  {table['name']}: {table['row_count']} rows, "
                  f"{table['column_count']} cols, "
                  f"suggested PK: {table['suggested_pk']}")

        # Now we can query it
        rows = csvdb.sql(
            result["output_dir"],
            "SELECT name, price FROM products WHERE in_stock = 'true' ORDER BY price",
        )
        print("\nIn-stock products:")
        for row in rows:
            print(f"  {row['name']}: ${row['price']}")

        # ---------------------------------------------------------
        section("6. Selective Export and Table Filtering")
        # ---------------------------------------------------------

        # Export only specific tables
        eng_db = csvdb.to_csvdb(
            db_path,
            output=os.path.join(tmpdir, "eng_only.csvdb"),
            tables=["departments", "employees"],
            force=True,
        )
        info = csvdb.validate(eng_db)
        print(f"Selective export: {info['table_count']} tables")
        rows = csvdb.sql(eng_db, "SELECT COUNT(*) AS n FROM employees")
        print(f"  employees: {rows[0]['n']}")

        # Export everything except projects
        no_proj = csvdb.to_csvdb(
            db_path,
            output=os.path.join(tmpdir, "no_projects.csvdb"),
            exclude=["projects"],
            force=True,
        )
        info = csvdb.validate(no_proj)
        print(f"\nExclude export: {info['table_count']} tables")

        # ---------------------------------------------------------
        section("7. Validation and Error Handling")
        # ---------------------------------------------------------

        # Validate a good directory
        info = csvdb.validate(csvdb_path)
        print(f"Valid directory: {info['table_count']} tables, "
              f"{info['view_count']} views")

        # Demonstrate error handling
        try:
            csvdb.sql(csvdb_path, "DROP TABLE employees")
        except RuntimeError as e:
            print(f"\nBlocked write query: {e}")

        try:
            csvdb.checksum("/nonexistent/path.csvdb")
        except RuntimeError as e:
            print(f"Bad path error: {e}")


if __name__ == "__main__":
    main()
