"""Functional tests for the to-parquetdb command."""

import sqlite3
from pathlib import Path


class TestToParquetdb:
    """Tests for basic to-parquetdb functionality."""

    def test_sqlite_to_parquetdb(self, run_csvdb, sample_sqlite):
        """to-parquetdb should create .parquetdb dir with schema.sql, csvdb.toml, and .parquet files."""
        run_csvdb("to-parquetdb", str(sample_sqlite))

        parquetdb_dir = sample_sqlite.parent / "sample.parquetdb"
        assert parquetdb_dir.exists()
        assert (parquetdb_dir / "schema.sql").exists()
        assert (parquetdb_dir / "csvdb.toml").exists()

        parquet_files = list(parquetdb_dir.glob("*.parquet"))
        assert len(parquet_files) > 0
        assert (parquetdb_dir / "users.parquet").exists()

    def test_csvdb_to_parquetdb(self, run_csvdb, sample_csvdb):
        """to-parquetdb should convert from csvdb source."""
        run_csvdb("to-parquetdb", str(sample_csvdb))

        parquetdb_dir = sample_csvdb.parent / "sample.parquetdb"
        assert parquetdb_dir.exists()
        assert (parquetdb_dir / "schema.sql").exists()
        assert (parquetdb_dir / "items.parquet").exists()

    def test_duckdb_to_parquetdb(self, run_csvdb, temp_dir):
        """to-parquetdb should convert from duckdb source."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        duck_path = temp_dir / "source.duckdb"
        conn = duckdb.connect(str(duck_path))
        conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, price REAL)")
        conn.executemany("INSERT INTO items VALUES (?, ?, ?)", [
            (1, "Widget", 9.99),
            (2, "Gadget", 19.99),
        ])
        conn.close()

        run_csvdb("to-parquetdb", str(duck_path))

        parquetdb_dir = temp_dir / "source.parquetdb"
        assert parquetdb_dir.exists()
        assert (parquetdb_dir / "items.parquet").exists()

    def test_parquetdb_data_accessible(self, run_csvdb, sample_sqlite):
        """Data in parquetdb should be accessible via DuckDB."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        run_csvdb("to-parquetdb", str(sample_sqlite))

        parquetdb_dir = sample_sqlite.parent / "sample.parquetdb"
        parquet_file = parquetdb_dir / "users.parquet"

        conn = duckdb.connect()
        rows = conn.execute(f"SELECT * FROM read_parquet('{parquet_file}') ORDER BY id").fetchall()
        conn.close()

        assert len(rows) == 3
        assert rows[0][1] == "Alice"
        assert rows[1][1] == "Bob"
        assert rows[2][1] == "Charlie"

    def test_parquetdb_custom_output(self, run_csvdb, sample_sqlite, temp_dir):
        """to-parquetdb -o should use custom output directory."""
        output_dir = temp_dir / "custom_output.parquetdb"

        run_csvdb("to-parquetdb", "-o", str(output_dir), str(sample_sqlite))

        assert output_dir.exists()
        assert (output_dir / "schema.sql").exists()
        assert (output_dir / "users.parquet").exists()


class TestToParquetdbFlags:
    """Tests for to-parquetdb command flags."""

    def test_force_overwrites(self, run_csvdb, sample_sqlite):
        """Without --force should fail on existing dir; with --force should succeed."""
        run_csvdb("to-parquetdb", str(sample_sqlite))

        # Second run without --force should fail
        result = run_csvdb("to-parquetdb", str(sample_sqlite), check=False)
        assert result.returncode != 0

        # With --force should succeed
        run_csvdb("to-parquetdb", "--force", str(sample_sqlite))

        parquetdb_dir = sample_sqlite.parent / "sample.parquetdb"
        assert parquetdb_dir.exists()

    def test_pipe_outputs_path(self, run_csvdb, sample_sqlite):
        """--pipe should write to temp dir and output only the path."""
        result = run_csvdb("to-parquetdb", "--pipe", str(sample_sqlite))

        output = result.stdout.strip()
        assert not output.startswith("Created:")
        assert output.endswith(".parquetdb")
        assert Path(output).exists()

    def test_tables_filter(self, run_csvdb, temp_dir):
        """--tables should include only named tables."""
        db_path = temp_dir / "multi.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO orders VALUES (1, 99.99)")
        conn.commit()
        conn.close()

        run_csvdb("to-parquetdb", "--tables=users", str(db_path))

        parquetdb_dir = temp_dir / "multi.parquetdb"
        assert (parquetdb_dir / "users.parquet").exists()
        assert not (parquetdb_dir / "orders.parquet").exists()

    def test_exclude_filter(self, run_csvdb, temp_dir):
        """--exclude should skip named tables."""
        db_path = temp_dir / "multi.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO orders VALUES (1, 99.99)")
        conn.commit()
        conn.close()

        run_csvdb("to-parquetdb", "--exclude=orders", str(db_path))

        parquetdb_dir = temp_dir / "multi.parquetdb"
        assert (parquetdb_dir / "users.parquet").exists()
        assert not (parquetdb_dir / "orders.parquet").exists()


