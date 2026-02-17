"""Functional tests for schema handling: views, indexes, and edge cases."""

import sqlite3


class TestViews:
    """Tests for database views."""

    def test_view_preserved_in_schema(self, run_csvdb, temp_dir):
        """Views should be preserved in schema.sql."""
        db_path = temp_dir / "with_views.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE employees (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                department TEXT,
                salary INTEGER
            )
        """)
        conn.executemany(
            "INSERT INTO employees VALUES (?, ?, ?, ?)",
            [
                (1, "Alice", "Engineering", 100000),
                (2, "Bob", "Engineering", 95000),
                (3, "Charlie", "Sales", 80000),
                (4, "Diana", "Sales", 85000),
            ]
        )
        conn.execute("""
            CREATE VIEW engineering_team AS
            SELECT id, name, salary FROM employees WHERE department = 'Engineering'
        """)
        conn.execute("""
            CREATE VIEW high_earners AS
            SELECT name, salary FROM employees WHERE salary > 90000
        """)
        conn.commit()
        conn.close()

        # Convert to csvdb
        run_csvdb("to-csvdb", str(db_path))
        csvdb_dir = temp_dir / "with_views.csvdb"

        # Check schema contains views
        schema = (csvdb_dir / "schema.sql").read_text()
        assert "CREATE VIEW" in schema
        assert "engineering_team" in schema
        assert "high_earners" in schema

    def test_view_roundtrip_sqlite(self, run_csvdb, temp_dir):
        """Views should survive SQLite -> csvdb -> SQLite roundtrip."""
        db_path = temp_dir / "views_rt.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, price REAL)")
        conn.executemany(
            "INSERT INTO items VALUES (?, ?, ?)",
            [(1, "Widget", 9.99), (2, "Gadget", 19.99), (3, "Gizmo", 29.99)]
        )
        conn.execute("CREATE VIEW expensive AS SELECT * FROM items WHERE price > 15")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Roundtrip
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "views_rt.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "views_rt.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

        # Verify view works
        conn = sqlite3.connect(temp_dir / "views_rt.sqlite")
        rows = conn.execute("SELECT * FROM expensive ORDER BY id").fetchall()
        conn.close()
        assert len(rows) == 2
        assert rows[0][1] == "Gadget"

    def test_view_roundtrip_duckdb(self, run_csvdb, temp_dir):
        """Views should survive SQLite -> csvdb -> DuckDB roundtrip."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "views_duck.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, stock INTEGER)")
        conn.executemany(
            "INSERT INTO products VALUES (?, ?, ?)",
            [(1, "Apple", 100), (2, "Banana", 0), (3, "Cherry", 50)]
        )
        conn.execute("CREATE VIEW in_stock AS SELECT * FROM products WHERE stock > 0")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Convert to csvdb then DuckDB
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "views_duck.csvdb"))

        duckdb_checksum = run_csvdb("checksum", str(temp_dir / "views_duck.duckdb")).stdout.strip()
        assert original_checksum == duckdb_checksum

        # Verify view works in DuckDB
        duck_conn = duckdb.connect(str(temp_dir / "views_duck.duckdb"))
        rows = duck_conn.execute("SELECT * FROM in_stock ORDER BY id").fetchall()
        duck_conn.close()
        assert len(rows) == 2


