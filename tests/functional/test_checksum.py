"""Functional tests for the checksum command."""

import sqlite3
from pathlib import Path


class TestChecksum:
    """Tests for the checksum command."""

    def test_checksum_csvdb(self, run_csvdb, sample_csvdb):
        """checksum should return hash for .csvdb directory."""
        result = run_csvdb("checksum", str(sample_csvdb))
        checksum = result.stdout.strip()

        assert len(checksum) == 64  # SHA-256 hex
        assert all(c in "0123456789abcdef" for c in checksum)

    def test_checksum_sqlite(self, run_csvdb, sample_sqlite):
        """checksum should return hash for SQLite database."""
        result = run_csvdb("checksum", str(sample_sqlite))
        checksum = result.stdout.strip()

        assert len(checksum) == 64

    def test_checksum_matches_across_formats(self, run_csvdb, sample_sqlite):
        """checksum should match for same data across formats."""
        # Get SQLite checksum
        sqlite_checksum = run_csvdb("checksum", str(sample_sqlite)).stdout.strip()

        # Convert to csvdb
        run_csvdb("to-csvdb", str(sample_sqlite))
        csvdb_dir = sample_sqlite.parent / "sample.csvdb"
        csvdb_checksum = run_csvdb("checksum", str(csvdb_dir)).stdout.strip()

        # Convert to DuckDB
        run_csvdb("to-duckdb", "--force", str(csvdb_dir))
        duckdb_path = sample_sqlite.parent / "sample.duckdb"
        duckdb_checksum = run_csvdb("checksum", str(duckdb_path)).stdout.strip()

        # All should match
        assert sqlite_checksum == csvdb_checksum, "SQLite != CSVDB"
        assert csvdb_checksum == duckdb_checksum, "CSVDB != DuckDB"

    def test_checksum_differs_for_different_data(self, run_csvdb, temp_dir):
        """checksum should differ for different data."""
        # Create two different databases
        db1 = temp_dir / "db1.sqlite"
        db2 = temp_dir / "db2.sqlite"

        conn1 = sqlite3.connect(db1)
        conn1.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)")
        conn1.execute("INSERT INTO t VALUES (1, 'hello')")
        conn1.commit()
        conn1.close()

        conn2 = sqlite3.connect(db2)
        conn2.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)")
        conn2.execute("INSERT INTO t VALUES (1, 'world')")
        conn2.commit()
        conn2.close()

        checksum1 = run_csvdb("checksum", str(db1)).stdout.strip()
        checksum2 = run_csvdb("checksum", str(db2)).stdout.strip()

        assert checksum1 != checksum2
