"""Performance tests for csvdb with larger databases.

These tests verify csvdb handles larger datasets correctly and within
reasonable time bounds. Marked with @pytest.mark.slow so they can be
skipped in quick CI runs with: pytest -m "not slow"
"""

import sqlite3
import time

import pytest


def create_large_db(path, num_tables=5, rows_per_table=10_000):
    """Create a database with multiple tables and many rows."""
    conn = sqlite3.connect(path)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=OFF")

    for t in range(num_tables):
        table_name = f"table_{t:03d}"
        conn.execute(f"""
            CREATE TABLE "{table_name}" (
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
            f'INSERT INTO "{table_name}" VALUES (?, ?, ?, ?, ?, ?, ?)', rows
        )

    conn.commit()
    conn.close()


def create_wide_table_db(path, num_columns=50, num_rows=5_000):
    """Create a database with a single wide table."""
    conn = sqlite3.connect(path)

    cols = ", ".join(f'"col_{i:03d}" TEXT' for i in range(num_columns))
    conn.execute(f'CREATE TABLE wide (id INTEGER PRIMARY KEY, {cols})')

    placeholders = ", ".join(["?"] * (num_columns + 1))
    rows = [
        tuple([i] + [f"val_{i}_{c}" for c in range(num_columns)])
        for i in range(num_rows)
    ]
    conn.executemany(f"INSERT INTO wide VALUES ({placeholders})", rows)
    conn.commit()
    conn.close()


def create_null_heavy_db(path, num_rows=10_000):
    """Create a database where most values are NULL."""
    conn = sqlite3.connect(path)
    conn.execute("""
        CREATE TABLE sparse (
            id INTEGER PRIMARY KEY,
            a TEXT, b TEXT, c TEXT, d TEXT, e TEXT,
            f REAL, g REAL, h REAL,
            i INTEGER, j INTEGER
        )
    """)
    rows = [
        (
            i,
            f"a_{i}" if i % 10 == 0 else None,
            f"b_{i}" if i % 15 == 0 else None,
            f"c_{i}" if i % 20 == 0 else None,
            f"d_{i}" if i % 25 == 0 else None,
            f"e_{i}" if i % 50 == 0 else None,
            i * 0.1 if i % 5 == 0 else None,
            i * 0.01 if i % 8 == 0 else None,
            i * 0.001 if i % 12 == 0 else None,
            i if i % 3 == 0 else None,
            i if i % 7 == 0 else None,
        )
        for i in range(num_rows)
    ]
    conn.executemany("INSERT INTO sparse VALUES (?,?,?,?,?,?,?,?,?,?,?)", rows)
    conn.commit()
    conn.close()


class TestLargeDatabase:
    """Tests with 5 tables x 10k rows = 50k total rows."""

    @pytest.mark.slow
    def test_sqlite_to_csvdb(self, run_csvdb, temp_dir):
        """Convert 50k-row database to csvdb."""
        db_path = temp_dir / "large.sqlite"
        create_large_db(str(db_path))

        start = time.time()
        run_csvdb("to-csvdb", "--force", str(db_path))
        elapsed = time.time() - start

        csvdb_dir = temp_dir / "large.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "schema.sql").exists()

        # All 5 table CSVs should exist
        for t in range(5):
            assert (csvdb_dir / f"table_{t:03d}.csv").exists()

        print(f"\n  to-csvdb (50k rows): {elapsed:.2f}s")
        assert elapsed < 30, f"to-csvdb took {elapsed:.1f}s, expected < 30s"

    @pytest.mark.slow
    def test_csvdb_to_sqlite_roundtrip(self, run_csvdb, temp_dir):
        """Roundtrip 50k rows through csvdb and back to SQLite."""
        db_path = temp_dir / "rt.sqlite"
        create_large_db(str(db_path))

        run_csvdb("to-csvdb", "--force", str(db_path))
        csvdb_dir = temp_dir / "rt.csvdb"

        start = time.time()
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        elapsed = time.time() - start

        restored = temp_dir / "rt.sqlite"
        assert restored.exists()

        # Verify row counts
        conn = sqlite3.connect(restored)
        for t in range(5):
            count = conn.execute(f'SELECT COUNT(*) FROM "table_{t:03d}"').fetchone()[0]
            assert count == 10_000
        conn.close()

        print(f"\n  to-sqlite (50k rows): {elapsed:.2f}s")
        assert elapsed < 30, f"to-sqlite took {elapsed:.1f}s, expected < 30s"

    @pytest.mark.slow
    def test_csvdb_to_duckdb(self, run_csvdb, temp_dir):
        """Convert 50k-row csvdb to DuckDB."""
        db_path = temp_dir / "duck.sqlite"
        create_large_db(str(db_path))
        run_csvdb("to-csvdb", "--force", str(db_path))

        start = time.time()
        run_csvdb("to-duckdb", "--force", str(temp_dir / "duck.csvdb"))
        elapsed = time.time() - start

        assert (temp_dir / "duck.duckdb").exists()
        print(f"\n  to-duckdb (50k rows): {elapsed:.2f}s")
        assert elapsed < 30, f"to-duckdb took {elapsed:.1f}s, expected < 30s"

    @pytest.mark.slow
    def test_checksum_large(self, run_csvdb, temp_dir):
        """Checksum 50k-row database."""
        db_path = temp_dir / "cksum.sqlite"
        create_large_db(str(db_path))

        start = time.time()
        result = run_csvdb("checksum", str(db_path))
        elapsed = time.time() - start

        assert len(result.stdout.strip()) == 64
        print(f"\n  checksum (50k rows): {elapsed:.2f}s")
        assert elapsed < 30, f"checksum took {elapsed:.1f}s, expected < 30s"

    @pytest.mark.slow
    def test_checksum_deterministic(self, run_csvdb, temp_dir):
        """Checksum should be deterministic (same input = same hash)."""
        db_path = temp_dir / "deterministic.sqlite"
        create_large_db(str(db_path))

        hash1 = run_csvdb("checksum", str(db_path)).stdout.strip()
        hash2 = run_csvdb("checksum", str(db_path)).stdout.strip()
        assert hash1 == hash2, "Checksum not deterministic"

        # Export to csvdb and checksum that too
        run_csvdb("to-csvdb", "--force", str(db_path))
        csvdb_dir = temp_dir / "deterministic.csvdb"
        hash3 = run_csvdb("checksum", str(csvdb_dir)).stdout.strip()
        hash4 = run_csvdb("checksum", str(csvdb_dir)).stdout.strip()
        assert hash3 == hash4, "csvdb checksum not deterministic"

    @pytest.mark.slow
    def test_diff_large(self, run_csvdb, temp_dir):
        """Diff two 50k-row databases."""
        db_path = temp_dir / "diff.sqlite"
        create_large_db(str(db_path))
        run_csvdb("to-csvdb", "--force", str(db_path))
        csvdb_dir = temp_dir / "diff.csvdb"

        start = time.time()
        result = run_csvdb("diff", str(csvdb_dir), str(csvdb_dir), check=False)
        elapsed = time.time() - start

        assert result.returncode == 0  # identical
        print(f"\n  diff (50k rows, identical): {elapsed:.2f}s")
        assert elapsed < 30, f"diff took {elapsed:.1f}s, expected < 30s"

    @pytest.mark.slow
    def test_sql_query_large(self, run_csvdb, temp_dir):
        """SQL aggregation on 50k-row database."""
        db_path = temp_dir / "query.sqlite"
        create_large_db(str(db_path))

        start = time.time()
        result = run_csvdb(
            "sql",
            "SELECT category, COUNT(*) AS cnt, ROUND(AVG(value), 2) AS avg_val "
            "FROM table_000 GROUP BY category ORDER BY cnt DESC LIMIT 5",
            str(db_path),
        )
        elapsed = time.time() - start

        assert "cat_" in result.stdout
        print(f"\n  sql query (10k rows): {elapsed:.2f}s")
        assert elapsed < 30, f"sql query took {elapsed:.1f}s, expected < 30s"


class TestWideTable:
    """Tests with a 50-column table."""

    @pytest.mark.slow
    def test_wide_table_roundtrip(self, run_csvdb, temp_dir):
        """Roundtrip a 50-column, 5k-row table."""
        db_path = temp_dir / "wide.sqlite"
        create_wide_table_db(str(db_path))

        start = time.time()
        run_csvdb("to-csvdb", "--force", str(db_path))
        csvdb_dir = temp_dir / "wide.csvdb"
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        elapsed = time.time() - start

        conn = sqlite3.connect(temp_dir / "wide.sqlite")
        count = conn.execute("SELECT COUNT(*) FROM wide").fetchone()[0]
        cols = conn.execute("PRAGMA table_info(wide)").fetchall()
        conn.close()

        assert count == 5_000
        assert len(cols) == 51  # id + 50 columns

        print(f"\n  wide table roundtrip (51 cols x 5k rows): {elapsed:.2f}s")
        assert elapsed < 30, f"wide roundtrip took {elapsed:.1f}s, expected < 30s"


class TestNullHeavy:
    """Tests with sparse/NULL-heavy data."""

    @pytest.mark.slow
    def test_null_heavy_roundtrip(self, run_csvdb, temp_dir):
        """Roundtrip a table where most cells are NULL."""
        db_path = temp_dir / "sparse.sqlite"
        create_null_heavy_db(str(db_path))

        original_hash = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", "--force", str(db_path))
        csvdb_dir = temp_dir / "sparse.csvdb"
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))

        restored_hash = run_csvdb("checksum", str(temp_dir / "sparse.sqlite")).stdout.strip()
        assert original_hash == restored_hash, "NULL-heavy data changed during roundtrip"

    @pytest.mark.slow
    def test_null_heavy_duckdb(self, run_csvdb, temp_dir):
        """NULL-heavy data survives csvdb -> DuckDB -> csvdb roundtrip."""
        db_path = temp_dir / "sparse_duck.sqlite"
        create_null_heavy_db(str(db_path))

        original_hash = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", "--force", str(db_path))
        csvdb_dir = temp_dir / "sparse_duck.csvdb"
        run_csvdb("to-duckdb", "--force", str(csvdb_dir))

        duckdb_hash = run_csvdb("checksum", str(temp_dir / "sparse_duck.duckdb")).stdout.strip()
        assert original_hash == duckdb_hash


class TestManyTables:
    """Tests with many tables in a single database."""

    @pytest.mark.slow
    def test_20_tables(self, run_csvdb, temp_dir):
        """Handle a database with 20 tables."""
        db_path = temp_dir / "many.sqlite"
        create_large_db(str(db_path), num_tables=20, rows_per_table=1_000)

        start = time.time()
        run_csvdb("to-csvdb", "--force", str(db_path))
        elapsed = time.time() - start

        csvdb_dir = temp_dir / "many.csvdb"
        for t in range(20):
            assert (csvdb_dir / f"table_{t:03d}.csv").exists()

        # Validate
        result = run_csvdb("validate", str(csvdb_dir))
        assert "0 warnings" in result.stderr or result.returncode == 0

        print(f"\n  20 tables x 1k rows: {elapsed:.2f}s")
        assert elapsed < 30, f"20 tables took {elapsed:.1f}s, expected < 30s"

    @pytest.mark.slow
    def test_selective_export_performance(self, run_csvdb, temp_dir):
        """Exporting a subset of tables should be faster than all."""
        db_path = temp_dir / "selective.sqlite"
        create_large_db(str(db_path), num_tables=20, rows_per_table=1_000)

        # Export all
        start = time.time()
        run_csvdb("to-csvdb", "--force", str(db_path))
        all_time = time.time() - start

        # Export just 2 tables
        start = time.time()
        run_csvdb(
            "to-csvdb", "--force",
            "--tables", "table_000,table_001",
            "--output", str(temp_dir / "subset.csvdb"),
            str(db_path),
        )
        subset_time = time.time() - start

        csvdb_dir = temp_dir / "subset.csvdb"
        assert (csvdb_dir / "table_000.csv").exists()
        assert (csvdb_dir / "table_001.csv").exists()

        print(f"\n  all 20 tables: {all_time:.2f}s, 2 tables: {subset_time:.2f}s")
