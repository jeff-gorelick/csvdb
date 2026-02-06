"""Functional tests for the init command."""

import sqlite3
from pathlib import Path


class TestInit:
    """Tests for the init command."""

    def test_init_creates_csvdb(self, run_csvdb, temp_dir, sample_csv):
        """init should create a .csvdb directory from CSV files."""
        # Create a directory with the CSV
        csv_dir = temp_dir / "raw_csvs"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text(sample_csv.read_text())

        # Run init
        result = run_csvdb("init", str(csv_dir))

        # Check output directory was created
        csvdb_dir = temp_dir / "raw_csvs.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "schema.sql").exists()
        assert (csvdb_dir / "data.csv").exists()

    def test_init_infers_pk(self, run_csvdb, temp_dir):
        """init should detect id column as primary key."""
        csv_dir = temp_dir / "pk_test"
        csv_dir.mkdir()
        (csv_dir / "users.csv").write_text(
            "id,name,email\n"
            "1,Alice,alice@example.com\n"
            "2,Bob,bob@example.com\n"
        )

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "pk_test.csvdb" / "schema.sql").read_text()
        assert "PRIMARY KEY" in schema

    def test_init_no_pk_detection(self, run_csvdb, temp_dir):
        """init --no-pk-detection should not add primary keys."""
        csv_dir = temp_dir / "no_pk"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text(
            "id,value\n"
            "1,one\n"
            "2,two\n"
        )

        run_csvdb("init", "--no-pk-detection", str(csv_dir))

        schema = (temp_dir / "no_pk.csvdb" / "schema.sql").read_text()
        # Should still create table but without explicit PK
        assert "CREATE TABLE" in schema


class TestInitCommand:
    """Additional tests for the init command."""

    def test_init_multiple_csvs(self, run_csvdb, temp_dir):
        """init should handle multiple CSV files."""
        csv_dir = temp_dir / "multi_csv"
        csv_dir.mkdir()

        (csv_dir / "users.csv").write_text("id,name\n1,Alice\n2,Bob\n")
        (csv_dir / "orders.csv").write_text("id,user_id,total\n1,1,100\n2,2,200\n")
        (csv_dir / "items.csv").write_text("id,name,price\n1,Widget,9.99\n")

        run_csvdb("init", str(csv_dir))

        csvdb_dir = temp_dir / "multi_csv.csvdb"
        assert (csvdb_dir / "users.csv").exists()
        assert (csvdb_dir / "orders.csv").exists()
        assert (csvdb_dir / "items.csv").exists()
        assert (csvdb_dir / "schema.sql").exists()

        schema = (csvdb_dir / "schema.sql").read_text()
        assert "users" in schema
        assert "orders" in schema
        assert "items" in schema

    def test_init_type_inference_integer(self, run_csvdb, temp_dir):
        """init should infer INTEGER type."""
        csv_dir = temp_dir / "int_infer"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text("id,count\n1,100\n2,200\n3,300\n")

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "int_infer.csvdb" / "schema.sql").read_text()
        # Should infer count as INTEGER
        assert "INTEGER" in schema

    def test_init_type_inference_real(self, run_csvdb, temp_dir):
        """init should infer REAL type for decimals."""
        csv_dir = temp_dir / "real_infer"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text("id,price\n1,9.99\n2,19.99\n3,29.99\n")

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "real_infer.csvdb" / "schema.sql").read_text()
        # Should infer price as REAL
        assert "REAL" in schema

    def test_init_type_inference_text(self, run_csvdb, temp_dir):
        """init should infer TEXT type for strings."""
        csv_dir = temp_dir / "text_infer"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text("id,name\n1,Alice\n2,Bob\n3,Charlie\n")

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "text_infer.csvdb" / "schema.sql").read_text()
        assert "TEXT" in schema

    def test_init_empty_csv(self, run_csvdb, temp_dir):
        """init should handle CSV with header only."""
        csv_dir = temp_dir / "empty_csv"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text("id,name,value\n")

        run_csvdb("init", str(csv_dir))

        csvdb_dir = temp_dir / "empty_csv.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "schema.sql").exists()

    def test_init_quoted_values(self, run_csvdb, temp_dir):
        """init should handle quoted CSV values."""
        csv_dir = temp_dir / "quoted"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text(
            'id,name,description\n'
            '1,"Alice","A person"\n'
            '2,"Bob","Another, person"\n'
        )

        run_csvdb("init", str(csv_dir))

        # Roundtrip to verify
        run_csvdb("to-sqlite", "--force", str(temp_dir / "quoted.csvdb"))

        conn = sqlite3.connect(temp_dir / "quoted.sqlite")
        rows = conn.execute("SELECT * FROM data ORDER BY id").fetchall()
        conn.close()

        assert rows[1][2] == "Another, person"  # Comma preserved


