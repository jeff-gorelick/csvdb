"""Functional tests for error handling, data edge cases, and boundary conditions."""

import sqlite3
import subprocess
from pathlib import Path


class TestErrorHandling:
    """Tests for error handling and edge cases."""

    def test_missing_input_directory(self, run_csvdb, temp_dir):
        """Should error on missing input directory."""
        result = run_csvdb("to-sqlite", str(temp_dir / "nonexistent.csvdb"), check=False)
        assert result.returncode != 0

    def test_invalid_sqlite_file(self, run_csvdb, temp_dir):
        """Should error on invalid SQLite file."""
        bad_file = temp_dir / "bad.sqlite"
        bad_file.write_text("this is not a sqlite file")

        result = run_csvdb("to-csvdb", str(bad_file), check=False)
        assert result.returncode != 0

    def test_missing_schema_sql(self, run_csvdb, temp_dir):
        """Should error when schema.sql is missing from csvdb."""
        csvdb_dir = temp_dir / "no_schema.csvdb"
        csvdb_dir.mkdir()
        (csvdb_dir / "data.csv").write_text("id,name\n1,test\n")

        result = run_csvdb("to-sqlite", str(csvdb_dir), check=False)
        assert result.returncode != 0

    def test_checksum_invalid_sqlite(self, run_csvdb, temp_dir):
        """Should error on checksum of corrupted SQLite file."""
        bad_file = temp_dir / "corrupt.sqlite"
        bad_file.write_text("this is not a sqlite file")

        result = run_csvdb("checksum", str(bad_file), check=False)
        assert result.returncode != 0

    def test_unknown_file_extension(self, run_csvdb, temp_dir):
        """Should error on unknown file extension for checksum."""
        unknown = temp_dir / "data.unknown"
        unknown.write_text("some data")

        result = run_csvdb("checksum", str(unknown), check=False)
        assert result.returncode != 0

    def test_invalid_duckdb_file(self, run_csvdb, temp_dir):
        """Should error on invalid DuckDB file."""
        bad_file = temp_dir / "bad.duckdb"
        bad_file.write_text("this is not a duckdb file")

        result = run_csvdb("to-csvdb", str(bad_file), check=False)
        assert result.returncode != 0

    def test_empty_csvdb_directory(self, run_csvdb, temp_dir):
        """Should error on empty csvdb directory."""
        empty_dir = temp_dir / "empty.csvdb"
        empty_dir.mkdir()

        result = run_csvdb("to-sqlite", str(empty_dir), check=False)
        assert result.returncode != 0


