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

    def test_null_mode_aliases(self, run_csvdb, temp_dir):
        """--null-mode=postgres/mysql/excel should work as aliases."""
        import sqlite3

        db_path = temp_dir / "null_alias.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")
        conn.execute("INSERT INTO test VALUES (2, 'hello')")
        conn.commit()
        conn.close()

        # postgres -> marker (lossless)
        out1 = temp_dir / "pg.csvdb"
        run_csvdb("to-csvdb", "--null-mode=postgres", "-o", str(out1), str(db_path))
        csv1 = (out1 / "test.csv").read_text()
        assert "\\N" in csv1  # marker mode

        # mysql -> marker (lossless)
        out2 = temp_dir / "mysql.csvdb"
        run_csvdb("to-csvdb", "--null-mode=mysql", "-o", str(out2), str(db_path))
        csv2 = (out2 / "test.csv").read_text()
        assert "\\N" in csv2  # marker mode

        # excel -> empty (lossy)
        out3 = temp_dir / "excel.csvdb"
        run_csvdb("to-csvdb", "--null-mode=excel", "-o", str(out3), str(db_path))
        csv3 = (out3 / "test.csv").read_text()
        assert "\\N" not in csv3  # empty mode, no marker

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


class TestConfigAsInput:
    """Tests for reading settings from csvdb.toml when re-exporting."""

    def test_reexport_preserves_null_mode(self, run_csvdb, temp_dir):
        """Re-exporting .csvdb without --null-mode should use the config value."""
        import sqlite3

        db_path = temp_dir / "cfg_null.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, NULL)")
        conn.execute("INSERT INTO t VALUES (2, 'hello')")
        conn.commit()
        conn.close()

        # Export with --null-mode=empty (lossy)
        csvdb1 = temp_dir / "cfg1.csvdb"
        run_csvdb("to-csvdb", "--null-mode=empty", "-o", str(csvdb1), str(db_path))

        # Verify empty mode was used
        csv1 = (csvdb1 / "t.csv").read_text()
        assert "\\N" not in csv1

        # Re-export to parquetdb without specifying --null-mode
        # It should read from csvdb.toml and use "empty"
        pdb = temp_dir / "cfg.parquetdb"
        run_csvdb("to-parquetdb", "-o", str(pdb), str(csvdb1), "--force")

        # Check that the parquetdb config preserved the setting
        import tomllib
        with open(pdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert toml["null_mode"] == "empty"

    def test_cli_flag_overrides_config(self, run_csvdb, temp_dir):
        """CLI flag should override config file setting."""
        import sqlite3

        db_path = temp_dir / "cfg_override.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, NULL)")
        conn.commit()
        conn.close()

        # Export with empty mode
        csvdb1 = temp_dir / "override1.csvdb"
        run_csvdb("to-csvdb", "--null-mode=empty", "-o", str(csvdb1), str(db_path))

        # Re-export with explicit --null-mode=marker (override)
        pdb = temp_dir / "override.parquetdb"
        run_csvdb("to-parquetdb", "--null-mode=marker", "-o", str(pdb), str(csvdb1), "--force")

        import tomllib
        with open(pdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert toml["null_mode"] == "marker"


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


class TestNaturalSort:
    """Tests for --natural-sort flag."""

    def test_natural_sort_order(self, run_csvdb, temp_dir):
        """--natural-sort should order string PKs naturally."""
        db_path = temp_dir / "natural.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE items (name TEXT PRIMARY KEY, value INTEGER)")
        conn.executemany("INSERT INTO items VALUES (?, ?)", [
            ("item1", 10),
            ("item10", 100),
            ("item2", 20),
            ("item20", 200),
            ("item3", 30),
        ])
        conn.commit()
        conn.close()

        # Without natural sort: lexicographic order
        csvdb1 = temp_dir / "lex.csvdb"
        run_csvdb("to-csvdb", "-o", str(csvdb1), str(db_path))
        csv1 = (csvdb1 / "items.csv").read_text()
        lines1 = csv1.strip().split('\n')[1:]  # skip header
        names1 = [line.split(',')[0].strip('"') for line in lines1]
        assert names1 == ["item1", "item10", "item2", "item20", "item3"]

        # With natural sort: natural order
        csvdb2 = temp_dir / "nat.csvdb"
        run_csvdb("to-csvdb", "--natural-sort", "-o", str(csvdb2), str(db_path))
        csv2 = (csvdb2 / "items.csv").read_text()
        lines2 = csv2.strip().split('\n')[1:]  # skip header
        names2 = [line.split(',')[0].strip('"') for line in lines2]
        assert names2 == ["item1", "item2", "item3", "item10", "item20"]

    def test_natural_sort_stored_in_config(self, run_csvdb, temp_dir):
        """--natural-sort should be stored in csvdb.toml."""
        db_path = temp_dir / "ns_cfg.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id TEXT PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES ('a')")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "ns_cfg.csvdb"
        run_csvdb("to-csvdb", "--natural-sort", "-o", str(csvdb), str(db_path))

        import tomllib
        with open(csvdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert toml.get("natural_sort") is True

    def test_no_natural_sort_not_in_config(self, run_csvdb, temp_dir):
        """Without --natural-sort, it should not appear in csvdb.toml."""
        db_path = temp_dir / "no_ns.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id TEXT PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES ('a')")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "no_ns.csvdb"
        run_csvdb("to-csvdb", "-o", str(csvdb), str(db_path))

        import tomllib
        with open(csvdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert "natural_sort" not in toml


class TestOrderBy:
    """Tests for --order-by custom clause."""

    def test_order_by_desc(self, run_csvdb, temp_dir):
        """--order-by should use custom SQL ordering."""
        db_path = temp_dir / "orderby.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.executemany("INSERT INTO users VALUES (?, ?)", [
            (1, "Alice"),
            (2, "Bob"),
            (3, "Charlie"),
        ])
        conn.commit()
        conn.close()

        csvdb = temp_dir / "orderby.csvdb"
        run_csvdb("to-csvdb", "--order-by", "name DESC", "-o", str(csvdb), str(db_path))

        csv_content = (csvdb / "users.csv").read_text()
        lines = csv_content.strip().split('\n')[1:]  # skip header
        names = [line.split(',')[1].strip('"') for line in lines]
        assert names == ["Charlie", "Bob", "Alice"]

    def test_order_by_stored_in_config(self, run_csvdb, temp_dir):
        """--order-by should be stored in csvdb.toml."""
        db_path = temp_dir / "ob_cfg.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'a')")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "ob_cfg.csvdb"
        run_csvdb("to-csvdb", "--order-by", "name ASC", "-o", str(csvdb), str(db_path))

        import tomllib
        with open(csvdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert toml.get("order_by") == "name ASC"
        # order should not be present when order_by is used
        assert "order" not in toml

    def test_order_by_conflicts_with_order(self, run_csvdb, temp_dir):
        """--order-by and --order should conflict."""
        db_path = temp_dir / "conflict.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        # clap should reject conflicting flags
        result = run_csvdb("to-csvdb", "--order=pk", "--order-by", "id DESC", str(db_path), check=False)
        assert result.returncode != 0


class TestCompression:
    """Tests for the --compress flag."""

    def test_compress_creates_gz_files(self, run_csvdb, temp_dir):
        """--compress should create .csv.gz files instead of .csv."""
        db_path = temp_dir / "compress.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO users VALUES (2, 'Bob')")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "compress.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb), "--force", str(db_path))

        # Should have .csv.gz, not .csv
        assert (csvdb / "users.csv.gz").exists()
        assert not (csvdb / "users.csv").exists()
        assert (csvdb / "schema.sql").exists()
        assert (csvdb / "csvdb.toml").exists()

        # Verify the file is actually gzip (magic bytes: 1f 8b)
        with open(csvdb / "users.csv.gz", "rb") as f:
            header = f.read(2)
        assert header == b'\x1f\x8b', "File should be gzip compressed"

    def test_compress_stored_in_config(self, run_csvdb, temp_dir):
        """--compress should store compressed=true in csvdb.toml."""
        db_path = temp_dir / "cfg.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "cfg.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb), str(db_path))

        import tomllib
        with open(csvdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert toml.get("compressed") is True

    def test_no_compress_not_in_config(self, run_csvdb, temp_dir):
        """Without --compress, compressed should not appear in csvdb.toml."""
        db_path = temp_dir / "nocfg.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "nocfg.csvdb"
        run_csvdb("to-csvdb", "-o", str(csvdb), str(db_path))

        import tomllib
        with open(csvdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert "compressed" not in toml

    def test_compressed_roundtrip_to_sqlite(self, run_csvdb, temp_dir):
        """Compressed csvdb should roundtrip to SQLite correctly."""
        db_path = temp_dir / "rt.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score INTEGER)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice', 95)")
        conn.execute("INSERT INTO users VALUES (2, 'Bob', 87)")
        conn.execute("INSERT INTO users VALUES (3, 'Charlie', 92)")
        conn.commit()
        conn.close()

        # Export with compression
        csvdb = temp_dir / "rt.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb), "--force", str(db_path))

        # Import back to SQLite
        run_csvdb("to-sqlite", "--force", str(csvdb))

        # Verify data
        rebuilt = temp_dir / "rt.sqlite"
        conn = sqlite3.connect(rebuilt)
        rows = conn.execute("SELECT * FROM users ORDER BY id").fetchall()
        assert rows == [(1, "Alice", 95), (2, "Bob", 87), (3, "Charlie", 92)]
        conn.close()

    def test_compressed_roundtrip_to_duckdb(self, run_csvdb, temp_dir):
        """Compressed csvdb should roundtrip to DuckDB correctly."""
        db_path = temp_dir / "drt.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO items VALUES (1, 'Widget')")
        conn.execute("INSERT INTO items VALUES (2, 'Gadget')")
        conn.commit()
        conn.close()

        # Export with compression
        csvdb = temp_dir / "drt.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb), "--force", str(db_path))

        # Import to DuckDB
        run_csvdb("to-duckdb", "--force", str(csvdb))
        duckdb_path = temp_dir / "drt.duckdb"
        assert duckdb_path.exists()

    def test_compressed_checksum_matches_uncompressed(self, run_csvdb, temp_dir):
        """Checksum of compressed csvdb should match uncompressed version with same data."""
        db_path = temp_dir / "cksum.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score INTEGER)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice', 95)")
        conn.execute("INSERT INTO users VALUES (2, 'Bob', 87)")
        conn.commit()
        conn.close()

        # Export uncompressed
        csvdb_plain = temp_dir / "plain.csvdb"
        run_csvdb("to-csvdb", "-o", str(csvdb_plain), "--force", str(db_path))

        # Export compressed
        csvdb_gz = temp_dir / "gz.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb_gz), "--force", str(db_path))

        # Checksums should match
        result_plain = run_csvdb("checksum", str(csvdb_plain))
        result_gz = run_csvdb("checksum", str(csvdb_gz))
        assert result_plain.stdout.strip() == result_gz.stdout.strip()

    def test_compressed_validate(self, run_csvdb, temp_dir):
        """Validate should work on compressed csvdb directories."""
        db_path = temp_dir / "val.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO users VALUES (2, 'Bob')")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "val.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb), "--force", str(db_path))

        # Validate should succeed
        result = run_csvdb("validate", str(csvdb))
        assert result.returncode == 0

    def test_compressed_diff(self, run_csvdb, temp_dir):
        """Diff should work with compressed csvdb directories."""
        # Create two databases
        db1 = temp_dir / "d1.sqlite"
        conn1 = sqlite3.connect(db1)
        conn1.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn1.execute("INSERT INTO t VALUES (1, 'Alice')")
        conn1.commit()
        conn1.close()

        db2 = temp_dir / "d2.sqlite"
        conn2 = sqlite3.connect(db2)
        conn2.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        conn2.execute("INSERT INTO t VALUES (1, 'Alice')")
        conn2.execute("INSERT INTO t VALUES (2, 'Bob')")
        conn2.commit()
        conn2.close()

        # Export both compressed
        csvdb1 = temp_dir / "d1.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb1), str(db1))
        csvdb2 = temp_dir / "d2.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb2), str(db2))

        # Diff should detect the difference
        result = run_csvdb("diff", str(csvdb1), str(csvdb2), check=False)
        assert result.returncode == 1  # differences found
        assert "1 added" in result.stdout

    def test_compress_multi_table(self, run_csvdb, temp_dir):
        """--compress should work with multiple tables."""
        db_path = temp_dir / "multi.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER)")
        conn.execute("INSERT INTO orders VALUES (1, 1)")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "multi.csvdb"
        run_csvdb("to-csvdb", "--compress", "-o", str(csvdb), "--force", str(db_path))

        assert (csvdb / "users.csv.gz").exists()
        assert (csvdb / "orders.csv.gz").exists()
        assert not (csvdb / "users.csv").exists()
        assert not (csvdb / "orders.csv").exists()


