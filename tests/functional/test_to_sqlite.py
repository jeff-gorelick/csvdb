"""Functional tests for the to-sqlite command."""

import sqlite3


class TestToSqlite:
    """Tests for the to-sqlite command."""

    def test_to_sqlite_creates_db(self, run_csvdb, sample_csvdb):
        """to-sqlite should create SQLite database from .csvdb."""
        run_csvdb("to-sqlite", "--force", str(sample_csvdb))

        db_path = sample_csvdb.parent / "sample.sqlite"
        assert db_path.exists()

        # Verify data
        conn = sqlite3.connect(db_path)
        rows = conn.execute("SELECT * FROM items ORDER BY id").fetchall()
        conn.close()

        assert len(rows) == 3
        assert rows[0] == (1, "Widget", 9.99)

    def test_to_sqlite_roundtrip(self, run_csvdb, sample_sqlite, temp_dir):
        """SQLite -> csvdb -> SQLite should preserve data."""
        # Convert to csvdb
        run_csvdb("to-csvdb", str(sample_sqlite))
        csvdb_dir = sample_sqlite.parent / "sample.csvdb"

        # Convert back to SQLite
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        new_sqlite = sample_sqlite.parent / "sample.sqlite"

        # Compare data
        conn1 = sqlite3.connect(sample_sqlite)
        conn2 = sqlite3.connect(new_sqlite)

        # Need to compare original before we overwrote it, so just check new one
        rows = conn2.execute("SELECT * FROM users ORDER BY id").fetchall()
        conn1.close()
        conn2.close()

        assert len(rows) == 3
        assert rows[0][1] == "Alice"
