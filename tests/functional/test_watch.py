"""Functional tests for the watch command."""

import subprocess
import time
import os
import threading


def read_stderr_lines(proc, lines, stop_event):
    """Read stderr lines from a process in a background thread."""
    while not stop_event.is_set():
        line = proc.stderr.readline()
        if line:
            lines.append(line)
        elif proc.poll() is not None:
            break


def wait_for_output(lines, substring, timeout=15):
    """Poll collected stderr lines until substring appears or timeout."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if any(substring in line for line in lines):
            return True
        time.sleep(0.3)
    return False


def start_watch(csvdb_bin, csvdb_dir, target="sqlite", debounce=None):
    """Start a watch process and return (proc, lines, stop_event)."""
    cmd = [csvdb_bin, "watch", str(csvdb_dir), "--target", target]
    if debounce:
        cmd += ["--debounce", str(debounce)]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    lines = []
    stop_event = threading.Event()
    thread = threading.Thread(target=read_stderr_lines, args=(proc, lines, stop_event), daemon=True)
    thread.start()
    return proc, lines, stop_event


def stop_watch(proc, stop_event):
    """Terminate a watch process cleanly."""
    stop_event.set()
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


class TestWatch:
    """Tests for the watch command."""

    def test_watch_requires_csvdb_dir(self, run_csvdb, temp_dir):
        """watch should error if path is not a valid .csvdb directory."""
        result = run_csvdb("watch", str(temp_dir), "--target", "sqlite", check=False)
        assert result.returncode != 0
        assert "schema.sql" in result.stderr

    def test_watch_initial_build(self, csvdb_bin, sample_csvdb):
        """watch should do an initial build before waiting for changes."""
        proc, lines, stop_event = start_watch(csvdb_bin, sample_csvdb)

        assert wait_for_output(lines, "Built:"), f"Expected 'Built:' in output, got: {lines}"

        stop_watch(proc, stop_event)

        sqlite_path = sample_csvdb.parent / "sample.sqlite"
        assert sqlite_path.exists()

    def test_watch_rebuilds_on_change(self, csvdb_bin, sample_csvdb):
        """watch should rebuild when a CSV file changes."""
        proc, lines, stop_event = start_watch(csvdb_bin, sample_csvdb, debounce=200)

        assert wait_for_output(lines, "Waiting for changes"), f"Expected initial build, got: {lines}"
        time.sleep(1)  # let filesystem watcher fully initialize

        # Modify the CSV
        csv_path = sample_csvdb / "items.csv"
        csv_path.write_text(
            "id,name,price\n"
            "1,Widget,9.99\n"
            "2,Gadget,29.99\n"
            "3,Gizmo,39.99\n"
            "4,Doohickey,49.99\n"
        )

        assert wait_for_output(lines, "Change detected"), f"Expected 'Change detected', got: {lines}"

        stop_watch(proc, stop_event)

    def test_watch_duckdb_target(self, csvdb_bin, sample_csvdb):
        """watch --target duckdb should build a DuckDB file."""
        proc, lines, stop_event = start_watch(csvdb_bin, sample_csvdb, target="duckdb")

        assert wait_for_output(lines, "Built:"), f"Expected 'Built:' in output, got: {lines}"

        stop_watch(proc, stop_event)

        duckdb_path = sample_csvdb.parent / "sample.duckdb"
        assert duckdb_path.exists()

    def test_watch_survives_build_error(self, csvdb_bin, sample_csvdb):
        """watch should continue watching after a build error."""
        proc, lines, stop_event = start_watch(csvdb_bin, sample_csvdb, debounce=200)

        assert wait_for_output(lines, "Waiting for changes"), f"Expected initial build, got: {lines}"
        time.sleep(1)  # let filesystem watcher fully initialize

        # Break the schema.sql to cause a build error
        schema_path = sample_csvdb / "schema.sql"
        original_schema = schema_path.read_text()
        schema_path.write_text("INVALID SQL GARBAGE")

        assert wait_for_output(lines, "Build error", timeout=10), f"Expected 'Build error', got: {lines}"

        # Fix the schema back
        schema_path.write_text(original_schema)

        # Wait for successful rebuild (second "Built:" after the error)
        built_count_before = sum(1 for l in lines if "Built:" in l)
        deadline = time.time() + 10
        rebuilt = False
        while time.time() < deadline:
            built_count = sum(1 for l in lines if "Built:" in l)
            if built_count > built_count_before:
                rebuilt = True
                break
            time.sleep(0.3)

        stop_watch(proc, stop_event)

        assert rebuilt, f"Expected rebuild after fix, got: {lines}"
