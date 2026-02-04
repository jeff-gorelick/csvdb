"""Functional tests for the diff command."""

import sqlite3
from pathlib import Path


class TestDiff:
    def test_diff_identical_databases(self, run_csvdb, temp_dir):
        """Reports 'identical' for two identical databases."""
        db_path = temp_dir / "same.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'Alice')")
        conn.execute("INSERT INTO t VALUES (2, 'Bob')")
        conn.commit()
        conn.close()

        # Export to two csvdb dirs
        csvdb1 = temp_dir / "a.csvdb"
        csvdb2 = temp_dir / "b.csvdb"
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb1), "--force")
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb2), "--force")

        result = run_csvdb("diff", str(csvdb1), str(csvdb2))
        assert result.returncode == 0
        assert "identical" in result.stdout

    def test_diff_added_deleted_modified_rows(self, run_csvdb, temp_dir):
        """Shows added, deleted, and modified rows."""
        # Create left database
        left_db = temp_dir / "left.sqlite"
        conn = sqlite3.connect(left_db)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score INTEGER)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice', 95)")
        conn.execute("INSERT INTO users VALUES (2, 'Bob', 87)")
        conn.execute("INSERT INTO users VALUES (3, 'Charlie', 92)")
        conn.commit()
        conn.close()

        # Create right database (modified)
        right_db = temp_dir / "right.sqlite"
        conn = sqlite3.connect(right_db)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score INTEGER)")
        conn.execute("INSERT INTO users VALUES (1, 'Alicia', 95)")   # modified name
        conn.execute("INSERT INTO users VALUES (3, 'Charlie', 98)")  # modified score
        conn.execute("INSERT INTO users VALUES (4, 'Eve', 88)")      # added
        # Bob (id=2) deleted
        conn.commit()
        conn.close()

        # Export both
        left_csvdb = temp_dir / "left.csvdb"
        right_csvdb = temp_dir / "right.csvdb"
        run_csvdb("to-csvdb", str(left_db), "-o", str(left_csvdb), "--force")
        run_csvdb("to-csvdb", str(right_db), "-o", str(right_csvdb), "--force")

        result = run_csvdb("diff", str(left_csvdb), str(right_csvdb), check=False)
        assert result.returncode == 1  # has differences
        assert "added" in result.stdout
        assert "deleted" in result.stdout
        assert "modified" in result.stdout

    def test_diff_cross_format(self, run_csvdb, temp_dir):
        """Diff works between sqlite and csvdb formats."""
        db_path = temp_dir / "cross.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'hello')")
        conn.commit()
        conn.close()

        csvdb_dir = temp_dir / "cross.csvdb"
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb_dir), "--force")

        result = run_csvdb("diff", str(db_path), str(csvdb_dir))
        assert result.returncode == 0
        assert "identical" in result.stdout

    def test_diff_summary_mode(self, run_csvdb, temp_dir):
        """--summary shows only counts, not individual rows."""
        left_db = temp_dir / "left.sqlite"
        conn = sqlite3.connect(left_db)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'a')")
        conn.commit()
        conn.close()

        right_db = temp_dir / "right.sqlite"
        conn = sqlite3.connect(right_db)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'b')")  # modified
        conn.execute("INSERT INTO t VALUES (2, 'c')")  # added
        conn.commit()
        conn.close()

        left_csvdb = temp_dir / "left.csvdb"
        right_csvdb = temp_dir / "right.csvdb"
        run_csvdb("to-csvdb", str(left_db), "-o", str(left_csvdb), "--force")
        run_csvdb("to-csvdb", str(right_db), "-o", str(right_csvdb), "--force")

        result = run_csvdb("diff", str(left_csvdb), str(right_csvdb), "--summary", check=False)
        assert result.returncode == 1
        # Summary should have counts but no detail lines (no + or ~ prefixed lines)
        assert "added" in result.stdout
        lines = result.stdout.strip().split("\n")
        detail_lines = [l for l in lines if l.strip().startswith("+") or l.strip().startswith("~")]
        assert len(detail_lines) == 0


class TestDiffEdgeCases:
    def test_diff_empty_tables(self, run_csvdb, temp_dir):
        """Both sides have 0-row tables -> identical."""
        db_path = temp_dir / "empty.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.commit()
        conn.close()

        a = temp_dir / "a.csvdb"
        b = temp_dir / "b.csvdb"
        run_csvdb("to-csvdb", str(db_path), "-o", str(a), "--force")
        run_csvdb("to-csvdb", str(db_path), "-o", str(b), "--force")

        result = run_csvdb("diff", str(a), str(b))
        assert result.returncode == 0
        assert "identical" in result.stdout

    def test_diff_table_only_in_left(self, run_csvdb, temp_dir):
        """Table removed on right side -> 'removed table' output."""
        left_db = temp_dir / "left.sqlite"
        conn = sqlite3.connect(left_db)
        conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t1 VALUES (1)")
        conn.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t2 VALUES (1)")
        conn.commit()
        conn.close()

        right_db = temp_dir / "right.sqlite"
        conn = sqlite3.connect(right_db)
        conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t1 VALUES (1)")
        conn.commit()
        conn.close()

        left_csvdb = temp_dir / "left.csvdb"
        right_csvdb = temp_dir / "right.csvdb"
        run_csvdb("to-csvdb", str(left_db), "-o", str(left_csvdb), "--force")
        run_csvdb("to-csvdb", str(right_db), "-o", str(right_csvdb), "--force")

        result = run_csvdb("diff", str(left_csvdb), str(right_csvdb), check=False)
        assert result.returncode == 1
        assert "removed table" in result.stdout

    def test_diff_table_only_in_right(self, run_csvdb, temp_dir):
        """Table added on right side -> 'added table' output."""
        left_db = temp_dir / "left.sqlite"
        conn = sqlite3.connect(left_db)
        conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t1 VALUES (1)")
        conn.commit()
        conn.close()

        right_db = temp_dir / "right.sqlite"
        conn = sqlite3.connect(right_db)
        conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t1 VALUES (1)")
        conn.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t2 VALUES (1)")
        conn.commit()
        conn.close()

        left_csvdb = temp_dir / "left.csvdb"
        right_csvdb = temp_dir / "right.csvdb"
        run_csvdb("to-csvdb", str(left_db), "-o", str(left_csvdb), "--force")
        run_csvdb("to-csvdb", str(right_db), "-o", str(right_csvdb), "--force")

        result = run_csvdb("diff", str(left_csvdb), str(right_csvdb), check=False)
        assert result.returncode == 1
        assert "added table" in result.stdout
