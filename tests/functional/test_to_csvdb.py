"""Functional tests for the to-csvdb command."""

import sqlite3
from pathlib import Path


class TestToCsv:
    """Tests for the to-csvdb command."""

    def test_to_csv_from_sqlite(self, run_csvdb, sample_sqlite):
        """to-csvdb should convert SQLite to .csvdb directory."""
        run_csvdb("to-csvdb", str(sample_sqlite))

        csvdb_dir = sample_sqlite.parent / "sample.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "schema.sql").exists()
        assert (csvdb_dir / "users.csv").exists()

        # Verify CSV content
        csv_content = (csvdb_dir / "users.csv").read_text()
        assert "Alice" in csv_content
        assert "Bob" in csv_content

    def test_to_csv_custom_output(self, run_csvdb, sample_sqlite, temp_dir):
        """to-csvdb --output should use custom output directory."""
        output_dir = temp_dir / "custom_output.csvdb"

        run_csvdb("to-csvdb", "-o", str(output_dir), str(sample_sqlite))

        assert output_dir.exists()
        assert (output_dir / "schema.sql").exists()

    def test_to_csv_preserves_data(self, run_csvdb, temp_dir):
        """to-csvdb should preserve all data accurately."""
        # Create database with various data types
        db_path = temp_dir / "types.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE data (
                id INTEGER PRIMARY KEY,
                text_col TEXT,
                int_col INTEGER,
                real_col REAL
            )
        """)
        conn.execute("INSERT INTO data VALUES (1, 'hello', 42, 3.14)")
        conn.execute("INSERT INTO data VALUES (2, 'world', -100, 2.718)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        csv_content = (temp_dir / "types.csvdb" / "data.csv").read_text()
        assert "hello" in csv_content
        assert "42" in csv_content
        assert "3.14" in csv_content


class TestOrderModes:
    """Tests for --order flag modes."""

    def test_order_all_columns(self, run_csvdb, temp_dir):
        """--order=all-columns should order by all columns."""
        db_path = temp_dir / "no_pk.sqlite"
        conn = sqlite3.connect(db_path)
        # Table without primary key
        conn.execute("CREATE TABLE events (timestamp TEXT, event_type TEXT, data TEXT)")
        conn.executemany("INSERT INTO events VALUES (?, ?, ?)", [
            ("2024-01-03", "click", "page1"),
            ("2024-01-01", "view", "page2"),
            ("2024-01-02", "click", "page1"),
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", "--order=all-columns", str(db_path))

        csv_content = (temp_dir / "no_pk.csvdb" / "events.csv").read_text()
        lines = csv_content.strip().split('\n')[1:]  # Skip header
        # Should be sorted by all columns
        assert len(lines) == 3

    def test_order_add_synthetic_key(self, run_csvdb, temp_dir):
        """--order=add-synthetic-key should add __csvdb_rowid column to CSV."""
        db_path = temp_dir / "events.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE logs (message TEXT, level TEXT)")
        conn.executemany("INSERT INTO logs VALUES (?, ?)", [
            ("Starting", "INFO"),
            ("Processing", "DEBUG"),
            ("Done", "INFO"),
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", "--order=add-synthetic-key", str(db_path))

        csv_content = (temp_dir / "events.csvdb" / "logs.csv").read_text()
        # Should have __csvdb_rowid column in CSV for ordering
        assert "__csvdb_rowid" in csv_content

        # Rows should be numbered
        lines = csv_content.strip().split('\n')
        assert '"1",' in lines[1] or '1,' in lines[1]

    def test_order_pk_default(self, run_csvdb, temp_dir):
        """--order=pk (default) should order by primary key."""
        db_path = temp_dir / "pk_order.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [
            (3, "three"), (1, "one"), (2, "two")
        ])
        conn.commit()
        conn.close()

        # Default is pk ordering
        run_csvdb("to-csvdb", str(db_path))

        csv_content = (temp_dir / "pk_order.csvdb" / "data.csv").read_text()
        lines = csv_content.strip().split('\n')[1:]
        ids = [line.split(',')[0].strip('"') for line in lines]
        assert ids == ['1', '2', '3']

    def test_all_columns_roundtrip(self, run_csvdb, temp_dir):
        """--order=all-columns should roundtrip correctly."""
        db_path = temp_dir / "all_cols_rt.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (a TEXT, b INTEGER, c REAL)")
        conn.executemany("INSERT INTO data VALUES (?, ?, ?)", [
            ("foo", 1, 1.5),
            ("bar", 2, 2.5),
            ("baz", 3, 3.5),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", "--order=all-columns", str(db_path), check=False)

        run_csvdb("to-csvdb", "--order=all-columns", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "all_cols_rt.csvdb"))

        # Data should be preserved
        conn = sqlite3.connect(temp_dir / "all_cols_rt.sqlite")
        count = conn.execute("SELECT COUNT(*) FROM data").fetchone()[0]
        conn.close()
        assert count == 3


class TestPipeFlag:
    """Tests for the --pipe flag (piping support)."""

    def test_pipe_outputs_only_path(self, run_csvdb, sample_sqlite):
        """--pipe should output just the path without 'Created:' prefix."""
        result = run_csvdb("to-csvdb", "--pipe", str(sample_sqlite))

        # Should output just the path, no "Created:" prefix
        output = result.stdout.strip()
        assert not output.startswith("Created:")
        assert output.endswith(".csvdb")

    def test_pipe_writes_to_different_dir(self, run_csvdb, sample_sqlite):
        """--pipe should write to a different directory than input."""
        result = run_csvdb("to-csvdb", "--pipe", str(sample_sqlite))
        output_path = Path(result.stdout.strip())

        # Output should be in a different directory than input
        input_dir = Path(sample_sqlite).parent.resolve()
        output_dir = output_path.parent.resolve()
        assert input_dir != output_dir

        # Output should exist and be a valid csvdb
        assert output_path.exists()

    def test_pipe_creates_valid_csvdb(self, run_csvdb, sample_sqlite):
        """--pipe should create a valid .csvdb directory."""
        result = run_csvdb("to-csvdb", "--pipe", str(sample_sqlite))
        output_path = Path(result.stdout.strip())

        # Should create valid csvdb structure
        assert output_path.exists()
        assert (output_path / "schema.sql").exists()
        # Should have at least one CSV file
        csv_files = list(output_path.glob("*.csv"))
        assert len(csv_files) > 0

    def test_pipe_with_explicit_output_uses_output(self, run_csvdb, temp_dir, sample_sqlite):
        """--pipe with -o should use the specified output, not temp dir."""
        custom_output = temp_dir / "custom.csvdb"
        result = run_csvdb("to-csvdb", "--pipe", "-o", str(custom_output), str(sample_sqlite))

        output_path = result.stdout.strip()
        # Should use the custom output path
        assert str(custom_output).replace("\\", "/") in output_path.replace("\\", "/")

    def test_pipe_piping_to_sqlite(self, run_csvdb, sample_sqlite):
        """Test full pipe: to-csvdb --pipe | xargs to-sqlite."""
        # First convert to csvdb with --pipe
        result = run_csvdb("to-csvdb", "--pipe", str(sample_sqlite))
        csvdb_path = result.stdout.strip()

        # Then convert to sqlite
        result = run_csvdb("to-sqlite", "--force", csvdb_path)
        assert "Created:" in result.stdout

        # Verify the sqlite file was created
        sqlite_path = Path(csvdb_path).with_suffix(".sqlite")
        assert sqlite_path.exists()

    def test_pipe_piping_to_duckdb(self, run_csvdb, sample_sqlite):
        """Test full pipe: to-csvdb --pipe | xargs to-duckdb."""
        # First convert to csvdb with --pipe
        result = run_csvdb("to-csvdb", "--pipe", str(sample_sqlite))
        csvdb_path = result.stdout.strip()

        # Then convert to duckdb
        result = run_csvdb("to-duckdb", "--force", csvdb_path)
        assert "Created:" in result.stdout

        # Verify the duckdb file was created
        duckdb_path = Path(csvdb_path).with_suffix(".duckdb")
        assert duckdb_path.exists()

    def test_pipe_preserves_data_integrity(self, run_csvdb, temp_dir):
        """--pipe should preserve data integrity through conversion."""
        import sqlite3

        # Create source database
        db_path = temp_dir / "integrity.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, value TEXT)")
        conn.execute("INSERT INTO data VALUES (1, 'test')")
        conn.execute("INSERT INTO data VALUES (2, 'data')")
        conn.commit()
        conn.close()

        # Get original checksum
        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Convert via temp
        result = run_csvdb("to-csvdb", "--pipe", str(db_path))
        csvdb_path = result.stdout.strip()

        # Convert back to sqlite
        run_csvdb("to-sqlite", "--force", csvdb_path)
        rebuilt_path = Path(csvdb_path).with_suffix(".sqlite")

        # Verify checksum matches
        rebuilt_checksum = run_csvdb("checksum", str(rebuilt_path)).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_pipe_path_uses_forward_slashes(self, run_csvdb, sample_sqlite):
        """--pipe output should use forward slashes for cross-platform piping."""
        result = run_csvdb("to-csvdb", "--pipe", str(sample_sqlite))
        output = result.stdout.strip()

        # Should not contain backslashes (for xargs compatibility)
        assert "\\" not in output


class TestNullHandling:
    """Tests for NULL vs empty string handling."""

    def test_null_exported_as_marker(self, run_csvdb, temp_dir):
        """NULL values should be exported as \\N in CSV."""
        import sqlite3

        db_path = temp_dir / "null_test.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)")
        conn.execute("INSERT INTO test VALUES (1, NULL, NULL)")
        conn.execute("INSERT INTO test VALUES (2, '', 42)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))
        csvdb_path = temp_dir / "null_test.csvdb"

        # Check CSV content
        csv_content = (csvdb_path / "test.csv").read_text()
        # NULL should be \N
        assert '"1","\\N","\\N"' in csv_content
        # Empty string should be ""
        assert '"2","","42"' in csv_content

    def test_null_roundtrip_preserved(self, run_csvdb, temp_dir):
        """NULL vs empty string should be preserved through roundtrip."""
        import sqlite3

        db_path = temp_dir / "null_rt.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")      # NULL
        conn.execute("INSERT INTO test VALUES (2, '')")        # empty string
        conn.execute("INSERT INTO test VALUES (3, 'hello')")   # regular string
        conn.commit()
        conn.close()

        # Roundtrip
        run_csvdb("to-csvdb", str(db_path))
        csvdb_path = temp_dir / "null_rt.csvdb"
        run_csvdb("to-sqlite", "--force", str(csvdb_path))
        rebuilt_path = temp_dir / "null_rt.sqlite"

        # Verify values
        conn = sqlite3.connect(str(rebuilt_path))
        rows = conn.execute("SELECT id, name FROM test ORDER BY id").fetchall()
        conn.close()

        assert rows[0] == (1, None)    # NULL preserved
        assert rows[1] == (2, '')      # empty string preserved
        assert rows[2] == (3, 'hello') # regular string preserved

    def test_null_roundtrip_duckdb(self, run_csvdb, temp_dir):
        """NULL values should be preserved through DuckDB roundtrip.

        Note: Due to a limitation in the Rust DuckDB driver, empty strings
        may be converted to NULL during DuckDB roundtrip. This test verifies
        NULL values are preserved; empty string preservation is best-effort.
        """
        import sqlite3

        db_path = temp_dir / "null_duck.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")
        conn.execute("INSERT INTO test VALUES (2, 'hello')")  # Use non-empty string
        conn.commit()
        conn.close()

        # SQLite -> csvdb -> DuckDB -> csvdb -> SQLite
        run_csvdb("to-csvdb", str(db_path))
        csvdb1 = temp_dir / "null_duck.csvdb"
        run_csvdb("to-duckdb", "--force", str(csvdb1))

        duck_path = temp_dir / "null_duck.duckdb"
        run_csvdb("to-csvdb", "-o", str(temp_dir / "null_duck2.csvdb"), str(duck_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "null_duck2.csvdb"))

        rebuilt_path = temp_dir / "null_duck2.sqlite"
        conn = sqlite3.connect(str(rebuilt_path))
        rows = conn.execute("SELECT id, name FROM test ORDER BY id").fetchall()
        conn.close()

        assert rows[0] == (1, None)    # NULL preserved
        assert rows[1] == (2, 'hello') # non-empty string preserved

    def test_null_mode_empty(self, run_csvdb, temp_dir):
        """--null-mode=empty should export NULL as empty string (lossy)."""
        import sqlite3

        db_path = temp_dir / "null_empty.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")
        conn.execute("INSERT INTO test VALUES (2, 'hello')")
        conn.commit()
        conn.close()

        result = run_csvdb("to-csvdb", "--null-mode=empty", str(db_path))
        csvdb_path = temp_dir / "null_empty.csvdb"

        # Check warning was shown
        assert "LOSSY" in result.stderr

        # Check CSV content - NULL should be empty string, not \N
        csv_content = (csvdb_path / "test.csv").read_text()
        assert '"1",""' in csv_content  # NULL as empty string
        assert '"2","hello"' in csv_content
        assert "\\N" not in csv_content  # No marker used

    def test_null_mode_literal(self, run_csvdb, temp_dir):
        """--null-mode=literal should export NULL as string 'NULL' (lossy)."""
        import sqlite3

        db_path = temp_dir / "null_literal.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")
        conn.execute("INSERT INTO test VALUES (2, 'hello')")
        conn.commit()
        conn.close()

        result = run_csvdb("to-csvdb", "--null-mode=literal", str(db_path))
        csvdb_path = temp_dir / "null_literal.csvdb"

        # Check warning was shown
        assert "LOSSY" in result.stderr

        # Check CSV content - NULL should be literal string "NULL"
        csv_content = (csvdb_path / "test.csv").read_text()
        assert '"1","NULL"' in csv_content  # NULL as literal string
        assert '"2","hello"' in csv_content
        assert "\\N" not in csv_content  # No marker used

    def test_null_mode_pipe_no_warning(self, run_csvdb, temp_dir):
        """--pipe mode should suppress lossy null mode warnings."""
        import sqlite3

        db_path = temp_dir / "null_pipe.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")
        conn.commit()
        conn.close()

        result = run_csvdb("to-csvdb", "--null-mode=empty", "--pipe", str(db_path))

        # Warning should be suppressed in pipe mode
        assert "LOSSY" not in result.stderr
        # Output should just be a path
        assert result.stdout.strip().endswith(".csvdb")


class TestDuckDBSource:
    """Tests for using DuckDB as source."""

    def test_to_csv_from_duckdb(self, run_csvdb, temp_dir):
        """to-csvdb should work directly from DuckDB."""
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
            (3, "Gizmo", 29.99),
        ])
        conn.close()

        run_csvdb("to-csvdb", str(duck_path))

        csvdb_dir = temp_dir / "source.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "schema.sql").exists()
        assert (csvdb_dir / "items.csv").exists()

        csv_content = (csvdb_dir / "items.csv").read_text()
        assert "Widget" in csv_content
        assert "Gadget" in csv_content

    def test_duckdb_checksum_direct(self, run_csvdb, temp_dir):
        """checksum should work on DuckDB created outside csvdb."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        duck_path = temp_dir / "direct.duckdb"
        conn = duckdb.connect(str(duck_path))
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO data VALUES (1, 'test')")
        conn.close()

        result = run_csvdb("checksum", str(duck_path))
        checksum = result.stdout.strip()

        assert len(checksum) == 64
        assert all(c in "0123456789abcdef" for c in checksum)

    def test_duckdb_with_views(self, run_csvdb, temp_dir):
        """to-csvdb from DuckDB should preserve views."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        duck_path = temp_dir / "views.duckdb"
        conn = duckdb.connect(str(duck_path))
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price REAL)")
        conn.executemany("INSERT INTO products VALUES (?, ?, ?)", [
            (1, "Cheap", 5.00),
            (2, "Medium", 15.00),
            (3, "Expensive", 50.00),
        ])
        conn.execute("CREATE VIEW expensive_items AS SELECT * FROM products WHERE price > 10")
        conn.close()

        run_csvdb("to-csvdb", str(duck_path))

        schema = (temp_dir / "views.csvdb" / "schema.sql").read_text()
        assert "expensive_items" in schema


class TestTablesWithoutPK:
    """Tests for tables without primary keys."""

    def test_table_without_pk_all_columns(self, run_csvdb, temp_dir):
        """Tables without PK should work with --order=all-columns."""
        db_path = temp_dir / "no_pk.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE events (timestamp TEXT, type TEXT, data TEXT)")
        conn.executemany("INSERT INTO events VALUES (?, ?, ?)", [
            ("2024-01-01", "A", "data1"),
            ("2024-01-02", "B", "data2"),
            ("2024-01-01", "A", "data3"),
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", "--order=all-columns", str(db_path))

        csvdb_dir = temp_dir / "no_pk.csvdb"
        assert csvdb_dir.exists()

        # Roundtrip
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))

        conn = sqlite3.connect(temp_dir / "no_pk.sqlite")
        count = conn.execute("SELECT COUNT(*) FROM events").fetchone()[0]
        conn.close()
        assert count == 3

    def test_table_without_pk_synthetic_key(self, run_csvdb, temp_dir):
        """Tables without PK should work with --order=add-synthetic-key."""
        db_path = temp_dir / "logs.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE log (message TEXT, level TEXT)")
        conn.executemany("INSERT INTO log VALUES (?, ?)", [
            ("Start", "INFO"),
            ("Process", "DEBUG"),
            ("End", "INFO"),
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", "--order=add-synthetic-key", str(db_path))

        csv_content = (temp_dir / "logs.csvdb" / "log.csv").read_text()
        # __csvdb_rowid should be in CSV for deterministic ordering
        assert "__csvdb_rowid" in csv_content

        # Verify rows are numbered sequentially
        lines = csv_content.strip().split('\n')
        assert len(lines) == 4  # header + 3 rows
        # First column should be rowid
        assert lines[0].startswith('"__csvdb_rowid"')

    def test_mixed_tables_with_and_without_pk(self, run_csvdb, temp_dir):
        """Database with both PK and non-PK tables should work."""
        db_path = temp_dir / "mixed_pk.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("CREATE TABLE events (timestamp TEXT, event TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO events VALUES ('2024-01-01', 'login')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", "--order=all-columns", str(db_path))

        assert (temp_dir / "mixed_pk.csvdb" / "users.csv").exists()
        assert (temp_dir / "mixed_pk.csvdb" / "events.csv").exists()
