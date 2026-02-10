"""Functional tests for the to-duckdb command."""

import sqlite3
from pathlib import Path


class TestToDuckdb:
    """Tests for the to-duckdb command."""

    def test_to_duckdb_creates_db(self, run_csvdb, sample_csvdb):
        """to-duckdb should create DuckDB database from .csvdb."""
        run_csvdb("to-duckdb", "--force", str(sample_csvdb))

        db_path = sample_csvdb.parent / "sample.duckdb"
        assert db_path.exists()
        assert db_path.stat().st_size > 0

    def test_to_duckdb_data_accessible(self, run_csvdb, sample_csvdb):
        """to-duckdb should create queryable database."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        run_csvdb("to-duckdb", "--force", str(sample_csvdb))

        db_path = sample_csvdb.parent / "sample.duckdb"
        conn = duckdb.connect(str(db_path))
        rows = conn.execute("SELECT * FROM items ORDER BY id").fetchall()
        conn.close()

        assert len(rows) == 3
        assert rows[0][1] == "Widget"


class TestForeignKeys:
    """Tests for FK dependency ordering in DuckDB conversion."""

    def test_fk_tables_to_duckdb(self, run_csvdb, temp_dir):
        """Tables with foreign keys should convert to DuckDB successfully."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        # Create SQLite with FK relationship: orders references users
        db_path = temp_dir / "fk_test.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("PRAGMA foreign_keys = ON")
        conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)"
        )
        conn.execute(
            "CREATE TABLE orders ("
            "  id INTEGER PRIMARY KEY,"
            "  user_id INTEGER NOT NULL REFERENCES users(id),"
            "  amount REAL NOT NULL"
            ")"
        )
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO users VALUES (2, 'Bob')")
        conn.execute("INSERT INTO orders VALUES (1, 1, 99.99)")
        conn.execute("INSERT INTO orders VALUES (2, 2, 49.50)")
        conn.execute("INSERT INTO orders VALUES (3, 1, 25.00)")
        conn.commit()
        conn.close()

        # SQLite -> csvdb -> DuckDB (exercises FK ordering)
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "fk_test.csvdb"))

        # Verify data
        duck = duckdb.connect(str(temp_dir / "fk_test.duckdb"))
        users = duck.execute("SELECT COUNT(*) FROM users").fetchone()[0]
        orders = duck.execute("SELECT COUNT(*) FROM orders").fetchone()[0]
        total = duck.execute(
            "SELECT SUM(amount) FROM orders WHERE user_id = 1"
        ).fetchone()[0]
        duck.close()

        assert users == 2
        assert orders == 3
        assert abs(total - 124.99) < 0.01

    def test_fk_schema_order(self, run_csvdb, temp_dir):
        """schema.sql should write referenced tables before dependent tables."""
        db_path = temp_dir / "fk_order.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)"
        )
        conn.execute(
            "CREATE TABLE orders ("
            "  id INTEGER PRIMARY KEY,"
            "  user_id INTEGER REFERENCES users(id)"
            ")"
        )
        conn.execute("INSERT INTO users VALUES (1, 'Alice')")
        conn.execute("INSERT INTO orders VALUES (1, 1)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        schema = (temp_dir / "fk_order.csvdb" / "schema.sql").read_text()
        users_pos = schema.index("CREATE TABLE users")
        orders_pos = schema.index("CREATE TABLE orders")
        assert users_pos < orders_pos, "users should appear before orders in schema.sql"

    def test_fk_chain_to_duckdb(self, run_csvdb, temp_dir):
        """Three-level FK chain should convert to DuckDB successfully."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "fk_chain.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("PRAGMA foreign_keys = ON")
        conn.execute(
            "CREATE TABLE countries (id INTEGER PRIMARY KEY, name TEXT NOT NULL)"
        )
        conn.execute(
            "CREATE TABLE cities ("
            "  id INTEGER PRIMARY KEY,"
            "  name TEXT NOT NULL,"
            "  country_id INTEGER REFERENCES countries(id)"
            ")"
        )
        conn.execute(
            "CREATE TABLE addresses ("
            "  id INTEGER PRIMARY KEY,"
            "  street TEXT NOT NULL,"
            "  city_id INTEGER REFERENCES cities(id)"
            ")"
        )
        conn.execute("INSERT INTO countries VALUES (1, 'US')")
        conn.execute("INSERT INTO cities VALUES (1, 'NYC', 1)")
        conn.execute("INSERT INTO addresses VALUES (1, '123 Main St', 1)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "fk_chain.csvdb"))

        duck = duckdb.connect(str(temp_dir / "fk_chain.duckdb"))
        street = duck.execute("SELECT street FROM addresses").fetchone()[0]
        duck.close()

        assert street == "123 Main St"


class TestIndexRoundtripDuckDB:
    """Tests for index preservation through DuckDB."""

    def test_index_preserved_in_csvdb(self, run_csvdb, temp_dir):
        """Indexes should be preserved in csvdb schema from SQLite."""
        db_path = temp_dir / "idx_src.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, name TEXT)")
        conn.execute("CREATE UNIQUE INDEX idx_email ON users(email)")
        conn.execute("CREATE INDEX idx_name ON users(name)")
        conn.execute("INSERT INTO users VALUES (1, 'alice@test.com', 'Alice')")
        conn.commit()
        conn.close()

        # SQLite -> CSV preserves indexes in schema
        run_csvdb("to-csvdb", str(db_path))

        schema = (temp_dir / "idx_src.csvdb" / "schema.sql").read_text()
        assert "idx_email" in schema
        assert "idx_name" in schema

        # CSV -> SQLite restores indexes
        run_csvdb("to-sqlite", "--force", str(temp_dir / "idx_src.csvdb"))

        conn = sqlite3.connect(temp_dir / "idx_src.sqlite")
        indexes = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%'"
        ).fetchall()
        conn.close()

        index_names = [idx[0] for idx in indexes]
        assert "idx_email" in index_names

    def test_duckdb_data_preserved(self, run_csvdb, temp_dir):
        """Data should be preserved through DuckDB even if indexes aren't."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "duck_data.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        conn.execute("CREATE UNIQUE INDEX idx_email ON users(email)")
        conn.execute("INSERT INTO users VALUES (1, 'alice@test.com')")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # SQLite -> CSV -> DuckDB
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "duck_data.csvdb"))

        # DuckDB checksum should match (data is same, indexes excluded from checksum)
        duck_checksum = run_csvdb("checksum", str(temp_dir / "duck_data.duckdb")).stdout.strip()
        assert original_checksum == duck_checksum