class TestToParquetdbNullAndOrder:
    """Tests for NULL handling and ordering modes in parquetdb."""

    def test_null_roundtrip(self, run_csvdb, temp_dir):
        """NULL values should be preserved through parquetdb."""
        db_path = temp_dir / "nulls.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)")
        conn.execute("INSERT INTO data VALUES (1, NULL, NULL)")
        conn.execute("INSERT INTO data VALUES (2, 'hello', 42)")
        conn.execute("INSERT INTO data VALUES (3, NULL, 7)")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # SQLite -> parquetdb -> csvdb -> SQLite
        run_csvdb("to-parquetdb", str(db_path))
        parquetdb_dir = temp_dir / "nulls.parquetdb"
        run_csvdb("to-csvdb", "-o", str(temp_dir / "nulls_rt.csvdb"), str(parquetdb_dir))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "nulls_rt.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "nulls_rt.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_order_all_columns(self, run_csvdb, temp_dir):
        """--order=all-columns should work with to-parquetdb."""
        db_path = temp_dir / "order_ac.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, timestamp TEXT, event_type TEXT)")
        conn.executemany("INSERT INTO events VALUES (?, ?, ?)", [
            (1, "2024-01-03", "click"),
            (2, "2024-01-01", "view"),
            (3, "2024-01-02", "click"),
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-parquetdb", "--order=all-columns", str(db_path))

        parquetdb_dir = temp_dir / "order_ac.parquetdb"
        assert parquetdb_dir.exists()
        assert (parquetdb_dir / "events.parquet").exists()

    def test_order_default_pk(self, run_csvdb, temp_dir):
        """Default --order=pk should order by primary key in parquetdb."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "order_pk.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [
            (3, "three"), (1, "one"), (2, "two")
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-parquetdb", str(db_path))

        parquetdb_dir = temp_dir / "order_pk.parquetdb"
        parquet_file = parquetdb_dir / "data.parquet"

        conn = duckdb.connect()
        rows = conn.execute(f"SELECT id, val FROM read_parquet('{parquet_file}')").fetchall()
        conn.close()

        ids = [row[0] for row in rows]
        assert ids == [1, 2, 3]


class TestToParquetdbEdgeCases:
    """Edge case tests for to-parquetdb."""

    def test_empty_table(self, run_csvdb, temp_dir):
        """Table with schema but no rows should be handled."""
        db_path = temp_dir / "empty.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, name TEXT)")
        conn.commit()
        conn.close()

        run_csvdb("to-parquetdb", str(db_path))

        parquetdb_dir = temp_dir / "empty.parquetdb"
        assert parquetdb_dir.exists()
        assert (parquetdb_dir / "data.parquet").exists()

    def test_views_preserved(self, run_csvdb, temp_dir):
        """Views should be written to schema.sql."""
        db_path = temp_dir / "views.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price REAL)")
        conn.executemany("INSERT INTO products VALUES (?, ?, ?)", [
            (1, "Cheap", 5.00),
            (2, "Expensive", 50.00),
        ])
        conn.execute("CREATE VIEW expensive AS SELECT * FROM products WHERE price > 10")
        conn.commit()
        conn.close()

        run_csvdb("to-parquetdb", str(db_path))

        schema = (temp_dir / "views.parquetdb" / "schema.sql").read_text()
        assert "expensive" in schema

    def test_real_precision_preserved(self, run_csvdb, temp_dir):
        """REAL values like 99.99 should survive without precision loss (validates REAL->DOUBLE fix)."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "precision.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, price REAL)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [
            (1, 99.99),
            (2, 0.1),
            (3, 3.14159265358979),
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-parquetdb", str(db_path))

        parquetdb_dir = temp_dir / "precision.parquetdb"
        parquet_file = parquetdb_dir / "data.parquet"

        conn = duckdb.connect()
        rows = conn.execute(
            f"SELECT id, price FROM read_parquet('{parquet_file}') ORDER BY id"
        ).fetchall()
        conn.close()

        assert rows[0][1] == 99.99
        assert rows[1][1] == 0.1
        assert abs(rows[2][1] - 3.14159265358979) < 1e-10