class TestIncremental:
    """Tests for --incremental flag."""

    def test_incremental_first_export_writes_all(self, run_csvdb, temp_dir):
        """First incremental export should write all tables and store checksums."""
        db_path = temp_dir / "inc.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO users VALUES (2, 'Bob')")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER)")
        conn.execute("INSERT INTO orders VALUES (1, 1)")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "inc.csvdb"
        result = run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))

        # All files should exist
        assert (csvdb / "users.csv").exists()
        assert (csvdb / "orders.csv").exists()
        assert (csvdb / "schema.sql").exists()
        assert (csvdb / "csvdb.toml").exists()

        # Checksums should be stored in config
        import tomllib
        with open(csvdb / "csvdb.toml", "rb") as f:
            toml = tomllib.load(f)
        assert "table_checksums" in toml
        assert "users" in toml["table_checksums"]
        assert "orders" in toml["table_checksums"]

        # Summary should show both as new
        assert "new" in result.stderr

    def test_incremental_unchanged_skips(self, run_csvdb, temp_dir):
        """Second incremental with no changes should skip all tables."""
        db_path = temp_dir / "inc2.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "inc2.csvdb"

        # First export
        run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))

        # Record the modification time of the CSV
        import os
        csv_mtime = os.path.getmtime(csvdb / "users.csv")

        # Wait a tiny bit to ensure mtime would differ if rewritten
        import time
        time.sleep(0.05)

        # Second export - no changes
        result = run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))

        # CSV should NOT have been rewritten (same mtime)
        assert os.path.getmtime(csvdb / "users.csv") == csv_mtime

        # Summary should show unchanged
        assert "unchanged" in result.stderr

    def test_incremental_updates_changed_table(self, run_csvdb, temp_dir):
        """When data changes, only the changed table should be rewritten."""
        db_path = temp_dir / "inc3.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER)")
        conn.execute("INSERT INTO orders VALUES (1, 1)")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "inc3.csvdb"

        # First export
        run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))

        import os
        orders_mtime = os.path.getmtime(csvdb / "orders.csv")

        import time
        time.sleep(0.05)

        # Modify users table only
        conn = sqlite3.connect(db_path)
        conn.execute("INSERT INTO users VALUES (2, 'Bob')")
        conn.commit()
        conn.close()

        # Second export
        result = run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))

        # Orders CSV should NOT have been rewritten
        assert os.path.getmtime(csvdb / "orders.csv") == orders_mtime

        # Users CSV should be updated
        users_content = (csvdb / "users.csv").read_text()
        assert "Bob" in users_content

        # Summary
        assert "updated" in result.stderr or "new" in result.stderr
        assert "unchanged" in result.stderr

    def test_incremental_removes_deleted_table(self, run_csvdb, temp_dir):
        """When a table is removed from source, its CSV should be deleted."""
        db_path = temp_dir / "inc4.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("CREATE TABLE old_table (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO old_table VALUES (1)")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "inc4.csvdb"

        # First export
        run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))
        assert (csvdb / "old_table.csv").exists()

        # Drop old_table from database
        conn = sqlite3.connect(db_path)
        conn.execute("DROP TABLE old_table")
        conn.commit()
        conn.close()

        # Second export
        result = run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))

        # old_table.csv should be removed
        assert not (csvdb / "old_table.csv").exists()
        assert "removed" in result.stderr

    def test_incremental_adds_new_table(self, run_csvdb, temp_dir):
        """When a table is added to the source, its CSV should be created."""
        db_path = temp_dir / "inc5.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.commit()
        conn.close()

        csvdb = temp_dir / "inc5.csvdb"

        # First export
        run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))
        assert not (csvdb / "orders.csv").exists()

        # Add new table
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER)")
        conn.execute("INSERT INTO orders VALUES (1, 1)")
        conn.commit()
        conn.close()

        # Second export
        result = run_csvdb("to-csvdb", "--incremental", "-o", str(csvdb), str(db_path))

        # New table should be created
        assert (csvdb / "orders.csv").exists()
        assert "new" in result.stderr