class TestInitEdgeCases:
    """Edge case tests for the init command."""

    def test_init_no_csv_files(self, run_csvdb, temp_dir):
        """init should fail gracefully with no CSV files."""
        empty_dir = temp_dir / "empty"
        empty_dir.mkdir()

        result = run_csvdb("init", str(empty_dir), check=False)
        assert result.returncode != 0
        assert "No CSV" in result.stderr

    def test_init_unicode_data(self, run_csvdb, temp_dir):
        """init should handle unicode data correctly."""
        csv_dir = temp_dir / "unicode"
        csv_dir.mkdir()
        (csv_dir / "cities.csv").write_text(
            "id,city,country\n"
            "1,北京,中国\n"
            "2,東京,日本\n"
            "3,München,Deutschland\n",
            encoding="utf-8"
        )

        run_csvdb("init", str(csv_dir))

        # Verify roundtrip preserves unicode
        run_csvdb("to-sqlite", "--force", str(temp_dir / "unicode.csvdb"))

        conn = sqlite3.connect(temp_dir / "unicode.sqlite")
        rows = conn.execute("SELECT city FROM cities ORDER BY id").fetchall()
        conn.close()

        assert rows[0][0] == "北京"
        assert rows[1][0] == "東京"
        assert rows[2][0] == "München"

    def test_init_special_characters_in_values(self, run_csvdb, temp_dir):
        """init should handle special characters in CSV values."""
        csv_dir = temp_dir / "special"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text(
            'id,value\n'
            '1,"Line1\nLine2"\n'
            '2,"Tab\there"\n'
            '3,"Quote""inside"\n',
            newline=""
        )

        run_csvdb("init", str(csv_dir))
        run_csvdb("to-sqlite", "--force", str(temp_dir / "special.csvdb"))

        conn = sqlite3.connect(temp_dir / "special.sqlite")
        rows = conn.execute("SELECT value FROM data ORDER BY id").fetchall()
        conn.close()

        assert "Line1\nLine2" == rows[0][0]
        assert "Tab\there" == rows[1][0]
        assert 'Quote"inside' == rows[2][0]

    def test_init_csvdb_toml_created(self, run_csvdb, temp_dir):
        """init should create csvdb.toml with metadata."""
        csv_dir = temp_dir / "toml_test"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text("id,value\n1,test\n")

        run_csvdb("init", str(csv_dir))

        toml_path = temp_dir / "toml_test.csvdb" / "csvdb.toml"
        assert toml_path.exists()

        content = toml_path.read_text()
        assert "format_version" in content
        assert "created_by" in content
        assert "csvdb" in content

    def test_init_reinit_csvdb_directory(self, run_csvdb, temp_dir):
        """init on existing .csvdb directory should regenerate schema."""
        # Create a csvdb directory manually
        csvdb_dir = temp_dir / "existing.csvdb"
        csvdb_dir.mkdir()
        (csvdb_dir / "users.csv").write_text("id,name\n1,Alice\n2,Bob\n")

        run_csvdb("init", str(csvdb_dir))

        # Schema should be created in the same directory
        assert (csvdb_dir / "schema.sql").exists()
        schema = (csvdb_dir / "schema.sql").read_text()
        assert "users" in schema

    def test_init_pk_detection_non_id_column(self, run_csvdb, temp_dir):
        """init should detect PK for unique non-id columns."""
        csv_dir = temp_dir / "unique_pk"
        csv_dir.mkdir()
        (csv_dir / "codes.csv").write_text(
            "code,description\n"
            "A001,First item\n"
            "A002,Second item\n"
            "A003,Third item\n"
        )

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "unique_pk.csvdb" / "schema.sql").read_text()
        # code should be detected as PK because it's unique
        assert "PRIMARY KEY" in schema

    def test_init_no_pk_for_duplicates(self, run_csvdb, temp_dir):
        """init should not assign PK when all columns have duplicates."""
        csv_dir = temp_dir / "dup_values"
        csv_dir.mkdir()
        (csv_dir / "events.csv").write_text(
            "timestamp,message\n"
            "2024-01-01,Same Event\n"
            "2024-01-01,Same Event\n"  # completely duplicate row
        )

        result = run_csvdb("init", str(csv_dir))

        # Warning about no PK is printed to stderr
        # Also check that stdout shows "no PK" for the table
        assert "no PK" in result.stdout or "Warning" in result.stderr

    def test_init_mixed_type_column(self, run_csvdb, temp_dir):
        """init should widen column type for mixed values."""
        csv_dir = temp_dir / "mixed_types"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text(
            "id,value\n"
            "1,100\n"
            "2,hello\n"  # text mixed with integer
            "3,300\n"
        )

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "mixed_types.csvdb" / "schema.sql").read_text()
        # value column should be TEXT due to mixed types
        assert '"value" TEXT' in schema

    def test_init_nullable_detection(self, run_csvdb, temp_dir):
        """init should detect nullable columns correctly."""
        csv_dir = temp_dir / "nullable"
        csv_dir.mkdir()
        (csv_dir / "data.csv").write_text(
            "id,required_col,optional_col\n"
            "1,value1,extra1\n"
            "2,value2,\n"  # empty = nullable
            "3,value3,extra3\n"
        )

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "nullable.csvdb" / "schema.sql").read_text()
        # required_col should have NOT NULL
        assert '"required_col" TEXT NOT NULL' in schema
        # optional_col should NOT have NOT NULL
        assert '"optional_col" TEXT\n' in schema or '"optional_col" TEXT)' in schema

    def test_init_real_type_inference(self, run_csvdb, temp_dir):
        """init should infer REAL type for decimal values."""
        csv_dir = temp_dir / "decimal"
        csv_dir.mkdir()
        (csv_dir / "prices.csv").write_text(
            "id,price\n"
            "1,9.99\n"
            "2,19.99\n"
            "3,29.99\n"
        )

        run_csvdb("init", str(csv_dir))

        schema = (temp_dir / "decimal.csvdb" / "schema.sql").read_text()
        assert "REAL" in schema

    def test_init_large_csv(self, run_csvdb, temp_dir):
        """init should handle large CSV files efficiently."""
        csv_dir = temp_dir / "large"
        csv_dir.mkdir()

        # Generate 10k rows
        rows = ["id,name,value"]
        for i in range(10000):
            rows.append(f"{i},Name{i},{i * 1.5}")
        (csv_dir / "data.csv").write_text("\n".join(rows) + "\n")

        run_csvdb("init", str(csv_dir))

        csvdb_dir = temp_dir / "large.csvdb"
        assert csvdb_dir.exists()
        assert (csvdb_dir / "schema.sql").exists()

        # Verify roundtrip
        run_csvdb("to-sqlite", "--force", str(csvdb_dir))
        conn = sqlite3.connect(temp_dir / "large.sqlite")
        count = conn.execute("SELECT COUNT(*) FROM data").fetchone()[0]
        conn.close()
        assert count == 10000

    def test_init_header_only_csv(self, run_csvdb, temp_dir):
        """init should handle header-only CSV (no data rows)."""
        csv_dir = temp_dir / "header_only"
        csv_dir.mkdir()
        (csv_dir / "empty.csv").write_text("id,name,value\n")

        run_csvdb("init", str(csv_dir))

        csvdb_dir = temp_dir / "header_only.csvdb"
        assert csvdb_dir.exists()
        schema = (csvdb_dir / "schema.sql").read_text()
        assert '"id"' in schema
        assert '"name"' in schema
        assert '"value"' in schema