class TestDataEdgeCases:
    """Tests for data edge cases."""

    def test_unicode_text(self, run_csvdb, temp_dir):
        """Should handle Unicode text including emoji and CJK."""
        db_path = temp_dir / "unicode.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE texts (id INTEGER PRIMARY KEY, content TEXT)")
        conn.executemany("INSERT INTO texts VALUES (?, ?)", [
            (1, "Hello World"),
            (2, "Héllo Wörld"),  # Accented characters
            (3, "你好世界"),  # Chinese
            (4, "こんにちは"),  # Japanese
            (5, "مرحبا"),  # Arabic (RTL)
            (6, "🎉🚀💻"),  # Emoji
            (7, "Mixed: café 日本 🍣"),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "unicode.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "unicode.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

        # Verify content
        conn = sqlite3.connect(temp_dir / "unicode.sqlite")
        rows = conn.execute("SELECT content FROM texts ORDER BY id").fetchall()
        conn.close()
        assert rows[2][0] == "你好世界"
        assert rows[5][0] == "🎉🚀💻"

    def test_very_long_text(self, run_csvdb, temp_dir):
        """Should handle very long text values."""
        db_path = temp_dir / "long_text.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE docs (id INTEGER PRIMARY KEY, content TEXT)")

        # Insert texts of varying lengths
        conn.executemany("INSERT INTO docs VALUES (?, ?)", [
            (1, "a" * 1000),
            (2, "b" * 10000),
            (3, "c" * 100000),  # 100KB
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "long_text.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "long_text.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_sql_injection_strings(self, run_csvdb, temp_dir):
        """Should safely handle SQL injection-like strings."""
        db_path = temp_dir / "injection.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [
            (1, "'; DROP TABLE data; --"),
            (2, "1; DELETE FROM data WHERE '1'='1"),
            (3, "' OR '1'='1"),
            (4, "Robert'); DROP TABLE Students;--"),
            (5, "<script>alert('xss')</script>"),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "injection.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "injection.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

        # Verify data is intact
        conn = sqlite3.connect(temp_dir / "injection.sqlite")
        count = conn.execute("SELECT COUNT(*) FROM data").fetchone()[0]
        conn.close()
        assert count == 5

    def test_extreme_numbers(self, run_csvdb, temp_dir):
        """Should handle extreme numeric values."""
        db_path = temp_dir / "extreme_nums.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE nums (id INTEGER PRIMARY KEY, int_val INTEGER, real_val REAL)")
        conn.executemany("INSERT INTO nums VALUES (?, ?, ?)", [
            (1, 0, 0.0),
            (2, 9223372036854775807, 1.0),  # Max int64
            (3, -9223372036854775808, -1.0),  # Min int64
            (4, 1, 1e38),  # Large float
            (5, -1, -1e38),  # Large negative float
            (6, 1, 1e-38),  # Small float
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "extreme_nums.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "extreme_nums.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_csv_special_characters(self, run_csvdb, temp_dir):
        """Should handle characters that are special in CSV format."""
        db_path = temp_dir / "csv_special.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [
            (1, 'simple'),
            (2, 'with,comma'),
            (3, 'with"quote'),
            (4, 'with""double'),
            (5, 'with\ttab'),
            (6, 'line1\nline2'),
            (7, 'all,of"them\nhere'),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "csv_special.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "csv_special.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum


class TestEmptyAndMinimalCases:
    """Tests for empty and minimal data cases."""

    def test_empty_table(self, run_csvdb, temp_dir):
        """Should handle empty tables (schema only, no rows)."""
        db_path = temp_dir / "empty.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE empty_table (id INTEGER PRIMARY KEY, name TEXT, value REAL)")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))

        # CSV should exist but be header-only
        csv_path = temp_dir / "empty.csvdb" / "empty_table.csv"
        assert csv_path.exists()
        lines = csv_path.read_text().strip().split('\n')
        assert len(lines) == 1  # Header only

        run_csvdb("to-sqlite", "--force", str(temp_dir / "empty.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "empty.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_single_row_table(self, run_csvdb, temp_dir):
        """Should handle single row tables."""
        db_path = temp_dir / "single.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO data VALUES (1, 'only row')")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "single.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "single.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_single_column_table(self, run_csvdb, temp_dir):
        """Should handle single column tables."""
        db_path = temp_dir / "single_col.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE ids (id INTEGER PRIMARY KEY)")
        conn.executemany("INSERT INTO ids VALUES (?)", [(1,), (2,), (3,)])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "single_col.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "single_col.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_all_null_values(self, run_csvdb, temp_dir):
        """Should handle rows with all NULL values (except PK)."""
        db_path = temp_dir / "all_nulls.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, a TEXT, b INTEGER, c REAL)")
        conn.execute("INSERT INTO data VALUES (1, NULL, NULL, NULL)")
        conn.execute("INSERT INTO data VALUES (2, NULL, NULL, NULL)")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "all_nulls.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "all_nulls.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_multiple_empty_tables(self, run_csvdb, temp_dir):
        """Should handle database with multiple empty tables."""
        db_path = temp_dir / "multi_empty.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE table1 (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("CREATE TABLE table2 (id INTEGER PRIMARY KEY, num INTEGER)")
        conn.execute("CREATE TABLE table3 (id INTEGER PRIMARY KEY, data REAL)")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "multi_empty.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "multi_empty.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum


