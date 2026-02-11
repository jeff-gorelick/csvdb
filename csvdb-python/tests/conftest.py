"""Pytest configuration for csvdb-python binding tests."""

import os
import sqlite3
import pytest
from pathlib import Path


@pytest.fixture
def temp_dir(tmp_path):
    return tmp_path


@pytest.fixture
def sample_csvdb(temp_dir):
    """Create a sample .csvdb directory."""
    csvdb_dir = temp_dir / "sample.csvdb"
    csvdb_dir.mkdir()

    (csvdb_dir / "schema.sql").write_text(
        'CREATE TABLE "users" (\n'
        '    "id" INTEGER PRIMARY KEY,\n'
        '    "name" TEXT NOT NULL,\n'
        '    "score" INTEGER\n'
        ');\n'
    )

    (csvdb_dir / "users.csv").write_text(
        "id,name,score\n"
        "1,Alice,95\n"
        "2,Bob,87\n"
        "3,Charlie,92\n"
    )

    return csvdb_dir


@pytest.fixture
def sample_sqlite(temp_dir):
    """Create a sample SQLite database."""
    db_path = temp_dir / "sample.sqlite"
    conn = sqlite3.connect(db_path)
    conn.execute("""
        CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            score INTEGER
        )
    """)
    conn.execute("INSERT INTO users VALUES (1, 'Alice', 95)")
    conn.execute("INSERT INTO users VALUES (2, 'Bob', 87)")
    conn.execute("INSERT INTO users VALUES (3, 'Charlie', 92)")
    conn.commit()
    conn.close()
    return db_path


@pytest.fixture
def sample_csvdb_with_nulls(temp_dir):
    """Create a .csvdb directory with NULL values."""
    csvdb_dir = temp_dir / "nulls.csvdb"
    csvdb_dir.mkdir()

    (csvdb_dir / "schema.sql").write_text(
        'CREATE TABLE "data" (\n'
        '    "id" INTEGER PRIMARY KEY,\n'
        '    "value" TEXT\n'
        ');\n'
    )

    (csvdb_dir / "data.csv").write_text(
        "id,value\n"
        "1,hello\n"
        "2,\\N\n"
        "3,world\n"
    )

    return csvdb_dir


@pytest.fixture
def raw_csv_dir(temp_dir):
    """Create a directory with raw CSV files for init."""
    csv_dir = temp_dir / "raw"
    csv_dir.mkdir()

    (csv_dir / "products.csv").write_text(
        "id,name,price\n"
        "1,Widget,9.99\n"
        "2,Gadget,19.99\n"
        "3,Gizmo,29.99\n"
    )

    return csv_dir


@pytest.fixture
def raw_csv_dir_with_fks(temp_dir):
    """Create a directory with raw CSV files that have FK relationships."""
    csv_dir = temp_dir / "raw_fk"
    csv_dir.mkdir()

    (csv_dir / "users.csv").write_text(
        "id,name\n"
        "1,Alice\n"
        "2,Bob\n"
    )

    (csv_dir / "orders.csv").write_text(
        "id,user_id,amount\n"
        "100,1,99.99\n"
        "101,2,49.50\n"
    )

    return csv_dir
