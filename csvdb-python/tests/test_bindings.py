"""Tests for csvdb Python bindings."""

import os
import pytest

import csvdb


class TestVersion:
    def test_version_returns_string(self):
        v = csvdb.version()
        assert isinstance(v, str)
        assert v.startswith("0.")


class TestToCsvdb:
    def test_from_sqlite(self, sample_sqlite, temp_dir):
        output = str(temp_dir / "out.csvdb")
        result = csvdb.py_to_csvdb(str(sample_sqlite), output=output)
        assert result == output
        assert os.path.isdir(output)
        assert os.path.isfile(os.path.join(output, "schema.sql"))
        assert os.path.isfile(os.path.join(output, "users.csv"))

    def test_force_overwrites(self, sample_sqlite, temp_dir):
        output = str(temp_dir / "out.csvdb")
        csvdb.py_to_csvdb(str(sample_sqlite), output=output)
        # Should succeed with force=True
        csvdb.py_to_csvdb(str(sample_sqlite), output=output, force=True)

    def test_no_force_fails_on_existing(self, sample_sqlite, temp_dir):
        output = str(temp_dir / "out.csvdb")
        csvdb.py_to_csvdb(str(sample_sqlite), output=output)
        with pytest.raises(RuntimeError, match="already exists"):
            csvdb.py_to_csvdb(str(sample_sqlite), output=output, force=False)

    def test_compress(self, sample_sqlite, temp_dir):
        output = str(temp_dir / "compressed.csvdb")
        csvdb.py_to_csvdb(str(sample_sqlite), output=output, compress=True)
        assert os.path.isfile(os.path.join(output, "users.csv.gz"))


class TestToCsvdbIncremental:
    def test_incremental_first_run(self, sample_sqlite, temp_dir):
        output = str(temp_dir / "inc.csvdb")
        result = csvdb.py_to_csvdb_incremental(str(sample_sqlite), output=output)
        assert result["path"] == output
        assert isinstance(result["added"], list)
        assert isinstance(result["unchanged"], list)


class TestToSqlite:
    def test_from_csvdb(self, sample_csvdb):
        result = csvdb.py_to_sqlite(str(sample_csvdb), force=True)
        assert result.endswith(".sqlite")
        assert os.path.isfile(result)

    def test_tables_filter(self, sample_csvdb):
        result = csvdb.py_to_sqlite(str(sample_csvdb), force=True, tables=["users"])
        assert os.path.isfile(result)


class TestToDuckdb:
    def test_from_csvdb(self, sample_csvdb):
        result = csvdb.py_to_duckdb(str(sample_csvdb), force=True)
        assert result.endswith(".duckdb")
        assert os.path.isfile(result)


class TestToParquetdb:
    def test_from_csvdb(self, sample_csvdb, temp_dir):
        output = str(temp_dir / "out.parquetdb")
        result = csvdb.py_to_parquetdb(str(sample_csvdb), output=output, force=True)
        assert result == output
        assert os.path.isdir(output)


class TestSqlQuery:
    def test_basic_query(self, sample_sqlite):
        rows = csvdb.sql_query(str(sample_sqlite), "SELECT * FROM users ORDER BY id")
        assert len(rows) == 3
        assert rows[0]["name"] == "Alice"
        assert rows[0]["id"] == "1"
        assert rows[1]["name"] == "Bob"

    def test_query_with_where(self, sample_sqlite):
        rows = csvdb.sql_query(str(sample_sqlite), "SELECT name FROM users WHERE score > 90")
        names = {r["name"] for r in rows}
        assert "Alice" in names
        assert "Charlie" in names
        assert "Bob" not in names

    def test_query_count(self, sample_sqlite):
        rows = csvdb.sql_query(str(sample_sqlite), "SELECT count(*) as cnt FROM users")
        assert len(rows) == 1
        assert rows[0]["cnt"] == "3"

    def test_query_csvdb(self, sample_csvdb):
        rows = csvdb.sql_query(str(sample_csvdb), "SELECT * FROM users ORDER BY id")
        assert len(rows) == 3
        assert rows[0]["name"] == "Alice"

    def test_null_values(self, sample_csvdb_with_nulls):
        rows = csvdb.sql_query(str(sample_csvdb_with_nulls), "SELECT * FROM data ORDER BY id")
        assert rows[0]["value"] == "hello"
        assert rows[1]["value"] is None  # NULL
        assert rows[2]["value"] == "world"

    def test_rejects_non_select(self, sample_sqlite):
        with pytest.raises(RuntimeError, match="SELECT"):
            csvdb.sql_query(str(sample_sqlite), "DROP TABLE users")

    def test_with_cte(self, sample_sqlite):
        rows = csvdb.sql_query(
            str(sample_sqlite),
            "WITH cte AS (SELECT name FROM users) SELECT * FROM cte ORDER BY name"
        )
        assert len(rows) == 3
        assert rows[0]["name"] == "Alice"