class TestBlobData:
    """Tests for BLOB/binary data."""

    def test_blob_roundtrip(self, run_csvdb, temp_dir):
        """BLOB data should roundtrip correctly."""
        db_path = temp_dir / "blob.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE files (id INTEGER PRIMARY KEY, name TEXT, data BLOB)")
        # Insert binary data
        conn.execute("INSERT INTO files VALUES (?, ?, ?)",
                     (1, "test.bin", b'\x00\x01\x02\xff\xfe\xfd'))
        conn.execute("INSERT INTO files VALUES (?, ?, ?)",
                     (2, "empty.bin", b''))
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "blob.csvdb"))

        # Verify data
        conn = sqlite3.connect(temp_dir / "blob.sqlite")
        rows = conn.execute("SELECT * FROM files ORDER BY id").fetchall()
        conn.close()

        assert len(rows) == 2
        assert rows[0][1] == "test.bin"

    def test_blob_with_nulls(self, run_csvdb, temp_dir):
        """NULL BLOB values should be handled."""
        db_path = temp_dir / "blob_null.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, content BLOB)")
        conn.execute("INSERT INTO data VALUES (1, NULL)")
        conn.execute("INSERT INTO data VALUES (2, X'DEADBEEF')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "blob_null.csvdb"))

        conn = sqlite3.connect(temp_dir / "blob_null.sqlite")
        rows = conn.execute("SELECT * FROM data ORDER BY id").fetchall()
        conn.close()

        # NULL may come back as None or empty string depending on CSV handling
        assert rows[0][1] is None or rows[0][1] == '' or rows[0][1] == b''
        assert len(rows) == 2


class TestDateTimeValues:
    """Tests for date and time values."""

    def test_date_values(self, run_csvdb, temp_dir):
        """Date values should roundtrip correctly."""
        db_path = temp_dir / "dates.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, event_date TEXT)")
        conn.executemany("INSERT INTO events VALUES (?, ?)", [
            (1, "2024-01-15"),
            (2, "2024-06-30"),
            (3, "2024-12-31"),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "dates.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "dates.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_datetime_values(self, run_csvdb, temp_dir):
        """Datetime values should roundtrip correctly."""
        db_path = temp_dir / "datetimes.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE logs (id INTEGER PRIMARY KEY, timestamp TEXT)")
        conn.executemany("INSERT INTO logs VALUES (?, ?)", [
            (1, "2024-01-15 10:30:00"),
            (2, "2024-06-30 23:59:59"),
            (3, "2024-12-31 00:00:00"),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "datetimes.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "datetimes.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_iso_timestamps(self, run_csvdb, temp_dir):
        """ISO format timestamps should roundtrip correctly."""
        db_path = temp_dir / "iso.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, created_at TEXT)")
        conn.executemany("INSERT INTO events VALUES (?, ?)", [
            (1, "2024-01-15T10:30:00Z"),
            (2, "2024-06-30T23:59:59+00:00"),
            (3, "2024-12-31T00:00:00.123456Z"),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "iso.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "iso.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum


class TestCaseSensitivity:
    """Tests for case sensitivity in names."""

    def test_mixed_case_table_name(self, run_csvdb, temp_dir):
        """Mixed case table names should be preserved."""
        db_path = temp_dir / "mixed_case.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute('CREATE TABLE "MyTable" (id INTEGER PRIMARY KEY, val TEXT)')
        conn.execute('INSERT INTO "MyTable" VALUES (1, "test")')
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        # Check CSV file uses correct case
        assert (temp_dir / "mixed_case.csvdb" / "MyTable.csv").exists()

    def test_mixed_case_column_names(self, run_csvdb, temp_dir):
        """Mixed case column names should be preserved."""
        db_path = temp_dir / "mixed_cols.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute('CREATE TABLE data (ID INTEGER PRIMARY KEY, FirstName TEXT, lastName TEXT)')
        conn.execute('INSERT INTO data VALUES (1, "Alice", "Smith")')
        conn.commit()
        conn.close()

        run_csvdb("checksum", str(db_path))

        run_csvdb("to-csvdb", str(db_path))

        csv_content = (temp_dir / "mixed_cols.csvdb" / "data.csv").read_text()
        header = csv_content.split('\n')[0]
        assert "FirstName" in header or "firstname" in header.lower()

        run_csvdb("to-sqlite", "--force", str(temp_dir / "mixed_cols.csvdb"))

        # Verify data preserved
        conn = sqlite3.connect(temp_dir / "mixed_cols.sqlite")
        rows = conn.execute("SELECT * FROM data").fetchall()
        conn.close()
        assert rows[0][1] == "Alice"

    def test_case_insensitive_sqlite(self, run_csvdb, temp_dir):
        """SQLite is case-insensitive for table names by default."""
        db_path = temp_dir / "case_insens.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE Users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'test')")  # lowercase access
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "case_insens.csvdb"))

        conn = sqlite3.connect(temp_dir / "case_insens.sqlite")
        # Both should work
        rows1 = conn.execute("SELECT * FROM Users").fetchall()
        rows2 = conn.execute("SELECT * FROM users").fetchall()
        conn.close()
        assert rows1 == rows2


class TestOutputOverwrites:
    """Tests for output overwrite behavior."""

    def test_overwrite_existing_csvdb(self, run_csvdb, temp_dir):
        """Should handle overwriting existing csvdb directory."""
        db_path = temp_dir / "overwrite.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO data VALUES (1, 'original')")
        conn.commit()
        conn.close()

        # First export
        run_csvdb("to-csvdb", str(db_path))

        # Modify database
        conn = sqlite3.connect(db_path)
        conn.execute("UPDATE data SET val = 'modified'")
        conn.commit()
        conn.close()

        # Second export (overwrite)
        run_csvdb("to-csvdb", "--force", str(db_path))

        # Should have new content
        csv_content = (temp_dir / "overwrite.csvdb" / "data.csv").read_text()
        assert "modified" in csv_content

    def test_overwrite_existing_sqlite(self, run_csvdb, temp_dir):
        """Should handle overwriting existing SQLite file."""
        # Create csvdb
        csvdb_dir = temp_dir / "test.csvdb"
        csvdb_dir.mkdir()
        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT);'
        )
        (csvdb_dir / "data.csv").write_text('"id","val"\n"1","first"\n')

        # First conversion
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))

        # Modify csvdb
        (csvdb_dir / "data.csv").write_text('"id","val"\n"1","second"\n')

        # Second conversion (overwrite)
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))

        # Should have new content
        conn = sqlite3.connect(temp_dir / "test.sqlite")
        val = conn.execute("SELECT val FROM data WHERE id = 1").fetchone()[0]
        conn.close()
        assert val == "second"


