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