class TestChecksum:
    def test_returns_hex_hash(self, sample_csvdb):
        h = csvdb.checksum_db(str(sample_csvdb))
        assert len(h) == 64
        assert all(c in "0123456789abcdef" for c in h)

    def test_same_data_same_hash(self, sample_csvdb):
        h1 = csvdb.checksum_db(str(sample_csvdb))
        h2 = csvdb.checksum_db(str(sample_csvdb))
        assert h1 == h2

    def test_consistency_across_formats(self, sample_csvdb):
        csvdb_hash = csvdb.checksum_db(str(sample_csvdb))
        sqlite_path = csvdb.py_to_sqlite(str(sample_csvdb), force=True)
        sqlite_hash = csvdb.checksum_db(sqlite_path)
        assert csvdb_hash == sqlite_hash


class TestDiff:
    def test_identical(self, sample_csvdb):
        result = csvdb.diff_db(str(sample_csvdb), str(sample_csvdb))
        assert result is False  # no differences

    def test_different(self, temp_dir):
        dir1 = temp_dir / "a.csvdb"
        dir1.mkdir()
        (dir1 / "schema.sql").write_text('CREATE TABLE "t" ("id" INTEGER PRIMARY KEY, "v" TEXT);\n')
        (dir1 / "t.csv").write_text("id,v\n1,hello\n")

        dir2 = temp_dir / "b.csvdb"
        dir2.mkdir()
        (dir2 / "schema.sql").write_text('CREATE TABLE "t" ("id" INTEGER PRIMARY KEY, "v" TEXT);\n')
        (dir2 / "t.csv").write_text("id,v\n1,world\n")

        result = csvdb.diff_db(str(dir1), str(dir2))
        assert result is True  # has differences


class TestValidate:
    def test_valid(self, sample_csvdb):
        result = csvdb.validate_db(str(sample_csvdb))
        assert result["table_count"] == 1
        assert result["errors"] == []

    def test_invalid_missing_schema(self, temp_dir):
        bad_dir = temp_dir / "bad.csvdb"
        bad_dir.mkdir()
        result = csvdb.validate_db(str(bad_dir))
        assert len(result["errors"]) > 0


class TestInit:
    def test_init_creates_csvdb(self, raw_csv_dir):
        result = csvdb.init_csvdb(str(raw_csv_dir))
        assert "output_dir" in result
        assert os.path.isdir(result["output_dir"])
        assert os.path.isfile(os.path.join(result["output_dir"], "schema.sql"))

    def test_init_returns_table_info(self, raw_csv_dir):
        result = csvdb.init_csvdb(str(raw_csv_dir))
        assert len(result["tables"]) == 1
        table = result["tables"][0]
        assert table["name"] == "products"
        assert table["row_count"] == 3
        assert table["column_count"] == 3

    def test_init_detect_pk_disabled(self, raw_csv_dir):
        result = csvdb.init_csvdb(str(raw_csv_dir), detect_pk=False)
        table = result["tables"][0]
        assert table["suggested_pk"] is None

    def test_init_detects_foreign_keys(self, raw_csv_dir_with_fks):
        result = csvdb.init_csvdb(str(raw_csv_dir_with_fks))
        orders = next(t for t in result["tables"] if t["name"] == "orders")
        assert len(orders["suggested_fks"]) == 1
        fk = orders["suggested_fks"][0]
        assert fk["column"] == "user_id"
        assert fk["references_table"] == "users"
        assert fk["references_column"] == "id"

    def test_init_detect_fk_disabled(self, raw_csv_dir_with_fks):
        result = csvdb.init_csvdb(str(raw_csv_dir_with_fks), detect_fk=False)
        orders = next(t for t in result["tables"] if t["name"] == "orders")
        assert len(orders["suggested_fks"]) == 0

    def test_init_no_fk_without_matching_table(self, raw_csv_dir):
        result = csvdb.init_csvdb(str(raw_csv_dir))
        table = result["tables"][0]
        assert len(table["suggested_fks"]) == 0


class TestErrorHandling:
    def test_bad_path(self):
        with pytest.raises(RuntimeError):
            csvdb.checksum_db("/nonexistent/path.csvdb")

    def test_bad_order_mode(self, sample_sqlite, temp_dir):
        with pytest.raises(RuntimeError, match="Unknown order mode"):
            csvdb.py_to_csvdb(str(sample_sqlite), output=str(temp_dir / "x.csvdb"), order="invalid")

    def test_bad_null_mode(self, sample_sqlite, temp_dir):
        with pytest.raises(RuntimeError, match="Unknown null mode"):
            csvdb.py_to_csvdb(str(sample_sqlite), output=str(temp_dir / "x.csvdb"), null_mode="invalid")