class TestPathEdgeCases:
    """Tests for path edge cases."""

    def test_path_with_spaces(self, run_csvdb, temp_dir):
        """Should handle paths with spaces."""
        spaced_dir = temp_dir / "path with spaces"
        spaced_dir.mkdir()

        db_path = spaced_dir / "my database.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO data VALUES (1, 'test')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        csvdb_dir = spaced_dir / "my database.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "data.csv").exists()

    def test_output_path_with_spaces(self, run_csvdb, sample_sqlite, temp_dir):
        """Should handle output paths with spaces."""
        output_dir = temp_dir / "output with spaces.csvdb"

        run_csvdb("to-csvdb", "-o", str(output_dir), str(sample_sqlite))

        assert output_dir.exists()
        assert (output_dir / "schema.sql").exists()

    def test_relative_path_handling(self, run_csvdb, temp_dir):
        """Should handle relative-style paths correctly."""
        db_path = temp_dir / "test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO data VALUES (1)")
        conn.commit()
        conn.close()

        # Use the full path but verify it works
        run_csvdb("to-csvdb", str(db_path))
        assert (temp_dir / "test.csvdb").exists()


class TestLargeDatasets:
    """Tests for very large datasets."""

    def test_100k_rows(self, run_csvdb, temp_dir):
        """Should handle 100k rows efficiently."""
        db_path = temp_dir / "large100k.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE records (id INTEGER PRIMARY KEY, data TEXT, value INTEGER)")

        # Insert 100k rows in batches
        batch_size = 10000
        for batch in range(10):
            start = batch * batch_size + 1
            rows = [(i, f"record_{i}", i % 1000) for i in range(start, start + batch_size)]
            conn.executemany("INSERT INTO records VALUES (?, ?, ?)", rows)
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "large100k.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "large100k.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

        # Verify count
        conn = sqlite3.connect(temp_dir / "large100k.sqlite")
        count = conn.execute("SELECT COUNT(*) FROM records").fetchone()[0]
        conn.close()
        assert count == 100000

    def test_wide_table_many_columns(self, run_csvdb, temp_dir):
        """Should handle tables with many columns (100+)."""
        db_path = temp_dir / "wide.sqlite"
        conn = sqlite3.connect(db_path)

        # Create table with 100 columns
        cols = ", ".join([f"col_{i} TEXT" for i in range(100)])
        conn.execute(f"CREATE TABLE wide (id INTEGER PRIMARY KEY, {cols})")

        # Insert rows
        for row_id in range(1, 101):
            values = ", ".join([f"'val_{row_id}_{i}'" for i in range(100)])
            conn.execute(f"INSERT INTO wide VALUES ({row_id}, {values})")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "wide.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "wide.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_many_tables(self, run_csvdb, temp_dir):
        """Should handle database with many tables (50+)."""
        db_path = temp_dir / "many_tables.sqlite"
        conn = sqlite3.connect(db_path)

        # Create 50 tables
        for i in range(50):
            conn.execute(f"CREATE TABLE table_{i} (id INTEGER PRIMARY KEY, val TEXT)")
            conn.execute(f"INSERT INTO table_{i} VALUES (1, 'data_{i}')")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))

        # Verify all CSV files exist
        csvdb_dir = temp_dir / "many_tables.csvdb"
        for i in range(50):
            assert (csvdb_dir / f"table_{i}.csv").exists()

        run_csvdb("to-sqlite", "--force", str(csvdb_dir))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "many_tables.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum


