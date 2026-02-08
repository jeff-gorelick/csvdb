"""Functional tests for the sql command."""

import sqlite3
from pathlib import Path


class TestSql:
    """Tests for the sql command."""

    def test_sql_from_sqlite(self, run_csvdb, sample_sqlite):
        """sql should query a SQLite database."""
        result = run_csvdb(
            "sql", "SELECT name FROM users ORDER BY id", str(sample_sqlite),
            "--format", "csv"
        )
        assert "Alice" in result.stdout
        assert "Bob" in result.stdout
        assert "Charlie" in result.stdout

    def test_sql_from_duckdb(self, run_csvdb, sample_sqlite, temp_dir):
        """sql should query a DuckDB database."""
        # First create a DuckDB from the sample
        run_csvdb("to-csvdb", str(sample_sqlite))
        csvdb_dir = sample_sqlite.parent / "sample.csvdb"
        run_csvdb("to-duckdb", str(csvdb_dir), "--force")
        duckdb_path = sample_sqlite.parent / "sample.duckdb"

        result = run_csvdb(
            "sql", "SELECT count(*) as cnt FROM users", str(duckdb_path),
            "--format", "csv"
        )
        assert "3" in result.stdout

    def test_sql_from_csvdb(self, run_csvdb, sample_csvdb):
        """sql should query a .csvdb directory."""
        result = run_csvdb(
            "sql", "SELECT name, price FROM items ORDER BY id", str(sample_csvdb),
            "--format", "csv"
        )
        assert "Widget" in result.stdout
        assert "Gadget" in result.stdout

    def test_sql_from_parquetdb(self, run_csvdb, sample_csvdb):
        """sql should query a .parquetdb directory."""
        run_csvdb("to-parquetdb", str(sample_csvdb), "--force")
        parquetdb_dir = sample_csvdb.parent / "sample.parquetdb"

        result = run_csvdb(
            "sql", "SELECT count(*) as cnt FROM items", str(parquetdb_dir),
            "--format", "csv"
        )
        assert "3" in result.stdout

    def test_sql_csv_output(self, run_csvdb, sample_sqlite):
        """--format csv should produce valid CSV with header."""
        result = run_csvdb(
            "sql", "SELECT id, name FROM users ORDER BY id", str(sample_sqlite),
            "--format", "csv"
        )
        lines = result.stdout.strip().split("\n")
        assert lines[0] == "id,name"
        assert lines[1] == "1,Alice"
        assert lines[2] == "2,Bob"
        assert lines[3] == "3,Charlie"

    def test_sql_table_output(self, run_csvdb, sample_sqlite):
        """--format table should produce a formatted table."""
        result = run_csvdb(
            "sql", "SELECT id, name FROM users ORDER BY id", str(sample_sqlite),
            "--format", "table"
        )
        assert "Alice" in result.stdout
        assert "Bob" in result.stdout
        # Row count on stderr
        assert "(3 rows)" in result.stderr

    def test_sql_rejects_non_select(self, run_csvdb, sample_sqlite):
        """sql should reject non-SELECT queries."""
        result = run_csvdb(
            "sql", "DROP TABLE users", str(sample_sqlite),
            "--format", "csv",
            check=False
        )
        assert result.returncode != 0
        assert "Only SELECT" in result.stderr

    def test_sql_rejects_insert(self, run_csvdb, sample_sqlite):
        """sql should reject INSERT queries."""
        result = run_csvdb(
            "sql", "INSERT INTO users VALUES (4, 'Dave', 100)", str(sample_sqlite),
            "--format", "csv",
            check=False
        )
        assert result.returncode != 0
        assert "Only SELECT" in result.stderr

    def test_sql_allows_with_cte(self, run_csvdb, sample_sqlite):
        """sql should allow WITH (CTE) queries."""
        result = run_csvdb(
            "sql",
            "WITH top AS (SELECT * FROM users WHERE score > 90) SELECT name FROM top ORDER BY name",
            str(sample_sqlite),
            "--format", "csv"
        )
        assert "Alice" in result.stdout
        assert "Charlie" in result.stdout

    def test_sql_bad_query(self, run_csvdb, sample_sqlite):
        """sql should report errors for invalid SQL."""
        result = run_csvdb(
            "sql", "SELECT * FROM nonexistent_table", str(sample_sqlite),
            "--format", "csv",
            check=False
        )
        assert result.returncode != 0

    def test_sql_null_handling_csv(self, run_csvdb, temp_dir):
        """sql should output empty fields for NULL in CSV mode."""
        db_path = temp_dir / "nulls.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'hello')")
        conn.execute("INSERT INTO t VALUES (2, NULL)")
        conn.commit()
        conn.close()

        result = run_csvdb(
            "sql", "SELECT id, val FROM t ORDER BY id", str(db_path),
            "--format", "csv"
        )
        lines = result.stdout.strip().split("\n")
        assert lines[1] == "1,hello"
        assert lines[2] == "2,"

    def test_sql_null_handling_table(self, run_csvdb, temp_dir):
        """sql should display NULL for null values in table mode."""
        db_path = temp_dir / "nulls.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO t VALUES (1, NULL)")
        conn.commit()
        conn.close()

        result = run_csvdb(
            "sql", "SELECT id, val FROM t ORDER BY id", str(db_path),
            "--format", "table"
        )
        assert "NULL" in result.stdout

    def test_sql_empty_result(self, run_csvdb, sample_sqlite):
        """sql should handle queries with no matching rows."""
        result = run_csvdb(
            "sql", "SELECT * FROM users WHERE id > 999", str(sample_sqlite),
            "--format", "csv"
        )
        lines = result.stdout.strip().split("\n")
        # Should have header only
        assert len(lines) == 1
        assert "id" in lines[0]

    def test_sql_aggregate(self, run_csvdb, sample_sqlite):
        """sql should handle aggregate queries."""
        result = run_csvdb(
            "sql", "SELECT count(*) as cnt, avg(score) as avg_score FROM users",
            str(sample_sqlite),
            "--format", "csv"
        )
        assert "cnt" in result.stdout
        assert "3" in result.stdout
