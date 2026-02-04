"""Functional tests for roundtrip conversions."""

import sqlite3
from pathlib import Path


class TestRoundtrip:
    """End-to-end roundtrip tests."""

    def test_full_roundtrip_sqlite(self, run_csvdb, temp_dir):
        """Full roundtrip: SQLite -> csvdb -> SQLite with checksum verification."""
        # Create original database
        original_db = temp_dir / "original.sqlite"
        conn = sqlite3.connect(original_db)
        conn.execute("""
            CREATE TABLE products (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                price REAL,
                quantity INTEGER
            )
        """)
        conn.executemany(
            "INSERT INTO products VALUES (?, ?, ?, ?)",
            [
                (1, "Apple", 1.50, 100),
                (2, "Banana", 0.75, 150),
                (3, "Cherry", 3.00, 50),
            ]
        )
        conn.commit()
        conn.close()

        # Get original checksum
        original_checksum = run_csvdb("checksum", str(original_db)).stdout.strip()

        # Convert to csvdb
        run_csvdb("to-csvdb", str(original_db))
        csvdb_dir = temp_dir / "original.csvdb"

        # Convert back to SQLite (different path)
        rebuilt_db = temp_dir / "rebuilt.sqlite"
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        # to-sqlite creates original.sqlite, so rename
        (temp_dir / "original.sqlite").replace(rebuilt_db)

        # Get rebuilt checksum
        rebuilt_checksum = run_csvdb("checksum", str(rebuilt_db)).stdout.strip()

        assert original_checksum == rebuilt_checksum

    def test_full_roundtrip_with_nulls(self, run_csvdb, temp_dir):
        """Roundtrip should preserve NULL values."""
        db_path = temp_dir / "nulls.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE data (
                id INTEGER PRIMARY KEY,
                optional_text TEXT,
                optional_int INTEGER
            )
        """)
        conn.execute("INSERT INTO data VALUES (1, 'has value', 42)")
        conn.execute("INSERT INTO data VALUES (2, NULL, NULL)")
        conn.execute("INSERT INTO data VALUES (3, 'another', NULL)")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Roundtrip
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "nulls.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "nulls.sqlite")).stdout.strip()

        assert original_checksum == rebuilt_checksum

    def test_full_roundtrip_duckdb(self, run_csvdb, temp_dir):
        """Full roundtrip: SQLite -> csvdb -> DuckDB with checksum verification."""
        # Create original database
        db_path = temp_dir / "source.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'one')")
        conn.execute("INSERT INTO t VALUES (2, 'two')")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Convert to csvdb, then to DuckDB
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "source.csvdb"))

        duckdb_checksum = run_csvdb("checksum", str(temp_dir / "source.duckdb")).stdout.strip()

        assert original_checksum == duckdb_checksum


class TestAdvancedRoundtrips:
    """Advanced multi-format roundtrip tests."""

    def test_duckdb_to_csv_to_sqlite_to_csv_to_duckdb(self, run_csvdb, temp_dir):
        """Full chain: DuckDB -> CSV -> SQLite -> CSV -> DuckDB with checksum at each step."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        # Start with DuckDB
        duck1_path = temp_dir / "chain_start.duckdb"
        conn = duckdb.connect(str(duck1_path))
        conn.execute("""
            CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                customer TEXT NOT NULL,
                amount REAL,
                quantity INTEGER
            )
        """)
        conn.executemany(
            "INSERT INTO orders VALUES (?, ?, ?, ?)",
            [
                (1, "Alice", 99.99, 2),
                (2, "Bob", 149.50, 1),
                (3, "Charlie", 29.99, 5),
                (4, "Diana", 199.00, 3),
            ]
        )
        conn.close()

        # Checkpoint 1: DuckDB checksum
        checksum_duck1 = run_csvdb("checksum", str(duck1_path)).stdout.strip()

        # Step 1: DuckDB -> CSV
        run_csvdb("to-csvdb", str(duck1_path))
        csvdb1_dir = temp_dir / "chain_start.csvdb"
        checksum_csv1 = run_csvdb("checksum", str(csvdb1_dir)).stdout.strip()
        assert checksum_duck1 == checksum_csv1, "DuckDB -> CSV failed"

        # Step 2: CSV -> SQLite
        run_csvdb("to-sqlite", "--force", str(csvdb1_dir))
        sqlite_path = temp_dir / "chain_start.sqlite"
        checksum_sqlite = run_csvdb("checksum", str(sqlite_path)).stdout.strip()
        assert checksum_csv1 == checksum_sqlite, "CSV -> SQLite failed"

        # Step 3: SQLite -> CSV (new directory)
        csvdb2_dir = temp_dir / "chain_middle.csvdb"
        run_csvdb("to-csvdb", "-o", str(csvdb2_dir), str(sqlite_path))
        checksum_csv2 = run_csvdb("checksum", str(csvdb2_dir)).stdout.strip()
        assert checksum_sqlite == checksum_csv2, "SQLite -> CSV failed"

        # Step 4: CSV -> DuckDB
        run_csvdb("to-duckdb", "--force", str(csvdb2_dir))
        duck2_path = temp_dir / "chain_middle.duckdb"
        checksum_duck2 = run_csvdb("checksum", str(duck2_path)).stdout.strip()
        assert checksum_csv2 == checksum_duck2, "CSV -> DuckDB failed"

        # Final verification: start == end
        assert checksum_duck1 == checksum_duck2, "Full chain checksum mismatch"

    def test_multi_table_roundtrip(self, run_csvdb, temp_dir):
        """Roundtrip with multiple related tables."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        # Create SQLite with multiple tables
        db_path = temp_dir / "multi_table.sqlite"
        conn = sqlite3.connect(db_path)

        conn.execute("""
            CREATE TABLE customers (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT
            )
        """)
        conn.execute("""
            CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                customer_id INTEGER,
                total REAL
            )
        """)
        conn.execute("""
            CREATE TABLE order_items (
                id INTEGER PRIMARY KEY,
                order_id INTEGER,
                product TEXT,
                quantity INTEGER,
                price REAL
            )
        """)

        conn.executemany("INSERT INTO customers VALUES (?, ?, ?)", [
            (1, "Alice", "alice@example.com"),
            (2, "Bob", "bob@example.com"),
        ])
        conn.executemany("INSERT INTO orders VALUES (?, ?, ?)", [
            (1, 1, 150.00),
            (2, 1, 75.50),
            (3, 2, 200.00),
        ])
        conn.executemany("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)", [
            (1, 1, "Widget", 2, 50.00),
            (2, 1, "Gadget", 1, 50.00),
            (3, 2, "Gizmo", 3, 25.00),
            (4, 3, "Widget", 4, 50.00),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # SQLite -> CSV -> DuckDB -> CSV -> SQLite
        run_csvdb("to-csvdb", str(db_path))
        csvdb1 = temp_dir / "multi_table.csvdb"
        assert (csvdb1 / "customers.csv").exists()
        assert (csvdb1 / "orders.csv").exists()
        assert (csvdb1 / "order_items.csv").exists()

        run_csvdb("to-duckdb", "--force", str(csvdb1))
        duck_path = temp_dir / "multi_table.duckdb"

        csvdb2 = temp_dir / "multi_rebuilt.csvdb"
        run_csvdb("to-csvdb", "-o", str(csvdb2), str(duck_path))

        run_csvdb("to-sqlite", "--force", str(csvdb2))
        rebuilt_path = temp_dir / "multi_rebuilt.sqlite"

        final_checksum = run_csvdb("checksum", str(rebuilt_path)).stdout.strip()
        assert original_checksum == final_checksum

    def test_complex_data_types_roundtrip(self, run_csvdb, temp_dir):
        """Roundtrip with edge case data values."""
        db_path = temp_dir / "complex_data.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE edge_cases (
                id INTEGER PRIMARY KEY,
                text_val TEXT,
                int_val INTEGER,
                real_val REAL
            )
        """)
        # Note: Using limited precision floats since DuckDB REAL has ~7 digits of precision
        conn.executemany("INSERT INTO edge_cases VALUES (?, ?, ?, ?)", [
            (1, "normal text", 42, 3.14),
            (2, "", 0, 0.0),  # empty string, zeros
            (3, "with,comma", -999, -0.001),  # comma in text
            (4, 'with"quote', 2147483647, 1e10),  # quote in text, large numbers
            (5, None, None, None),  # NULLs
            (6, "   spaces   ", 1, 0.12345),  # leading/trailing spaces
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Full roundtrip through all formats
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "complex_data.csvdb"))
        run_csvdb("to-csvdb", "-o", str(temp_dir / "complex_rt.csvdb"),
                  str(temp_dir / "complex_data.duckdb"))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "complex_rt.csvdb"))

        final_checksum = run_csvdb("checksum", str(temp_dir / "complex_rt.sqlite")).stdout.strip()
        assert original_checksum == final_checksum

    def test_views_survive_full_chain(self, run_csvdb, temp_dir):
        """Views should survive the full DuckDB -> CSV -> SQLite -> CSV -> DuckDB chain."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        # Start with SQLite (easier to create views)
        db_path = temp_dir / "views_chain.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE employees (
                id INTEGER PRIMARY KEY,
                name TEXT,
                dept TEXT,
                salary INTEGER
            )
        """)
        conn.executemany("INSERT INTO employees VALUES (?, ?, ?, ?)", [
            (1, "Alice", "Eng", 100000),
            (2, "Bob", "Eng", 90000),
            (3, "Charlie", "Sales", 80000),
        ])
        conn.execute("CREATE VIEW engineers AS SELECT * FROM employees WHERE dept = 'Eng'")
        conn.execute("CREATE VIEW well_paid AS SELECT name, salary FROM employees WHERE salary >= 90000")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # SQLite -> CSV
        run_csvdb("to-csvdb", str(db_path))
        csvdb1 = temp_dir / "views_chain.csvdb"
        schema1 = (csvdb1 / "schema.sql").read_text()
        assert "engineers" in schema1
        assert "well_paid" in schema1

        # CSV -> DuckDB
        run_csvdb("to-duckdb", "--force", str(csvdb1))
        duck_path = temp_dir / "views_chain.duckdb"
        duck_checksum = run_csvdb("checksum", str(duck_path)).stdout.strip()
        assert original_checksum == duck_checksum

        # DuckDB -> CSV
        csvdb2 = temp_dir / "views_chain2.csvdb"
        run_csvdb("to-csvdb", "-o", str(csvdb2), str(duck_path))
        schema2 = (csvdb2 / "schema.sql").read_text()
        assert "engineers" in schema2
        assert "well_paid" in schema2

        # CSV -> SQLite
        run_csvdb("to-sqlite", "--force", str(csvdb2))
        sqlite2_path = temp_dir / "views_chain2.sqlite"
        final_checksum = run_csvdb("checksum", str(sqlite2_path)).stdout.strip()
        assert original_checksum == final_checksum

        # Verify views actually work
        conn = sqlite3.connect(sqlite2_path)
        engineers = conn.execute("SELECT * FROM engineers ORDER BY id").fetchall()
        well_paid = conn.execute("SELECT * FROM well_paid ORDER BY salary DESC").fetchall()
        conn.close()

        assert len(engineers) == 2
        assert engineers[0][1] == "Alice"
        assert len(well_paid) == 2

    def test_large_dataset_roundtrip(self, run_csvdb, temp_dir):
        """Roundtrip with larger dataset (10k rows)."""
        db_path = temp_dir / "large.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE records (
                id INTEGER PRIMARY KEY,
                name TEXT,
                value REAL,
                category INTEGER
            )
        """)

        # Insert 10k rows
        rows = [(i, f"item_{i}", i * 0.01, i % 10) for i in range(1, 10001)]
        conn.executemany("INSERT INTO records VALUES (?, ?, ?, ?)", rows)
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Full roundtrip
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "large.csvdb"))
        run_csvdb("to-csvdb", "-o", str(temp_dir / "large_rt.csvdb"),
                  str(temp_dir / "large.duckdb"))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "large_rt.csvdb"))

        final_checksum = run_csvdb("checksum", str(temp_dir / "large_rt.sqlite")).stdout.strip()
        assert original_checksum == final_checksum

        # Verify row count
        conn = sqlite3.connect(temp_dir / "large_rt.sqlite")
        count = conn.execute("SELECT COUNT(*) FROM records").fetchone()[0]
        conn.close()
        assert count == 10000
