"""Functional tests for CLI behavior, flags, and determinism."""

import sqlite3


class TestCLIBehavior:
    """Tests for CLI behavior and arguments."""

    def test_help_flag(self, run_csvdb):
        """--help should show usage information."""
        result = run_csvdb("--help", check=False)
        assert result.returncode == 0
        assert "csvdb" in result.stdout.lower() or "usage" in result.stdout.lower()

    def test_subcommand_help(self, run_csvdb):
        """Subcommand --help should show subcommand usage."""
        result = run_csvdb("to-csvdb", "--help", check=False)
        assert result.returncode == 0

    def test_invalid_subcommand(self, run_csvdb):
        """Invalid subcommand should error."""
        result = run_csvdb("invalid-command", check=False)
        assert result.returncode != 0

    def test_missing_required_argument(self, run_csvdb):
        """Missing required argument should error."""
        result = run_csvdb("to-csvdb", check=False)
        assert result.returncode != 0

    def test_to_csv_output_flag_short(self, run_csvdb, sample_sqlite, temp_dir):
        """-o flag should work for output."""
        output_dir = temp_dir / "out1.csvdb"
        run_csvdb("to-csvdb", "-o", str(output_dir), str(sample_sqlite))
        assert output_dir.exists()

    def test_to_csv_output_flag_long(self, run_csvdb, sample_sqlite, temp_dir):
        """--output flag should work for output."""
        output_dir = temp_dir / "out2.csvdb"
        run_csvdb("to-csvdb", "--output", str(output_dir), str(sample_sqlite))
        assert output_dir.exists()


def make_multi_table_sqlite(db_path):
    """Create a SQLite database with 3 tables for filtering tests."""
    conn = sqlite3.connect(db_path)
    conn.execute("""
        CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL
        )
    """)
    conn.execute("INSERT INTO users VALUES (1, 'Alice')")
    conn.execute("INSERT INTO users VALUES (2, 'Bob')")

    conn.execute("""
        CREATE TABLE orders (
            id INTEGER PRIMARY KEY,
            user_id INTEGER NOT NULL,
            amount REAL NOT NULL
        )
    """)
    conn.execute("INSERT INTO orders VALUES (1, 1, 99.99)")
    conn.execute("INSERT INTO orders VALUES (2, 2, 49.50)")

    conn.execute("""
        CREATE TABLE products (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            price REAL NOT NULL
        )
    """)
    conn.execute("INSERT INTO products VALUES (1, 'Widget', 9.99)")
    conn.execute("INSERT INTO products VALUES (2, 'Gadget', 19.99)")

    conn.commit()
    conn.close()


class TestForceFlag:
    def test_to_csvdb_refuses_overwrite_without_force(self, run_csvdb, temp_dir):
        """Existing .csvdb dir blocks export without --force."""
        db_path = temp_dir / "test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        # First export succeeds
        result = run_csvdb("to-csvdb", str(db_path))
        assert result.returncode == 0

        # Second export should fail without --force
        result = run_csvdb("to-csvdb", str(db_path), check=False)
        assert result.returncode != 0
        assert "--force" in result.stderr

    def test_to_csvdb_overwrites_with_force(self, run_csvdb, temp_dir):
        """--force allows overwriting existing .csvdb dir."""
        db_path = temp_dir / "test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        # First export
        run_csvdb("to-csvdb", str(db_path))

        # Second export with --force succeeds
        result = run_csvdb("to-csvdb", str(db_path), "--force")
        assert result.returncode == 0

    def test_to_sqlite_refuses_overwrite_without_force(self, run_csvdb, sample_csvdb):
        """Existing .sqlite file blocks import without --force."""
        # First import
        run_csvdb("to-sqlite", str(sample_csvdb), "--force")

        # Second import should fail without --force
        result = run_csvdb("to-sqlite", str(sample_csvdb), check=False)
        assert result.returncode != 0
        assert "--force" in result.stderr

    def test_to_sqlite_overwrites_with_force(self, run_csvdb, sample_csvdb):
        """--force allows overwriting existing .sqlite file."""
        # First import
        run_csvdb("to-sqlite", str(sample_csvdb), "--force")

        # Second import with --force
        result = run_csvdb("to-sqlite", str(sample_csvdb), "--force")
        assert result.returncode == 0