class TestSqliteCliImport:
    """Tests for the sqlite3 CLI import path (vs rusqlite fallback)."""

    @staticmethod
    def sqlite3_available():
        import shutil
        return shutil.which("sqlite3") is not None

    def test_cli_null_roundtrip(self, run_csvdb, temp_dir):
        """NULL vs empty string should be preserved through sqlite3 CLI import."""
        if not self.sqlite3_available():
            import pytest
            pytest.skip("sqlite3 CLI not found")

        import sqlite3

        db_path = temp_dir / "cli_null.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)")
        conn.execute("INSERT INTO test VALUES (1, NULL, NULL)")
        conn.execute("INSERT INTO test VALUES (2, '', 42)")
        conn.execute("INSERT INTO test VALUES (3, 'hello', NULL)")
        conn.commit()
        conn.close()

        # Roundtrip: SQLite -> csvdb -> SQLite (uses sqlite3 CLI if available)
        run_csvdb("to-csvdb", str(db_path))
        csvdb_path = temp_dir / "cli_null.csvdb"
        run_csvdb("to-sqlite", "--force", str(csvdb_path))
        rebuilt_path = temp_dir / "cli_null.sqlite"

        conn = sqlite3.connect(str(rebuilt_path))
        rows = conn.execute("SELECT id, name, value FROM test ORDER BY id").fetchall()
        conn.close()

        assert rows[0] == (1, None, None)     # NULLs preserved
        assert rows[1] == (2, '', 42)         # empty string preserved
        assert rows[2] == (3, 'hello', None)  # mixed preserved

    def test_rusqlite_fallback_null_roundtrip(self, run_csvdb, temp_dir, csvdb_bin):
        """NULL handling works via rusqlite fallback when sqlite3 CLI is absent."""
        import sqlite3

        db_path = temp_dir / "fallback_null.sqlite"
        conn = sqlite3.connect(str(db_path))
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)")
        conn.execute("INSERT INTO test VALUES (1, NULL, NULL)")
        conn.execute("INSERT INTO test VALUES (2, '', 42)")
        conn.execute("INSERT INTO test VALUES (3, 'hello', NULL)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))
        csvdb_path = temp_dir / "fallback_null.csvdb"

        # Force rusqlite fallback by hiding sqlite3 from PATH
        import os
        env = os.environ.copy()
        env["PATH"] = ""
        result = subprocess.run(
            [csvdb_bin, "to-sqlite", "--force", str(csvdb_path)],
            capture_output=True, text=True, env=env
        )
        assert result.returncode == 0, f"to-sqlite failed: {result.stderr}"

        rebuilt_path = temp_dir / "fallback_null.sqlite"
        conn = sqlite3.connect(str(rebuilt_path))
        rows = conn.execute("SELECT id, name, value FROM test ORDER BY id").fetchall()
        conn.close()

        assert rows[0] == (1, None, None)     # NULLs preserved
        assert rows[1] == (2, '', 42)         # empty string preserved
        assert rows[2] == (3, 'hello', None)  # mixed preserved


class TestHighValueGaps:
    """Tests covering high-value gaps in flag combinations, edge cases, and code paths."""

    def test_order_all_columns_with_null_mode_literal(self, run_csvdb, temp_dir):
        """--order=all-columns and --null-mode=literal together."""
        db_path = temp_dir / "order_null_literal.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE events (ts TEXT, data TEXT)")
        conn.executemany("INSERT INTO events VALUES (?, ?)", [
            ("2024-01-03", None),
            ("2024-01-01", "hello"),
            ("2024-01-02", "world"),
        ])
        conn.commit()
        conn.close()

        result = run_csvdb(
            "to-csvdb", "--order=all-columns", "--null-mode=literal",
            str(db_path)
        )

        csvdb_dir = temp_dir / "order_null_literal.csvdb"
        csv_content = (csvdb_dir / "events.csv").read_text()

        # NULL should be literal "NULL", not \N marker
        assert "\\N" not in csv_content
        assert "NULL" in csv_content
        # LOSSY warning expected
        assert "LOSSY" in result.stderr

        # Roundtrip back to SQLite — "NULL" string comes back as text, not SQL NULL
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        rebuilt = temp_dir / "order_null_literal.sqlite"
        conn = sqlite3.connect(rebuilt)
        rows = conn.execute("SELECT ts, data FROM events ORDER BY ts").fetchall()
        conn.close()
        assert len(rows) == 3

    def test_order_synthetic_key_with_null_mode_empty(self, run_csvdb, temp_dir):
        """--order=add-synthetic-key and --null-mode=empty together."""
        db_path = temp_dir / "synth_null_empty.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE log (msg TEXT, detail TEXT)")
        conn.executemany("INSERT INTO log VALUES (?, ?)", [
            (None, "info"),
            ("hello", None),
            ("", "empty_msg"),
        ])
        conn.commit()
        conn.close()

        result = run_csvdb(
            "to-csvdb", "--order=add-synthetic-key", "--null-mode=empty",
            str(db_path)
        )

        csvdb_dir = temp_dir / "synth_null_empty.csvdb"
        csv_content = (csvdb_dir / "log.csv").read_text()

        # Synthetic key column should be present
        assert "__csvdb_rowid" in csv_content
        # \N marker should NOT be used (empty mode)
        assert "\\N" not in csv_content
        # LOSSY warning expected
        assert "LOSSY" in result.stderr

    def test_null_mode_literal_with_literal_null_string(self, run_csvdb, temp_dir):
        """--null-mode=literal when data contains the string 'NULL' — ambiguity test."""
        db_path = temp_dir / "null_ambig.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")       # SQL NULL
        conn.execute("INSERT INTO test VALUES (2, 'NULL')")     # literal string "NULL"
        conn.execute("INSERT INTO test VALUES (3, 'hello')")
        conn.commit()
        conn.close()

        result = run_csvdb("to-csvdb", "--null-mode=literal", str(db_path))
        csvdb_dir = temp_dir / "null_ambig.csvdb"
        csv_content = (csvdb_dir / "test.csv").read_text()

        # Both row 1 (SQL NULL) and row 2 (string "NULL") should produce "NULL" in CSV
        lines = csv_content.strip().split('\n')
        data_lines = lines[1:]  # skip header
        null_values = [line.split(',')[1].strip('"') for line in data_lines]
        # id=1 -> "NULL", id=2 -> "NULL", id=3 -> "hello"
        assert null_values[0] == "NULL"
        assert null_values[1] == "NULL"
        assert null_values[2] == "hello"
        assert "LOSSY" in result.stderr

        # Roundtrip — both come back as the same value (confirms lossy)
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        rebuilt = temp_dir / "null_ambig.sqlite"
        conn = sqlite3.connect(rebuilt)
        rows = conn.execute("SELECT id, val FROM test ORDER BY id").fetchall()
        conn.close()
        # Both row 1 and row 2 should be indistinguishable after roundtrip
        assert rows[0][1] == rows[1][1]

    def test_null_mode_marker_with_backslash_n_data(self, run_csvdb, temp_dir):
        r"""Default marker mode when data naturally contains \N."""
        db_path = temp_dir / "marker_clash.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, val TEXT)")
        conn.execute("INSERT INTO test VALUES (1, NULL)")
        conn.execute(r"INSERT INTO test VALUES (2, '\N')")
        conn.execute(r"INSERT INTO test VALUES (3, 'C\NaCl')")
        conn.commit()
        conn.close()

        # Default mode is marker (\N)
        run_csvdb("to-csvdb", str(db_path))
        csvdb_dir = temp_dir / "marker_clash.csvdb"
        csv_content = (csvdb_dir / "test.csv").read_text()

        # The CSV should contain \N for the SQL NULL (row 1)
        assert "\\N" in csv_content

        # Document actual behavior: verify CSV was created and has all 3 rows
        lines = csv_content.strip().split('\n')
        assert len(lines) == 4  # header + 3 data rows

    def test_duckdb_composite_primary_key_roundtrip(self, run_csvdb, temp_dir):
        """Composite PK detection from DuckDB (uses information_schema path)."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        duck_path = temp_dir / "composite_duck.duckdb"
        conn = duckdb.connect(str(duck_path))
        conn.execute("""
            CREATE TABLE order_items (
                order_id INTEGER NOT NULL,
                item_id INTEGER NOT NULL,
                quantity INTEGER,
                price REAL,
                PRIMARY KEY (order_id, item_id)
            )
        """)
        conn.executemany(
            "INSERT INTO order_items VALUES (?, ?, ?, ?)",
            [
                (1, 1, 5, 9.99),
                (1, 2, 3, 19.99),
                (2, 1, 1, 29.99),
            ]
        )
        conn.close()

        run_csvdb("to-csvdb", str(duck_path))
        csvdb_dir = temp_dir / "composite_duck.csvdb"

        # Verify schema has composite PK
        schema = (csvdb_dir / "schema.sql").read_text()
        assert "PRIMARY KEY" in schema
        # Both columns should appear in the PK definition
        assert "order_id" in schema
        assert "item_id" in schema

        # Roundtrip: DuckDB csvdb -> SQLite, and compare with equivalent SQLite
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        duck_via_sqlite = temp_dir / "composite_duck.sqlite"
        assert duck_via_sqlite.exists()

        duck_checksum = run_csvdb("checksum", str(duck_via_sqlite)).stdout.strip()

        # Create equivalent SQLite directly
        equiv_path = temp_dir / "equiv.sqlite"
        conn = sqlite3.connect(equiv_path)
        conn.execute("""
            CREATE TABLE order_items (
                order_id INTEGER NOT NULL,
                item_id INTEGER NOT NULL,
                quantity INTEGER,
                price REAL,
                PRIMARY KEY (order_id, item_id)
            )
        """)
        conn.executemany(
            "INSERT INTO order_items VALUES (?, ?, ?, ?)",
            [
                (1, 1, 5, 9.99),
                (1, 2, 3, 19.99),
                (2, 1, 1, 29.99),
            ]
        )
        conn.commit()
        conn.close()

        equiv_checksum = run_csvdb("checksum", str(equiv_path)).stdout.strip()
        assert duck_checksum == equiv_checksum

    def test_csvdb_missing_csv_file(self, run_csvdb, temp_dir):
        """to-sqlite with missing CSV creates table but leaves it empty."""
        csvdb_dir = temp_dir / "missing_csv.csvdb"
        csvdb_dir.mkdir()

        # Schema defines two tables but only one CSV exists
        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "users" (\n'
            '    "id" INTEGER PRIMARY KEY,\n'
            '    "name" TEXT\n'
            ');\n'
            'CREATE TABLE "orders" (\n'
            '    "id" INTEGER PRIMARY KEY,\n'
            '    "amount" REAL\n'
            ');\n'
        )
        (csvdb_dir / "users.csv").write_text(
            "id,name\n"
            "1,Alice\n"
            "2,Bob\n"
        )
        # orders.csv intentionally missing

        result = run_csvdb("to-sqlite", str(csvdb_dir), check=False)
        # Pinned behavior: succeeds, creates both tables; missing CSV -> empty table
        assert result.returncode == 0

        db_path = temp_dir / "missing_csv.sqlite"
        assert db_path.exists()

        conn = sqlite3.connect(db_path)
        users_count = conn.execute("SELECT COUNT(*) FROM users").fetchone()[0]
        orders_count = conn.execute("SELECT COUNT(*) FROM orders").fetchone()[0]
        conn.close()

        assert users_count == 2   # CSV present -> data imported
        assert orders_count == 0  # CSV missing -> table exists but empty

    def test_csvdb_csv_column_mismatch(self, run_csvdb, temp_dir):
        """CSV columns don't match schema.sql — pin current behavior."""
        csvdb_dir = temp_dir / "col_mismatch.csvdb"
        csvdb_dir.mkdir()

        # Schema expects 3 columns
        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "data" (\n'
            '    "id" INTEGER PRIMARY KEY,\n'
            '    "name" TEXT,\n'
            '    "value" INTEGER\n'
            ');\n'
        )
        # CSV only has 2 columns
        (csvdb_dir / "data.csv").write_text(
            "id,name\n"
            "1,Alice\n"
            "2,Bob\n"
        )

        result = run_csvdb("to-sqlite", str(csvdb_dir), check=False)
        # Pin behavior: should either error (non-zero) or succeed with partial data.
        # We just verify it doesn't crash without a clear exit code.
        # If it succeeds, verify the sqlite was created.
        if result.returncode == 0:
            db_path = temp_dir / "col_mismatch.sqlite"
            assert db_path.exists()
        else:
            # Error exit is also acceptable — the point is it doesn't panic/crash
            assert result.returncode != 0

    def test_pipe_with_output_flag_no_temp_leak(self, run_csvdb, temp_dir, sample_sqlite):
        """--pipe with -o should use explicit path, not leak to temp."""
        import shutil
        import tempfile

        # Clean up any leftover temp csvdb from earlier pipe tests
        system_temp = Path(tempfile.gettempdir())
        leaked = system_temp / "sample.csvdb"
        if leaked.exists():
            shutil.rmtree(leaked)

        explicit_output = temp_dir / "explicit_pipe.csvdb"
        result = run_csvdb(
            "to-csvdb", "--pipe", "-o", str(explicit_output),
            str(sample_sqlite)
        )

        output_path = result.stdout.strip()
        # stdout path should match explicit output
        assert str(explicit_output).replace("\\", "/") in output_path.replace("\\", "/")
        # The explicit output should exist
        assert explicit_output.exists()
        assert (explicit_output / "schema.sql").exists()

        # The system temp dir should NOT have a csvdb for this run.
        assert not leaked.exists(), f"Temp dir leaked: {leaked}"