class TestDependentViews:
    """Tests for views that depend on other views."""

    def test_view_depending_on_view(self, run_csvdb, temp_dir):
        """Views depending on other views should work."""
        db_path = temp_dir / "dep_views.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE employees (id INTEGER PRIMARY KEY, name TEXT, salary INTEGER)")
        conn.executemany("INSERT INTO employees VALUES (?, ?, ?)", [
            (1, "Alice", 100000),
            (2, "Bob", 80000),
            (3, "Charlie", 120000),
        ])
        # First view
        conn.execute("CREATE VIEW high_earners AS SELECT * FROM employees WHERE salary > 90000")
        # View depending on first view
        conn.execute("CREATE VIEW top_earner AS SELECT * FROM high_earners ORDER BY salary DESC LIMIT 1")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        schema = (temp_dir / "dep_views.csvdb" / "schema.sql").read_text()
        assert "high_earners" in schema
        assert "top_earner" in schema

        # Roundtrip
        run_csvdb("to-sqlite", "--force", str(temp_dir / "dep_views.csvdb"))

        conn = sqlite3.connect(temp_dir / "dep_views.sqlite")
        result = conn.execute("SELECT name FROM top_earner").fetchone()
        conn.close()
        assert result[0] == "Charlie"

    def test_multiple_dependent_views(self, run_csvdb, temp_dir):
        """Multiple levels of view dependencies should work."""
        db_path = temp_dir / "multi_dep.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val INTEGER)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [(i, i * 10) for i in range(1, 6)])
        conn.execute("CREATE VIEW step1 AS SELECT * FROM data WHERE val > 20")
        conn.execute("CREATE VIEW step2 AS SELECT * FROM step1 WHERE val < 50")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "multi_dep.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "multi_dep.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_view_reverse_alpha_dependency(self, run_csvdb, temp_dir):
        """View a_derived depends on z_base — must work via DuckDB despite alpha order."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "reverse_alpha.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val INTEGER)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [(1, 10), (2, 20), (3, 30)])
        conn.execute("CREATE VIEW z_base AS SELECT * FROM data WHERE val > 10")
        conn.execute("CREATE VIEW a_derived AS SELECT * FROM z_base WHERE val < 30")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "reverse_alpha.csvdb"))

        duck_checksum = run_csvdb("checksum", str(temp_dir / "reverse_alpha.duckdb")).stdout.strip()
        assert original_checksum == duck_checksum

        duck_conn = duckdb.connect(str(temp_dir / "reverse_alpha.duckdb"))
        rows = duck_conn.execute("SELECT * FROM a_derived ORDER BY id").fetchall()
        duck_conn.close()
        assert len(rows) == 1
        assert rows[0][1] == 20

    def test_three_level_view_chain(self, run_csvdb, temp_dir):
        """Three-level chain with reverse-alpha names roundtrips through DuckDB."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "three_level.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val INTEGER)")
        conn.executemany("INSERT INTO data VALUES (?, ?)", [(i, i * 10) for i in range(1, 6)])
        conn.execute("CREATE VIEW z_step1 AS SELECT * FROM data WHERE val > 10")
        conn.execute("CREATE VIEW m_step2 AS SELECT * FROM z_step1 WHERE val < 50")
        conn.execute("CREATE VIEW a_step3 AS SELECT * FROM m_step2 WHERE val > 20")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "three_level.csvdb"))

        duck_checksum = run_csvdb("checksum", str(temp_dir / "three_level.duckdb")).stdout.strip()
        assert original_checksum == duck_checksum

        duck_conn = duckdb.connect(str(temp_dir / "three_level.duckdb"))
        rows = duck_conn.execute("SELECT * FROM a_step3 ORDER BY id").fetchall()
        duck_conn.close()
        assert len(rows) == 2  # val=30 and val=40

    def test_diamond_view_dependency(self, run_csvdb, temp_dir):
        """Diamond dependency: two views share a base, a fourth depends on both."""
        try:
            import duckdb
        except ImportError:
            import pytest
            pytest.skip("duckdb not installed")

        db_path = temp_dir / "diamond.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val INTEGER, cat TEXT)")
        conn.executemany("INSERT INTO data VALUES (?, ?, ?)", [
            (1, 10, "a"), (2, 20, "b"), (3, 30, "a"), (4, 40, "b"),
        ])
        conn.execute("CREATE VIEW z_base AS SELECT * FROM data WHERE val > 10")
        conn.execute("CREATE VIEW m_left AS SELECT * FROM z_base WHERE cat = 'a'")
        conn.execute("CREATE VIEW m_right AS SELECT * FROM z_base WHERE cat = 'b'")
        conn.execute("""
            CREATE VIEW a_combined AS
            SELECT * FROM m_left
            UNION ALL
            SELECT * FROM m_right
        """)
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-duckdb", "--force", str(temp_dir / "diamond.csvdb"))

        duck_checksum = run_csvdb("checksum", str(temp_dir / "diamond.duckdb")).stdout.strip()
        assert original_checksum == duck_checksum

        duck_conn = duckdb.connect(str(temp_dir / "diamond.duckdb"))
        rows = duck_conn.execute("SELECT * FROM a_combined ORDER BY id").fetchall()
        duck_conn.close()
        assert len(rows) == 3  # val > 10: ids 2,3,4