class TestTableFiltering:
    def test_tables_flag_exports_only_named_tables(self, run_csvdb, temp_dir):
        """--tables exports only the specified tables' CSV files."""
        db_path = temp_dir / "multi.sqlite"
        make_multi_table_sqlite(db_path)

        result = run_csvdb("to-csvdb", str(db_path), "--tables", "users", "--force")
        assert result.returncode == 0

        csvdb_dir = temp_dir / "multi.csvdb"
        assert (csvdb_dir / "schema.sql").exists()
        assert (csvdb_dir / "users.csv").exists()
        assert not (csvdb_dir / "orders.csv").exists()
        assert not (csvdb_dir / "products.csv").exists()

    def test_exclude_flag_skips_named_tables(self, run_csvdb, temp_dir):
        """--exclude skips the specified table's CSV file."""
        db_path = temp_dir / "multi.sqlite"
        make_multi_table_sqlite(db_path)

        result = run_csvdb("to-csvdb", str(db_path), "--exclude", "users", "--force")
        assert result.returncode == 0

        csvdb_dir = temp_dir / "multi.csvdb"
        assert (csvdb_dir / "schema.sql").exists()
        assert not (csvdb_dir / "users.csv").exists()
        assert (csvdb_dir / "orders.csv").exists()
        assert (csvdb_dir / "products.csv").exists()

    def test_tables_and_exclude_conflict(self, run_csvdb, temp_dir):
        """Using both --tables and --exclude produces an error."""
        db_path = temp_dir / "multi.sqlite"
        make_multi_table_sqlite(db_path)

        result = run_csvdb(
            "to-csvdb", str(db_path),
            "--tables", "users",
            "--exclude", "orders",
            check=False
        )
        assert result.returncode != 0

    def test_filter_to_sqlite_imports_subset(self, run_csvdb, temp_dir):
        """--tables on to-sqlite only imports data for the named table."""
        db_path = temp_dir / "multi.sqlite"
        make_multi_table_sqlite(db_path)

        # Export to csvdb first
        run_csvdb("to-csvdb", str(db_path), "--force")

        csvdb_dir = temp_dir / "multi.csvdb"

        # Import only users
        run_csvdb("to-sqlite", str(csvdb_dir), "--tables", "users", "--force")

        # Verify: users has data, orders is empty
        rebuilt = temp_dir / "multi.sqlite"
        conn = sqlite3.connect(rebuilt)
        user_count = conn.execute("SELECT COUNT(*) FROM users").fetchone()[0]
        order_count = conn.execute("SELECT COUNT(*) FROM orders").fetchone()[0]
        conn.close()

        assert user_count == 2
        assert order_count == 0


class TestCsvdbToml:
    def test_toml_written_on_export(self, run_csvdb, temp_dir):
        """to-csvdb creates a csvdb.toml with order and null_mode."""
        db_path = temp_dir / "test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO t VALUES (1, 'a')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path), "--force")

        toml_path = temp_dir / "test.csvdb" / "csvdb.toml"
        assert toml_path.exists()

        content = toml_path.read_text()
        assert "order" in content
        assert "null_mode" in content

    def test_toml_records_options(self, run_csvdb, temp_dir):
        """csvdb.toml records the order and null_mode used during export."""
        db_path = temp_dir / "test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        run_csvdb(
            "to-csvdb", str(db_path),
            "--order", "all-columns",
            "--null-mode", "empty",
            "--force"
        )

        content = (temp_dir / "test.csvdb" / "csvdb.toml").read_text()
        assert 'order = "all-columns"' in content
        assert 'null_mode = "empty"' in content

    def test_toml_parse_valid(self, run_csvdb, temp_dir):
        """csvdb.toml is valid TOML that can be parsed."""
        import tomllib  # Python 3.11+

        db_path = temp_dir / "test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path), "--force")

        toml_path = temp_dir / "test.csvdb" / "csvdb.toml"
        with open(toml_path, "rb") as f:
            data = tomllib.load(f)

        assert data["order"] == "pk"
        assert data["null_mode"] == "marker"


