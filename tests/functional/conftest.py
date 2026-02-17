"""Pytest configuration and fixtures for csvdb functional tests."""

import subprocess
import pytest
from pathlib import Path


def get_csvdb_binary():
    """Get path to csvdb binary, building if necessary."""
    import platform
    repo_root = Path(__file__).parent.parent.parent

    # Determine binary name based on platform
    exe_suffix = ".exe" if platform.system() == "Windows" else ""
    bin_name = f"csvdb{exe_suffix}"

    # Try release binary first, then debug
    # In workspace layout, binaries are built at the workspace root target/
    release_bin = repo_root / "target" / "release" / bin_name
    debug_bin = repo_root / "target" / "debug" / bin_name

    if release_bin.exists():
        return str(release_bin)
    elif debug_bin.exists():
        return str(debug_bin)
    else:
        # Build release binary
        subprocess.run(
            ["cargo", "build", "--release", "-p", "csvdb"],
            cwd=repo_root,
            check=True,
            capture_output=True
        )
        return str(release_bin)


CSVDB_BIN = None


@pytest.fixture(scope="session")
def csvdb_bin():
    """Fixture providing path to csvdb binary."""
    global CSVDB_BIN
    if CSVDB_BIN is None:
        CSVDB_BIN = get_csvdb_binary()
    return CSVDB_BIN


@pytest.fixture
def run_csvdb(csvdb_bin):
    """Fixture providing a function to run csvdb commands."""
    def _run(*args, check=True, capture=True, cwd=None):
        cmd = [csvdb_bin] + list(args)
        result = subprocess.run(
            cmd,
            capture_output=capture,
            text=True,
            encoding="utf-8",
            check=False,
            cwd=cwd,
        )
        if check and result.returncode != 0:
            raise subprocess.CalledProcessError(
                result.returncode, cmd,
                output=result.stdout,
                stderr=result.stderr
            )
        return result
    return _run


@pytest.fixture
def temp_dir(tmp_path):
    """Fixture providing a temporary directory."""
    return tmp_path


@pytest.fixture
def sample_csv(temp_dir):
    """Create a sample CSV file for testing."""
    csv_path = temp_dir / "sample.csv"
    csv_path.write_text(
        "id,name,value\n"
        "1,Alice,100\n"
        "2,Bob,200\n"
        "3,Charlie,300\n"
    )
    return csv_path


@pytest.fixture
def sample_sqlite(temp_dir):
    """Create a sample SQLite database for testing."""
    import sqlite3

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
def sample_csvdb(temp_dir):
    """Create a sample .csvdb directory for testing."""
    csvdb_dir = temp_dir / "sample.csvdb"
    csvdb_dir.mkdir()

    # schema.sql
    (csvdb_dir / "schema.sql").write_text(
        'CREATE TABLE "items" (\n'
        '    "id" INTEGER PRIMARY KEY,\n'
        '    "name" TEXT NOT NULL,\n'
        '    "price" REAL\n'
        ');\n'
    )

    # items.csv
    (csvdb_dir / "items.csv").write_text(
        "id,name,price\n"
        "1,Widget,9.99\n"
        "2,Gadget,19.99\n"
        "3,Gizmo,29.99\n"
    )

    return csvdb_dir
