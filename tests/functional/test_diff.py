"""Functional tests for the diff command."""

import json
import sqlite3


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
        detail_lines = [line for line in lines if line.strip().startswith("+") or line.strip().startswith("~")]
        assert len(detail_lines) == 0


    def test_diff_normalizes_float_formatting(self, run_csvdb, temp_dir):
        """Diff ignores float formatting differences across formats (e.g. 32 vs 32.00)."""
        db_path = temp_dir / "floats.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price REAL)")
        conn.execute("INSERT INTO products VALUES (1, 'Widget', 9.99)")
        conn.execute("INSERT INTO products VALUES (2, 'Gadget', 24.50)")
        conn.execute("INSERT INTO products VALUES (3, 'Gizmo', 149.00)")
        conn.execute("INSERT INTO products VALUES (4, 'Thing', 32.00)")
        conn.commit()
        conn.close()

        csvdb_dir = temp_dir / "floats.csvdb"
        parquetdb_dir = temp_dir / "floats.parquetdb"
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb_dir), "--force")
        run_csvdb("to-parquetdb", str(db_path), "-o", str(parquetdb_dir), "--force")

        result = run_csvdb("diff", str(parquetdb_dir), str(csvdb_dir))
        assert result.returncode == 0
        assert "identical" in result.stdout

    def test_diff_detects_real_change_despite_float_noise(self, run_csvdb, temp_dir):
        """Diff still detects actual value changes even when float noise is present."""
        db_path = temp_dir / "base.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price REAL, category_id INTEGER)")
        conn.execute("INSERT INTO products VALUES (1, 'Widget', 9.99, 1)")
        conn.execute("INSERT INTO products VALUES (2, 'Gadget', 24.50, 1)")
        conn.commit()
        conn.close()

        # Export to parquetdb (float formatting may differ)
        parquetdb_dir = temp_dir / "base.parquetdb"
        run_csvdb("to-parquetdb", str(db_path), "-o", str(parquetdb_dir), "--force")

        # Export to csvdb, then manually change category_id
        csvdb_dir = temp_dir / "base.csvdb"
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb_dir), "--force")

        csv_path = csvdb_dir / "products.csv"
        content = csv_path.read_text()
        # Change category_id from 1 to 99 for Gadget (id=2)
        content = content.replace('"2","Gadget","24.5","1"', '"2","Gadget","24.5","99"')
        csv_path.write_text(content)

        result = run_csvdb("diff", str(parquetdb_dir), str(csvdb_dir), check=False)
        assert result.returncode == 1
        assert "1 modified" in result.stdout
        assert "category_id" in result.stdout


    def test_diff_format_json_identical(self, run_csvdb, temp_dir):
        """--format json outputs valid JSON for identical databases."""
        db_path = temp_dir / "same.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'Alice')")
        conn.commit()
        conn.close()

        csvdb1 = temp_dir / "a.csvdb"
        csvdb2 = temp_dir / "b.csvdb"
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb1), "--force")
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb2), "--force")

        result = run_csvdb("diff", str(csvdb1), str(csvdb2), "--format", "json")
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert data["has_differences"] is False
        assert len(data["tables"]) == 1
        assert data["tables"][0]["status"] == "identical"

    def test_diff_format_json_with_changes(self, run_csvdb, temp_dir):
        """--format json outputs structured diff data for modified databases."""
        left_db = temp_dir / "left.sqlite"
        conn = sqlite3.connect(left_db)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'Alice')")
        conn.execute("INSERT INTO t VALUES (2, 'Bob')")
        conn.commit()
        conn.close()

        right_db = temp_dir / "right.sqlite"
        conn = sqlite3.connect(right_db)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'Alicia')")
        conn.execute("INSERT INTO t VALUES (3, 'Charlie')")
        conn.commit()
        conn.close()

        left_csvdb = temp_dir / "left.csvdb"
        right_csvdb = temp_dir / "right.csvdb"
        run_csvdb("to-csvdb", str(left_db), "-o", str(left_csvdb), "--force")
        run_csvdb("to-csvdb", str(right_db), "-o", str(right_csvdb), "--force")

        result = run_csvdb("diff", str(left_csvdb), str(right_csvdb), "--format", "json", check=False)
        assert result.returncode == 1
        data = json.loads(result.stdout)
        assert data["has_differences"] is True
        table = data["tables"][0]
        assert table["status"] == "modified"
        assert table["rows"]["added"] == 1
        assert table["rows"]["deleted"] == 1
        assert table["rows"]["modified"] == 1
        assert len(table["changes"]) == 3

    def test_diff_format_json_summary(self, run_csvdb, temp_dir):
        """--format json --summary outputs counts without changes."""
        left_db = temp_dir / "left.sqlite"
        conn = sqlite3.connect(left_db)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'a')")
        conn.commit()
        conn.close()

        right_db = temp_dir / "right.sqlite"
        conn = sqlite3.connect(right_db)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'b')")
        conn.commit()
        conn.close()

        left_csvdb = temp_dir / "left.csvdb"
        right_csvdb = temp_dir / "right.csvdb"
        run_csvdb("to-csvdb", str(left_db), "-o", str(left_csvdb), "--force")
        run_csvdb("to-csvdb", str(right_db), "-o", str(right_csvdb), "--force")

        result = run_csvdb("diff", str(left_csvdb), str(right_csvdb), "--format", "json", "--summary", check=False)
        assert result.returncode == 1
        data = json.loads(result.stdout)
        assert data["has_differences"] is True
        table = data["tables"][0]
        assert table["rows"]["modified"] == 1
        assert len(table["changes"]) == 0


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
