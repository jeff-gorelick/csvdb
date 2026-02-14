"""Tests for csvdb DataFrame bindings (Arrow, pandas, polars)."""

import os
import pytest

import csvdb
import pyarrow as pa
import pandas as pd
import polars as pl


class TestToArrow:
    def test_single_table(self, sample_csvdb):
        table = csvdb.to_arrow(str(sample_csvdb), "users")
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert table.column_names == ["id", "name", "score"]

    def test_all_tables(self, sample_csvdb):
        tables = csvdb.to_arrow(str(sample_csvdb))
        assert isinstance(tables, dict)
        assert "users" in tables
        assert isinstance(tables["users"], pa.Table)
        assert tables["users"].num_rows == 3

    def test_table_not_found(self, sample_csvdb):
        with pytest.raises(RuntimeError, match="not found"):
            csvdb.to_arrow(str(sample_csvdb), "nonexistent")

    def test_null_values(self, sample_csvdb_with_nulls):
        table = csvdb.to_arrow(str(sample_csvdb_with_nulls), "data")
        data = table.to_pydict()
        assert data["value"][0] == "hello"
        assert data["value"][1] is None
        assert data["value"][2] == "world"

    def test_from_sqlite(self, sample_sqlite):
        table = csvdb.to_arrow(str(sample_sqlite), "users")
        assert table.num_rows == 3


class TestSqlArrow:
    def test_basic_query(self, sample_csvdb):
        table = csvdb.sql_arrow(str(sample_csvdb), "SELECT name, score FROM users ORDER BY id")
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert table.column_names == ["name", "score"]

    def test_with_filter(self, sample_csvdb):
        table = csvdb.sql_arrow(str(sample_csvdb), "SELECT name FROM users WHERE score > 90")
        assert table.num_rows == 2

    def test_rejects_non_select(self, sample_sqlite):
        with pytest.raises(RuntimeError, match="SELECT"):
            csvdb.sql_arrow(str(sample_sqlite), "DROP TABLE users")


class TestToPandas:
    def test_single_table(self, sample_csvdb):
        df = csvdb.to_pandas(str(sample_csvdb), "users")
        assert isinstance(df, pd.DataFrame)
        assert len(df) == 3
        assert list(df.columns) == ["id", "name", "score"]

    def test_all_tables(self, sample_csvdb):
        dfs = csvdb.to_pandas(str(sample_csvdb))
        assert isinstance(dfs, dict)
        assert "users" in dfs
        assert isinstance(dfs["users"], pd.DataFrame)

    def test_from_sqlite(self, sample_sqlite):
        df = csvdb.to_pandas(str(sample_sqlite), "users")
        assert len(df) == 3


class TestSqlPandas:
    def test_basic_query(self, sample_csvdb):
        df = csvdb.sql_pandas(str(sample_csvdb), "SELECT name FROM users ORDER BY id")
        assert isinstance(df, pd.DataFrame)
        assert len(df) == 3
        assert df.iloc[0]["name"] == "Alice"


class TestToPolars:
    def test_single_table(self, sample_csvdb):
        df = csvdb.to_polars(str(sample_csvdb), "users")
        assert isinstance(df, pl.DataFrame)
        assert len(df) == 3
        assert df.columns == ["id", "name", "score"]

    def test_all_tables(self, sample_csvdb):
        dfs = csvdb.to_polars(str(sample_csvdb))
        assert isinstance(dfs, dict)
        assert "users" in dfs
        assert isinstance(dfs["users"], pl.DataFrame)

    def test_from_sqlite(self, sample_sqlite):
        df = csvdb.to_polars(str(sample_sqlite), "users")
        assert len(df) == 3


class TestSqlPolars:
    def test_basic_query(self, sample_csvdb):
        df = csvdb.sql_polars(str(sample_csvdb), "SELECT name FROM users ORDER BY id")
        assert isinstance(df, pl.DataFrame)
        assert len(df) == 3
        assert df["name"][0] == "Alice"


class TestWriteFromDataFrames:
    def test_write_from_pyarrow(self, temp_dir):
        table = pa.table({
            "id": pa.array([1, 2, 3], type=pa.int32()),
            "name": pa.array(["Alice", "Bob", "Charlie"]),
            "score": pa.array([95, 87, None], type=pa.int32()),
        })
        output = str(temp_dir / "from_arrow.csvdb")
        result = csvdb.to_csvdb({"users": table}, output=output)
        assert result == output
        assert os.path.isfile(os.path.join(output, "schema.sql"))
        assert os.path.isfile(os.path.join(output, "users.csv"))

        # Roundtrip
        read_back = csvdb.to_arrow(result, "users")
        assert read_back.num_rows == 3
        data = read_back.to_pydict()
        assert data["name"] == ["Alice", "Bob", "Charlie"]
        assert data["score"][2] is None

    def test_write_from_pandas(self, temp_dir):
        df = pd.DataFrame({
            "id": [1, 2],
            "name": ["Alice", "Bob"],
        })
        output = str(temp_dir / "from_pandas.csvdb")
        csvdb.to_csvdb({"users": df}, output=output)

        read_back = csvdb.to_pandas(output, "users")
        assert len(read_back) == 2

    def test_write_from_polars(self, temp_dir):
        df = pl.DataFrame({
            "id": [1, 2],
            "name": ["Alice", "Bob"],
        })
        output = str(temp_dir / "from_polars.csvdb")
        csvdb.to_csvdb({"users": df}, output=output)

        read_back = csvdb.to_polars(output, "users")
        assert len(read_back) == 2

    def test_write_multi_table(self, temp_dir):
        users = pa.table({"id": [1, 2], "name": ["Alice", "Bob"]})
        orders = pa.table({"id": [1], "user_id": [1], "total": [100.0]})
        output = str(temp_dir / "multi.csvdb")
        csvdb.to_csvdb({"users": users, "orders": orders}, output=output)

        tables = csvdb.to_arrow(output)
        assert sorted(tables.keys()) == ["orders", "users"]

    def test_write_requires_output(self, temp_dir):
        table = pa.table({"id": [1]})
        with pytest.raises(RuntimeError, match="output is required"):
            csvdb.to_csvdb({"t": table})

    def test_write_force(self, temp_dir):
        table = pa.table({"id": [1]})
        output = str(temp_dir / "force.csvdb")
        csvdb.to_csvdb({"t": table}, output=output)

        # Without force should fail
        with pytest.raises(RuntimeError, match="already exists"):
            csvdb.to_csvdb({"t": table}, output=output)

        # With force should succeed
        csvdb.to_csvdb({"t": table}, output=output, force=True)

    def test_roundtrip_with_nulls(self, temp_dir):
        """Full roundtrip: DataFrame -> csvdb -> DataFrame with NULL values."""
        original = pl.DataFrame({
            "id": [1, 2, 3],
            "name": ["Alice", None, "Charlie"],
            "score": [95, 87, None],
        })
        output = str(temp_dir / "nulls.csvdb")
        csvdb.to_csvdb({"data": original}, output=output)

        read_back = csvdb.to_polars(output, "data")
        assert read_back["name"][0] == "Alice"
        assert read_back["name"][1] is None
        assert read_back["score"][2] is None
