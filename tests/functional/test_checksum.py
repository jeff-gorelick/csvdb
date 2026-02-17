"""Functional tests for the checksum command."""

import sqlite3


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

    def test_checksum_parquetdb(self, run_csvdb, sample_sqlite):
        """checksum should work on a .parquetdb directory."""
        run_csvdb("to-parquetdb", str(sample_sqlite))
        parquetdb_dir = sample_sqlite.parent / "sample.parquetdb"

        result = run_csvdb("checksum", str(parquetdb_dir))
        checksum = result.stdout.strip()

        assert len(checksum) == 64
        assert all(c in "0123456789abcdef" for c in checksum)

    def test_checksum_parquetdb_matches_csvdb(self, run_csvdb, sample_sqlite):
        """Same data in csvdb and parquetdb should produce the same checksum."""
        run_csvdb("to-csvdb", str(sample_sqlite))
        run_csvdb("to-parquetdb", "--force", str(sample_sqlite))

        csvdb_dir = sample_sqlite.parent / "sample.csvdb"
        parquetdb_dir = sample_sqlite.parent / "sample.parquetdb"

        csvdb_checksum = run_csvdb("checksum", str(csvdb_dir)).stdout.strip()
        parquetdb_checksum = run_csvdb("checksum", str(parquetdb_dir)).stdout.strip()

        assert csvdb_checksum == parquetdb_checksum


class TestChecksumFilters:
    """Tests for --tables and --exclude checksum filters."""

    def test_checksum_tables_filter(self, run_csvdb, temp_dir):
        """--tables flag should produce a different hash than full checksum."""
        db_path = temp_dir / "multi.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO orders VALUES (1, 99.99)")
        conn.commit()
        conn.close()

        full_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()
        filtered_checksum = run_csvdb("checksum", "--tables=users", str(db_path)).stdout.strip()

        assert full_checksum != filtered_checksum
        assert len(filtered_checksum) == 64

    def test_checksum_exclude_filter(self, run_csvdb, temp_dir):
        """--exclude flag should produce a different hash than full checksum."""
        db_path = temp_dir / "multi.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO orders VALUES (1, 99.99)")
        conn.commit()
        conn.close()

        full_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()
        filtered_checksum = run_csvdb("checksum", "--exclude=orders", str(db_path)).stdout.strip()

        assert full_checksum != filtered_checksum
        assert len(filtered_checksum) == 64

    def test_checksum_filter_matches_subset(self, run_csvdb, temp_dir):
        """Checksum with --tables=X should match checksum of DB containing only X."""
        # Create multi-table database
        multi_db = temp_dir / "multi.sqlite"
        conn = sqlite3.connect(multi_db)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO users VALUES (2, 'Bob')")
        conn.execute("INSERT INTO orders VALUES (1, 99.99)")
        conn.commit()
        conn.close()

        # Create single-table database with same data
        single_db = temp_dir / "single.sqlite"
        conn = sqlite3.connect(single_db)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO users VALUES (2, 'Bob')")
        conn.commit()
        conn.close()

        filtered_checksum = run_csvdb("checksum", "--tables=users", str(multi_db)).stdout.strip()
        single_checksum = run_csvdb("checksum", str(single_db)).stdout.strip()

        assert filtered_checksum == single_checksum
