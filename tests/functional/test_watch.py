"""Functional tests for the watch command."""

import subprocess
import time
from pathlib import Path


class TestWatch:
    """Tests for the watch command."""

    def test_watch_requires_csvdb_dir(self, run_csvdb, temp_dir):
        """watch should error if path is not a valid .csvdb directory."""
        result = run_csvdb("watch", str(temp_dir), "--target", "sqlite", check=False)
        assert result.returncode != 0
        assert "schema.sql" in result.stderr

    def test_watch_initial_build(self, csvdb_bin, sample_csvdb):
        """watch should do an initial build before waiting for changes."""
        # Start watch in background, let it do initial build, then kill it
        proc = subprocess.Popen(
            [csvdb_bin, "watch", str(sample_csvdb), "--target", "sqlite"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        # Give it time to do the initial build
        time.sleep(2)
        proc.terminate()
        try:
            _, stderr = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            _, stderr = proc.communicate()

        # Should have done an initial build
        assert "Watching:" in stderr
        assert "Built:" in stderr

        # The sqlite file should exist
        sqlite_path = sample_csvdb.parent / "sample.sqlite"
        assert sqlite_path.exists()

    def test_watch_rebuilds_on_change(self, csvdb_bin, sample_csvdb):
        """watch should rebuild when a CSV file changes."""
        proc = subprocess.Popen(
            [csvdb_bin, "watch", str(sample_csvdb), "--target", "sqlite", "--debounce", "200"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        # Wait for initial build
        time.sleep(3)

        # Get initial mtime of sqlite
        sqlite_path = sample_csvdb.parent / "sample.sqlite"
        assert sqlite_path.exists()
        initial_mtime = sqlite_path.stat().st_mtime

        # Modify the CSV
        time.sleep(1)  # ensure time difference
        csv_path = sample_csvdb / "items.csv"
        csv_path.write_text(
            "id,name,price\n"
            "1,Widget,9.99\n"
            "2,Gadget,29.99\n"
            "3,Gizmo,39.99\n"
            "4,Doohickey,49.99\n"
        )

        # Wait for rebuild (poll for up to 10s)
        deadline = time.time() + 10
        rebuilt = False
        while time.time() < deadline:
            new_mtime = sqlite_path.stat().st_mtime
            if new_mtime > initial_mtime:
                rebuilt = True
                break
            time.sleep(0.5)

        proc.terminate()
        try:
            _, stderr = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            _, stderr = proc.communicate()

        # Should have detected change and rebuilt
        assert "Change detected" in stderr
        assert rebuilt, "sqlite file was not updated after CSV change"

    def test_watch_duckdb_target(self, csvdb_bin, sample_csvdb):
        """watch --target duckdb should build a DuckDB file."""
        proc = subprocess.Popen(
            [csvdb_bin, "watch", str(sample_csvdb), "--target", "duckdb"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        time.sleep(2)
        proc.terminate()
        try:
            _, stderr = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            _, stderr = proc.communicate()

        assert "Built:" in stderr
        duckdb_path = sample_csvdb.parent / "sample.duckdb"
        assert duckdb_path.exists()

    def test_watch_survives_build_error(self, csvdb_bin, sample_csvdb):
        """watch should continue watching after a build error."""
        proc = subprocess.Popen(
            [csvdb_bin, "watch", str(sample_csvdb), "--target", "sqlite", "--debounce", "200"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        # Wait for initial build
        time.sleep(2)

        # Break the schema.sql to cause a build error
        schema_path = sample_csvdb / "schema.sql"
        original_schema = schema_path.read_text()
        schema_path.write_text("INVALID SQL GARBAGE")

        # Wait for error
        time.sleep(3)

        # Fix the schema back
        schema_path.write_text(original_schema)

        # Wait for successful rebuild
        time.sleep(3)

        proc.terminate()
        try:
            _, stderr = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            _, stderr = proc.communicate()

        # Should have seen build error but continued
        assert "Build error" in stderr
        # Should have rebuilt after fix
        assert stderr.count("Built:") >= 2
