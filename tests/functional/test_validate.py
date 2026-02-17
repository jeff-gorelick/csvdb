"""Functional tests for the validate command."""



class TestValidate:
    def test_validate_valid_csvdb(self, run_csvdb, sample_csvdb):
        """A valid .csvdb directory passes validation."""
        result = run_csvdb("validate", str(sample_csvdb))
        assert result.returncode == 0
        assert "OK" in result.stdout
        assert "0 warnings" in result.stdout

    def test_validate_missing_csv(self, run_csvdb, temp_dir):
        """Reports warning for missing CSV file."""
        csvdb_dir = temp_dir / "broken.csvdb"
        csvdb_dir.mkdir()

        # Schema references a table, but no CSV file exists
        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "missing" (\n'
            '    "id" INTEGER PRIMARY KEY\n'
            ');\n'
        )

        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0  # warnings don't cause failure
        assert "missing CSV" in result.stdout

    def test_validate_column_mismatch(self, run_csvdb, temp_dir):
        """Reports warning when CSV headers don't match schema."""
        csvdb_dir = temp_dir / "mismatch.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "t" (\n'
            '    "id" INTEGER PRIMARY KEY,\n'
            '    "name" TEXT\n'
            ');\n'
        )

        # CSV has wrong columns
        (csvdb_dir / "t.csv").write_text(
            "id,wrong_column\n"
            "1,bad\n"
        )

        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0
        assert "column mismatch" in result.stdout or "WARN" in result.stdout

    def test_validate_bad_schema(self, run_csvdb, temp_dir):
        """Invalid SQL in schema.sql reports an error."""
        csvdb_dir = temp_dir / "bad.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text("THIS IS NOT SQL;\n")

        result = run_csvdb("validate", str(csvdb_dir), check=False)
        # Should report error
        assert "ERROR" in result.stdout or result.returncode != 0


class TestValidateDuplicatePK:
    def test_validate_duplicate_pk_warns(self, run_csvdb, temp_dir):
        """Duplicate PK values in CSV should produce a warning."""
        csvdb_dir = temp_dir / "dupkey.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "t" (\n'
            '    "id" INTEGER PRIMARY KEY,\n'
            '    "name" TEXT\n'
            ');\n'
        )
        # id=1 appears twice
        (csvdb_dir / "t.csv").write_text("id,name\n1,Alice\n1,Bob\n2,Charlie\n")

        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0  # warnings don't cause failure
        assert "duplicate" in result.stdout.lower()

    def test_validate_unique_pks_no_warning(self, run_csvdb, temp_dir):
        """Unique PKs should not produce duplicate warnings."""
        csvdb_dir = temp_dir / "unique.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "t" (\n'
            '    "id" INTEGER PRIMARY KEY,\n'
            '    "name" TEXT\n'
            ');\n'
        )
        (csvdb_dir / "t.csv").write_text("id,name\n1,Alice\n2,Bob\n3,Charlie\n")

        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0
        assert "0 warnings" in result.stdout

    def test_validate_no_pk_skips_check(self, run_csvdb, temp_dir):
        """Tables without PK skip duplicate check."""
        csvdb_dir = temp_dir / "nopk.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "t" (\n'
            '    "a" TEXT,\n'
            '    "b" TEXT\n'
            ');\n'
        )
        (csvdb_dir / "t.csv").write_text("a,b\nfoo,bar\nfoo,bar\n")

        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0
        assert "duplicate" not in result.stdout.lower()


class TestValidateEdgeCases:
    def test_validate_empty_table(self, run_csvdb, temp_dir):
        """Schema with table, CSV has header only (0 rows) -> OK."""
        csvdb_dir = temp_dir / "empty.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "t" (\n'
            '    "id" INTEGER PRIMARY KEY,\n'
            '    "name" TEXT\n'
            ');\n'
        )
        # Header only, no data rows
        (csvdb_dir / "t.csv").write_text("id,name\n")

        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0
        assert "0 rows" in result.stdout

    def test_validate_extra_csv_ignored(self, run_csvdb, temp_dir):
        """CSV files not in schema are silently ignored."""
        csvdb_dir = temp_dir / "extra.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "t" (\n'
            '    "id" INTEGER PRIMARY KEY\n'
            ');\n'
        )
        (csvdb_dir / "t.csv").write_text("id\n1\n")
        # Extra CSV not referenced in schema
        (csvdb_dir / "orphan.csv").write_text("x\n1\n")

        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0
        assert "0 warnings" in result.stdout

    def test_validate_with_toml(self, run_csvdb, temp_dir):
        """Valid csvdb.toml is reported OK; invalid toml produces a warning."""
        csvdb_dir = temp_dir / "toml.csvdb"
        csvdb_dir.mkdir()

        (csvdb_dir / "schema.sql").write_text(
            'CREATE TABLE "t" (\n'
            '    "id" INTEGER PRIMARY KEY\n'
            ');\n'
        )
        (csvdb_dir / "t.csv").write_text("id\n1\n")

        # Valid TOML
        (csvdb_dir / "csvdb.toml").write_text('order = "pk"\nnull_mode = "marker"\n')
        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0
        assert "csvdb.toml" in result.stdout
        assert "OK" in result.stdout

        # Invalid TOML
        (csvdb_dir / "csvdb.toml").write_text("{{{not valid toml")
        result = run_csvdb("validate", str(csvdb_dir))
        assert result.returncode == 0  # warning, not failure
        assert "WARN" in result.stdout or "warning" in result.stdout.lower()
