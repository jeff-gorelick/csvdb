"""Functional tests for the hooks command."""

import subprocess


class TestHooksInstall:
    """Tests for csvdb hooks install."""

    def _init_git_repo(self, path):
        """Initialize a git repo at the given path."""
        subprocess.run(
            ["git", "init", "--initial-branch=main"],
            cwd=path,
            capture_output=True,
        )

    def test_hooks_install(self, run_csvdb, temp_dir):
        """hooks install should create pre-commit and post-merge hooks."""
        self._init_git_repo(temp_dir)

        run_csvdb("hooks", "install", cwd=temp_dir)

        hooks_dir = temp_dir / ".git" / "hooks"
        assert (hooks_dir / "pre-commit").exists()
        assert (hooks_dir / "post-merge").exists()

        # Check content
        pre_commit = (hooks_dir / "pre-commit").read_text()
        assert "csvdb" in pre_commit
        assert "validate" in pre_commit

        post_merge = (hooks_dir / "post-merge").read_text()
        assert "csvdb" in post_merge
        assert "to-sqlite" in post_merge

    def test_hooks_install_idempotent(self, run_csvdb, temp_dir):
        """Running install twice should succeed."""
        self._init_git_repo(temp_dir)

        run_csvdb("hooks", "install", cwd=temp_dir)
        result = run_csvdb("hooks", "install", cwd=temp_dir)
        assert result.returncode == 0
        assert "already installed" in result.stdout.lower()

    def test_hooks_install_force(self, run_csvdb, temp_dir):
        """hooks install --force should overwrite existing hooks."""
        self._init_git_repo(temp_dir)

        hooks_dir = temp_dir / ".git" / "hooks"
        hooks_dir.mkdir(parents=True, exist_ok=True)
        (hooks_dir / "pre-commit").write_text("#!/bin/sh\necho custom")

        run_csvdb("hooks", "install", "--force", cwd=temp_dir)

        content = (hooks_dir / "pre-commit").read_text()
        assert "csvdb" in content

    def test_hooks_uninstall(self, run_csvdb, temp_dir):
        """hooks uninstall should remove csvdb hooks."""
        self._init_git_repo(temp_dir)

        run_csvdb("hooks", "install", cwd=temp_dir)
        run_csvdb("hooks", "uninstall", cwd=temp_dir)

        hooks_dir = temp_dir / ".git" / "hooks"
        assert not (hooks_dir / "pre-commit").exists()
        assert not (hooks_dir / "post-merge").exists()

    def test_hooks_uninstall_preserves_custom(self, run_csvdb, temp_dir):
        """hooks uninstall should not remove non-csvdb hooks."""
        self._init_git_repo(temp_dir)

        hooks_dir = temp_dir / ".git" / "hooks"
        hooks_dir.mkdir(parents=True, exist_ok=True)
        (hooks_dir / "pre-commit").write_text("#!/bin/sh\necho custom hook")

        run_csvdb("hooks", "uninstall", cwd=temp_dir)

        # Custom hook should still exist
        assert (hooks_dir / "pre-commit").exists()
        content = (hooks_dir / "pre-commit").read_text()
        assert "custom hook" in content

    def test_hooks_not_git_repo(self, run_csvdb, temp_dir):
        """hooks install should fail outside a git repo."""
        result = run_csvdb("hooks", "install", cwd=temp_dir, check=False)
        assert result.returncode != 0
        assert "git" in result.stderr.lower()

    def test_hooks_help(self, run_csvdb):
        """hooks --help should show subcommands."""
        result = run_csvdb("hooks", "--help", check=False)
        assert result.returncode == 0
        assert "install" in result.stdout.lower()
        assert "uninstall" in result.stdout.lower()
