"""Functional tests for the merge command."""

import json
import sqlite3


class TestMerge:
    def _make_csvdb(self, run_csvdb, temp_dir, name, tables):
        """Create a csvdb from a sqlite database.

        tables: dict of table_name -> (schema_sql, rows)
        where rows is a list of tuples.
        """
        db_path = temp_dir / f"{name}.sqlite"
        conn = sqlite3.connect(db_path)
        for table_name, (schema_sql, rows) in tables.items():
            conn.execute(schema_sql)
            if rows:
                placeholders = ",".join(["?"] * len(rows[0]))
                for row in rows:
                    conn.execute(
                        f"INSERT INTO {table_name} VALUES ({placeholders})", row
                    )
        conn.commit()
        conn.close()

        csvdb_dir = temp_dir / f"{name}.csvdb"
        run_csvdb("to-csvdb", str(db_path), "-o", str(csvdb_dir), "--force")
        return csvdb_dir

    def test_merge_identical(self, run_csvdb, temp_dir):
        """Merging three identical databases produces no changes."""
        tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice"), (2, "Bob")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge", str(base), str(left), str(right), "-o", str(output)
        )
        assert result.returncode == 0
        assert "identical" in result.stdout

    def test_merge_left_modify(self, run_csvdb, temp_dir):
        """Left-side modification is taken when right is unchanged."""
        base_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice"), (2, "Bob")],
            )
        }
        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alicia"), (2, "Bob")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", base_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge", str(base), str(left), str(right), "-o", str(output)
        )
        assert result.returncode == 0
        assert "merged" in result.stdout

    def test_merge_conflict(self, run_csvdb, temp_dir):
        """Modify/modify conflict is detected and reported."""
        base_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice")],
            )
        }
        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alicia")],
            )
        }
        right_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Ally")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", right_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(base),
            str(left),
            str(right),
            "-o",
            str(output),
            check=False,
        )
        assert result.returncode == 1
        assert "CONFLICT" in result.stdout

    def test_merge_strategy_ours(self, run_csvdb, temp_dir):
        """--strategy ours resolves conflicts with left values."""
        base_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice")],
            )
        }
        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alicia")],
            )
        }
        right_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Ally")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", right_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(base),
            str(left),
            str(right),
            "-o",
            str(output),
            "--strategy",
            "ours",
        )
        assert result.returncode == 0
        assert "merged" in result.stdout

    def test_merge_strategy_theirs(self, run_csvdb, temp_dir):
        """--strategy theirs resolves conflicts with right values."""
        base_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice")],
            )
        }
        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alicia")],
            )
        }
        right_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Ally")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", right_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(base),
            str(left),
            str(right),
            "-o",
            str(output),
            "--strategy",
            "theirs",
        )
        assert result.returncode == 0
        assert "merged" in result.stdout

    def test_merge_json_format(self, run_csvdb, temp_dir):
        """--format json outputs valid JSON merge report."""
        tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(base),
            str(left),
            str(right),
            "-o",
            str(output),
            "--format",
            "json",
        )
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert data["has_conflicts"] is False
        assert len(data["tables"]) == 1
        assert data["tables"][0]["status"] == "identical"

    def test_merge_json_with_conflicts(self, run_csvdb, temp_dir):
        """--format json shows conflicts in structured output."""
        base_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice")],
            )
        }
        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alicia")],
            )
        }
        right_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Ally")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", right_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(base),
            str(left),
            str(right),
            "-o",
            str(output),
            "--format",
            "json",
            check=False,
        )
        assert result.returncode == 1
        data = json.loads(result.stdout)
        assert data["has_conflicts"] is True
        assert data["tables"][0]["status"] == "conflict"
        assert len(data["tables"][0]["conflicts"]) == 1
        assert data["tables"][0]["conflicts"][0]["kind"] == "modify_modify"

    def test_merge_cross_format(self, run_csvdb, temp_dir):
        """Merge works with sqlite base and csvdb branches."""
        db_path = temp_dir / "base.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'Alice')")
        conn.commit()
        conn.close()

        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice"), (2, "Bob")],
            )
        }
        right_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice"), (3, "Charlie")],
            )
        }
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", right_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(db_path),
            str(left),
            str(right),
            "-o",
            str(output),
        )
        assert result.returncode == 0
        assert "merged" in result.stdout

    def test_merge_table_filter(self, run_csvdb, temp_dir):
        """--tables flag limits which tables are merged."""
        base_tables = {
            "t1": (
                "CREATE TABLE t1 (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice")],
            ),
            "t2": (
                "CREATE TABLE t2 (id INTEGER PRIMARY KEY, val TEXT)",
                [(1, "x")],
            ),
        }
        left_tables = {
            "t1": (
                "CREATE TABLE t1 (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alicia")],
            ),
            "t2": (
                "CREATE TABLE t2 (id INTEGER PRIMARY KEY, val TEXT)",
                [(1, "y")],
            ),
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", base_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(base),
            str(left),
            str(right),
            "-o",
            str(output),
            "--tables",
            "t1",
        )
        assert result.returncode == 0
        # Only t1 should appear in output
        assert "t1" in result.stdout
        assert "t2" not in result.stdout

    def test_merge_adds_from_both_sides(self, run_csvdb, temp_dir):
        """Additions from both sides are included."""
        base_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice")],
            )
        }
        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice"), (2, "Bob")],
            )
        }
        right_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice"), (3, "Charlie")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", right_tables)
        output = temp_dir / "merged.csvdb"

        result = run_csvdb(
            "merge",
            str(base),
            str(left),
            str(right),
            "-o",
            str(output),
            "--format",
            "json",
        )
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert data["tables"][0]["rows_merged"] == 3

    def test_merge_output_is_valid_csvdb(self, run_csvdb, temp_dir):
        """Merged output passes validation."""
        base_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alice"), (2, "Bob")],
            )
        }
        left_tables = {
            "t": (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
                [(1, "Alicia"), (2, "Bob"), (3, "Charlie")],
            )
        }
        base = self._make_csvdb(run_csvdb, temp_dir, "base", base_tables)
        left = self._make_csvdb(run_csvdb, temp_dir, "left", left_tables)
        right = self._make_csvdb(run_csvdb, temp_dir, "right", base_tables)
        output = temp_dir / "merged.csvdb"

        run_csvdb(
            "merge", str(base), str(left), str(right), "-o", str(output)
        )
        # Validate the merged output
        result = run_csvdb("validate", str(output))
        assert result.returncode == 0