class TestSchemaEdgeCases:
    """Tests for schema edge cases."""

    def test_composite_primary_key(self, run_csvdb, temp_dir):
        """Should handle composite primary keys."""
        db_path = temp_dir / "composite_pk.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE order_items (
                order_id INTEGER,
                item_id INTEGER,
                quantity INTEGER,
                PRIMARY KEY (order_id, item_id)
            )
        """)
        conn.executemany("INSERT INTO order_items VALUES (?, ?, ?)", [
            (1, 1, 5),
            (1, 2, 3),
            (2, 1, 1),
        ])
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        # Roundtrip
        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "composite_pk.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "composite_pk.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_reserved_keyword_table_name(self, run_csvdb, temp_dir):
        """Should handle reserved SQL keywords as table names."""
        db_path = temp_dir / "keywords.sqlite"
        conn = sqlite3.connect(db_path)
        # 'order', 'select', 'group' are reserved keywords
        conn.execute('CREATE TABLE "order" (id INTEGER PRIMARY KEY, val TEXT)')
        conn.execute('INSERT INTO "order" VALUES (1, "test")')
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "keywords.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "keywords.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_reserved_keyword_column_name(self, run_csvdb, temp_dir):
        """Should handle reserved SQL keywords as column names."""
        db_path = temp_dir / "col_keywords.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute('''
            CREATE TABLE data (
                id INTEGER PRIMARY KEY,
                "select" TEXT,
                "from" INTEGER,
                "where" REAL
            )
        ''')
        conn.execute('INSERT INTO data VALUES (1, "test", 42, 3.14)')
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "col_keywords.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "col_keywords.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_long_table_name(self, run_csvdb, temp_dir):
        """Should handle very long table names."""
        db_path = temp_dir / "long_name.sqlite"
        conn = sqlite3.connect(db_path)
        long_name = "a" * 100
        conn.execute(f'CREATE TABLE "{long_name}" (id INTEGER PRIMARY KEY, val TEXT)')
        conn.execute(f'INSERT INTO "{long_name}" VALUES (1, "test")')
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "long_name.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "long_name.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_special_characters_in_column_name(self, run_csvdb, temp_dir):
        """Should handle special characters in column names."""
        db_path = temp_dir / "special_cols.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute('''
            CREATE TABLE data (
                id INTEGER PRIMARY KEY,
                "column with spaces" TEXT,
                "column-with-dashes" INTEGER
            )
        ''')
        conn.execute('INSERT INTO data VALUES (1, "test", 42)')
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "special_cols.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "special_cols.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum

    def test_many_columns(self, run_csvdb, temp_dir):
        """Should handle tables with many columns."""
        db_path = temp_dir / "many_cols.sqlite"
        conn = sqlite3.connect(db_path)

        # Create table with 50 columns
        cols = ", ".join([f"col_{i} TEXT" for i in range(50)])
        conn.execute(f"CREATE TABLE wide (id INTEGER PRIMARY KEY, {cols})")

        # Insert a row
        values = ", ".join([f"'val_{i}'" for i in range(50)])
        conn.execute(f"INSERT INTO wide VALUES (1, {values})")
        conn.commit()
        conn.close()

        original_checksum = run_csvdb("checksum", str(db_path)).stdout.strip()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "many_cols.csvdb"))

        rebuilt_checksum = run_csvdb("checksum", str(temp_dir / "many_cols.sqlite")).stdout.strip()
        assert original_checksum == rebuilt_checksum


class TestIndexPreservation:
    """Tests for index preservation."""

    def test_unique_index_preserved(self, run_csvdb, temp_dir):
        """Unique indexes should be preserved in schema."""
        db_path = temp_dir / "unique_idx.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        conn.execute("CREATE UNIQUE INDEX idx_email ON users(email)")
        conn.execute("INSERT INTO users VALUES (1, 'alice@example.com')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        schema = (temp_dir / "unique_idx.csvdb" / "schema.sql").read_text()
        assert "CREATE" in schema and "INDEX" in schema
        assert "idx_email" in schema

    def test_multi_column_index(self, run_csvdb, temp_dir):
        """Multi-column indexes should be preserved."""
        db_path = temp_dir / "multi_idx.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("""
            CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                customer_id INTEGER,
                order_date TEXT,
                status TEXT
            )
        """)
        conn.execute("CREATE INDEX idx_customer_date ON orders(customer_id, order_date)")
        conn.execute("INSERT INTO orders VALUES (1, 100, '2024-01-01', 'pending')")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))

        schema = (temp_dir / "multi_idx.csvdb" / "schema.sql").read_text()
        assert "idx_customer_date" in schema

    def test_index_roundtrip_sqlite(self, run_csvdb, temp_dir):
        """Indexes should survive SQLite roundtrip."""
        db_path = temp_dir / "idx_rt.sqlite"
        conn = sqlite3.connect(db_path)
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT, num INTEGER)")
        conn.execute("CREATE INDEX idx_val ON data(val)")
        conn.execute("CREATE UNIQUE INDEX idx_num ON data(num)")
        conn.execute("INSERT INTO data VALUES (1, 'test', 42)")
        conn.commit()
        conn.close()

        run_csvdb("to-csvdb", str(db_path))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "idx_rt.csvdb"))

        # Check indexes exist in rebuilt database
        conn = sqlite3.connect(temp_dir / "idx_rt.sqlite")
        indexes = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%'"
        ).fetchall()
        conn.close()

        index_names = [idx[0] for idx in indexes]
        assert "idx_val" in index_names
        assert "idx_num" in index_names