class TestOrderingAndDeterminism:
    """Tests for ordering and deterministic output."""

    def test_csv_sorted_by_pk(self, run_csvdb, temp_dir):
        """CSV output should be sorted by primary key."""
        db_path = temp_dir / "unsorted.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        # Insert out of order
        conn.execute("INSERT INTO data VALUES (3, 'three')")
        conn.execute("INSERT INTO data VALUES (1, 'one')")
        conn.execute("INSERT INTO data VALUES (5, 'five')")
        conn.execute("INSERT INTO data VALUES (2, 'two')")
        conn.execute("INSERT INTO data VALUES (4, 'four')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        csv_content = (temp_dir / "unsorted.csvdb" / "data.csv").read_text()
        lines = csv_content.strip().split('\n')
        # Skip header, check data rows are in PK order
        data_lines = lines[1:]
        ids = [line.split(',')[0].strip('"') for line in data_lines]
        assert ids == ['1', '2', '3', '4', '5']

    def test_deterministic_output(self, run_csvdb, temp_dir):
        """Multiple runs should produce identical output."""
        db_path = temp_dir / "deterministic.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [
            (1, "one"), (2, "two"), (3, "three")
        ])
        conn.commit()
        conn.close()

        # Run twice to different outputs
        out1 = temp_dir / "run1.csvdb"
        out2 = temp_dir / "run2.csvdb"

        run_csvdb("to-csvdb", "-o", str(out1), str(db_path))
        run_csvdb("to-csvdb", "-o", str(out2), str(db_path))

        # Compare outputs
        csv1 = (out1 / "data.csv").read_text()
        csv2 = (out2 / "data.csv").read_text()
        schema1 = (out1 / "schema.sql").read_text()
        schema2 = (out2 / "schema.sql").read_text()

        assert csv1 == csv2
        assert schema1 == schema2

    def test_checksum_deterministic(self, run_csvdb, sample_sqlite):
        """Checksum should be deterministic across runs."""
        checksum1 = run_csvdb("checksum", str(sample_sqlite)).stdout.strip()
        checksum2 = run_csvdb("checksum", str(sample_sqlite)).stdout.strip()
        checksum3 = run_csvdb("checksum", str(sample_sqlite)).stdout.strip()

        assert checksum1 == checksum2 == checksum3

    def test_composite_pk_ordering(self, run_csvdb, temp_dir):
        """Composite PK should order by all key columns."""
        db_path = temp_dir / "composite_order.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE data (
                a INTEGER,
                b INTEGER,
                val TEXT,
                PRIMARY KEY (a, b)
            )
        """)
        # Insert out of order
        conn.executemany("INSERT INTO data VALUES (?, ?, ?)", [
            (2, 1, "2-1"),
            (1, 2, "1-2"),
            (1, 1, "1-1"),
            (2, 2, "2-2"),
        ])
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        csv_content = (temp_dir / "composite_order.csvdb" / "data.csv").read_text()
        lines = csv_content.strip().split('\n')[1:]  # Skip header
        values = [line.split(',')[2].strip('"') for line in lines]
        assert values == ['1-1', '1-2', '2-1', '2-2']


class TestProgressBars:
    def test_progress_bar_hidden_in_pipe_mode(self, run_csvdb, temp_dir):
        """--pipe mode doesn't show progress bar output on stderr."""
        db_path = temp_dir / "test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        conn.execute("INSERT INTO t VALUES (1)")
        conn.commit()
        conn.close()

        result = run_csvdb("to-csvdb", str(db_path), "--pipe")
        # In pipe mode (stderr not a TTY in subprocess), progress bar should be hidden
        # stderr should only have warnings or nothing - not progress bar characters
        assert "[" not in result.stderr or "Warning" in result.stderr