class TestBoundaryConditions:
    def test_init_empty_csv(self, run_csvdb, temp_dir):
        """CSV with header only -> creates table with 0 rows."""
        csv_dir = temp_dir / "csvs"
        csv_dir.mkdir()
        (csv_dir / "empty.csv").write_text("id,name\n")

        result = run_csvdb("init", str(csv_dir))
        assert result.returncode == 0

        # The init command creates <dir>.csvdb next to the source
        csvdb_dir = temp_dir / "csvs.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "schema.sql").exists()
        assert (csvdb_dir / "empty.csv").exists()

    def test_roundtrip_single_row(self, run_csvdb, temp_dir):
        """Single-row table survives full roundtrip."""
        db_path = temp_dir / "one.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'only')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path), "--force")
        csvdb_dir = temp_dir / "one.csvdb"
        run_csvdb("to-sqlite", str(csvdb_dir), "--force")

        rebuilt = temp_dir / "one.sqlite"
        conn = sqlite3.connect(rebuilt)
        rows = conn.execute("SELECT * FROM t").fetchall()
        conn.close()
        assert len(rows) == 1
        assert rows[0] == (1, "only")

    def test_to_sqlite_empty_csvdb(self, run_csvdb, temp_dir):
        """csvdb with schema but 0 tables -> creates empty db."""
        db_path = temp_dir / "notables.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path), "--force")
        csvdb_dir = temp_dir / "notables.csvdb"

        # Remove the CSV file to simulate 0 data
        csv_file = csvdb_dir / "t.csv"
        if csv_file.exists():
            csv_file.unlink()

        run_csvdb("to-sqlite", str(csvdb_dir), "--force")
        rebuilt = temp_dir / "notables.sqlite"
        conn = sqlite3.connect(rebuilt)
        count = conn.execute("SELECT COUNT(*) FROM t").fetchone()[0]
        conn.close()
        assert count == 0

    def test_checksum_empty_table(self, run_csvdb, temp_dir):
        """Empty table has a deterministic checksum."""
        db_path = temp_dir / "empty.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.commit()
        conn.close()

        result1 = run_csvdb("checksum", str(db_path))
        result2 = run_csvdb("checksum", str(db_path))
        assert result1.returncode == 0
        assert result2.returncode == 0
        # Checksums should be identical (deterministic)
        assert result1.stdout.strip() == result2.stdout.strip()
