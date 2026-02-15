use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

const PRE_COMMIT_HOOK: &str = r#"#!/bin/sh
# csvdb pre-commit hook: validate staged .csvdb directories
# Installed by: csvdb hooks install

# Find all staged .csvdb directories
staged_csvdb=$(git diff --cached --name-only --diff-filter=ACM | grep '\.csvdb/' | sed 's|/[^/]*$||' | sort -u)

if [ -z "$staged_csvdb" ]; then
    exit 0
fi

failed=0
for dir in $staged_csvdb; do
    if [ -d "$dir" ]; then
        echo "csvdb: validating $dir"
        if ! csvdb validate "$dir" > /dev/null 2>&1; then
            echo "csvdb: validation failed for $dir" >&2
            failed=1
        fi
    fi
done

if [ $failed -ne 0 ]; then
    echo "csvdb: commit blocked — fix validation errors first" >&2
    exit 1
fi
"#;

const POST_MERGE_HOOK: &str = r#"#!/bin/sh
# csvdb post-merge hook: rebuild databases from changed .csvdb directories
# Installed by: csvdb hooks install

# Find .csvdb directories that changed in the merge
changed_csvdb=$(git diff --name-only HEAD@{1} HEAD | grep '\.csvdb/' | sed 's|/[^/]*$||' | sort -u)

if [ -z "$changed_csvdb" ]; then
    exit 0
fi

for dir in $changed_csvdb; do
    if [ -d "$dir" ]; then
        echo "csvdb: rebuilding SQLite from $dir"
        csvdb to-sqlite --force "$dir" 2>&1 || echo "csvdb: rebuild failed for $dir" >&2
    fi
done
"#;

/// Install git hooks for csvdb directories.
pub fn install(repo_dir: &Path) -> Result<()> {
    let hooks_dir = find_hooks_dir(repo_dir)?;

    if !hooks_dir.exists() {
        fs::create_dir_all(&hooks_dir)
            .with_context(|| format!("Failed to create hooks directory: {}", hooks_dir.display()))?;
    }

    let installed = install_hook(&hooks_dir, "pre-commit", PRE_COMMIT_HOOK)?;
    let installed2 = install_hook(&hooks_dir, "post-merge", POST_MERGE_HOOK)?;

    if installed || installed2 {
        println!("Hooks installed in {}", hooks_dir.display());
    }

    if installed {
        println!("  pre-commit: validates .csvdb directories before commit");
    }
    if installed2 {
        println!("  post-merge: rebuilds SQLite after merge");
    }

    if !installed && !installed2 {
        println!("All hooks already installed (use --force to overwrite)");
    }

    Ok(())
}

/// Force-install git hooks (overwrite existing).
pub fn install_force(repo_dir: &Path) -> Result<()> {
    let hooks_dir = find_hooks_dir(repo_dir)?;

    if !hooks_dir.exists() {
        fs::create_dir_all(&hooks_dir)
            .with_context(|| format!("Failed to create hooks directory: {}", hooks_dir.display()))?;
    }

    write_hook(&hooks_dir, "pre-commit", PRE_COMMIT_HOOK)?;
    println!("  pre-commit: validates .csvdb directories before commit");

    write_hook(&hooks_dir, "post-merge", POST_MERGE_HOOK)?;
    println!("  post-merge: rebuilds SQLite after merge");

    println!("Hooks installed in {}", hooks_dir.display());
    Ok(())
}

/// Uninstall csvdb git hooks.
pub fn uninstall(repo_dir: &Path) -> Result<()> {
    let hooks_dir = find_hooks_dir(repo_dir)?;
    let mut removed = false;

    for hook_name in &["pre-commit", "post-merge"] {
        let path = hooks_dir.join(hook_name);
        if path.exists() {
            let content = fs::read_to_string(&path).unwrap_or_default();
            if content.contains("csvdb") {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
                println!("  Removed: {}", hook_name);
                removed = true;
            } else {
                println!("  Skipped: {} (not a csvdb hook)", hook_name);
            }
        }
    }

    if !removed {
        println!("No csvdb hooks found to remove");
    }

    Ok(())
}

/// Find the git hooks directory for the repository.
fn find_hooks_dir(start: &Path) -> Result<std::path::PathBuf> {
    // Walk up from start to find .git directory
    let mut current = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir()?.join(start)
    };

    loop {
        let git_dir = current.join(".git");
        if git_dir.is_dir() {
            return Ok(git_dir.join("hooks"));
        }
        // .git could also be a file (worktrees)
        if git_dir.is_file() {
            let content = fs::read_to_string(&git_dir)?;
            if let Some(gitdir) = content.strip_prefix("gitdir: ") {
                let gitdir = gitdir.trim();
                let gitdir_path = if Path::new(gitdir).is_absolute() {
                    std::path::PathBuf::from(gitdir)
                } else {
                    current.join(gitdir)
                };
                return Ok(gitdir_path.join("hooks"));
            }
        }
        if !current.pop() {
            bail!("Not a git repository (or any parent up to mount point)");
        }
    }
}

/// Install a hook if it doesn't already exist. Returns true if installed.
fn install_hook(hooks_dir: &Path, name: &str, content: &str) -> Result<bool> {
    let path = hooks_dir.join(name);
    if path.exists() {
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if existing.contains("csvdb") {
            return Ok(false); // Already installed
        }
        // Existing hook that's not ours — don't overwrite
        eprintln!(
            "Warning: {} hook already exists and is not a csvdb hook. Skipping.",
            name
        );
        eprintln!("  Use 'csvdb hooks install --force' to overwrite.");
        return Ok(false);
    }

    write_hook(hooks_dir, name, content)?;
    Ok(true)
}

/// Write a hook file and make it executable.
fn write_hook(hooks_dir: &Path, name: &str, content: &str) -> Result<()> {
    let path = hooks_dir.join(name);
    fs::write(&path, content)
        .with_context(|| format!("Failed to write hook: {}", path.display()))?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::process::Command;

    fn init_git_repo(dir: &Path) {
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
    }

    #[test]
    fn test_install_hooks() -> Result<()> {
        let dir = tempdir()?;
        init_git_repo(dir.path());

        install(dir.path())?;

        let hooks_dir = dir.path().join(".git").join("hooks");
        assert!(hooks_dir.join("pre-commit").exists());
        assert!(hooks_dir.join("post-merge").exists());

        // Check content
        let pre_commit = fs::read_to_string(hooks_dir.join("pre-commit"))?;
        assert!(pre_commit.contains("csvdb"));
        assert!(pre_commit.contains("validate"));

        let post_merge = fs::read_to_string(hooks_dir.join("post-merge"))?;
        assert!(post_merge.contains("csvdb"));
        assert!(post_merge.contains("to-sqlite"));

        Ok(())
    }

    #[test]
    fn test_install_idempotent() -> Result<()> {
        let dir = tempdir()?;
        init_git_repo(dir.path());

        install(dir.path())?;
        install(dir.path())?; // Second install should be a no-op

        let hooks_dir = dir.path().join(".git").join("hooks");
        assert!(hooks_dir.join("pre-commit").exists());

        Ok(())
    }

    #[test]
    fn test_uninstall_hooks() -> Result<()> {
        let dir = tempdir()?;
        init_git_repo(dir.path());

        install(dir.path())?;
        uninstall(dir.path())?;

        let hooks_dir = dir.path().join(".git").join("hooks");
        assert!(!hooks_dir.join("pre-commit").exists());
        assert!(!hooks_dir.join("post-merge").exists());

        Ok(())
    }

    #[test]
    fn test_uninstall_skips_non_csvdb_hooks() -> Result<()> {
        let dir = tempdir()?;
        init_git_repo(dir.path());

        let hooks_dir = dir.path().join(".git").join("hooks");
        fs::create_dir_all(&hooks_dir)?;
        fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho 'custom hook'")?;

        uninstall(dir.path())?;

        // Should NOT remove the custom hook
        assert!(hooks_dir.join("pre-commit").exists());

        Ok(())
    }

    #[test]
    fn test_install_force_overwrites() -> Result<()> {
        let dir = tempdir()?;
        init_git_repo(dir.path());

        let hooks_dir = dir.path().join(".git").join("hooks");
        fs::create_dir_all(&hooks_dir)?;
        fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho 'custom hook'")?;

        install_force(dir.path())?;

        let content = fs::read_to_string(hooks_dir.join("pre-commit"))?;
        assert!(content.contains("csvdb"));

        Ok(())
    }

    #[test]
    fn test_not_a_git_repo() {
        let dir = tempdir().unwrap();
        // No git init
        let result = install(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Not a git repository"));
    }

    #[test]
    fn test_install_skips_existing_non_csvdb() -> Result<()> {
        let dir = tempdir()?;
        init_git_repo(dir.path());

        let hooks_dir = dir.path().join(".git").join("hooks");
        fs::create_dir_all(&hooks_dir)?;
        // Write a non-csvdb pre-commit hook
        fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho 'custom linter'")?;

        install(dir.path())?;

        // Should NOT overwrite the existing hook
        let content = fs::read_to_string(hooks_dir.join("pre-commit"))?;
        assert!(content.contains("custom linter"));
        assert!(!content.contains("csvdb"));

        // post-merge should still be installed
        assert!(hooks_dir.join("post-merge").exists());

        Ok(())
    }

    #[test]
    fn test_uninstall_no_hooks() -> Result<()> {
        let dir = tempdir()?;
        init_git_repo(dir.path());

        // No hooks installed — should not error
        uninstall(dir.path())?;
        Ok(())
    }

    #[test]
    fn test_git_worktree() -> Result<()> {
        let dir = tempdir()?;
        let repo = dir.path().join("repo");
        fs::create_dir(&repo)?;
        init_git_repo(&repo);

        // Simulate a worktree by creating a .git file pointing to a gitdir
        let wt = dir.path().join("worktree");
        fs::create_dir(&wt)?;
        let git_dir = dir.path().join("repo").join(".git").join("worktrees").join("wt");
        fs::create_dir_all(&git_dir)?;
        fs::write(wt.join(".git"), format!("gitdir: {}", git_dir.display()))?;

        install(&wt)?;

        assert!(git_dir.join("hooks").join("pre-commit").exists());
        assert!(git_dir.join("hooks").join("post-merge").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_hooks_executable() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir()?;
        init_git_repo(dir.path());

        install(dir.path())?;

        let hooks_dir = dir.path().join(".git").join("hooks");
        let perms = fs::metadata(hooks_dir.join("pre-commit"))?.permissions();
        assert!(perms.mode() & 0o111 != 0, "pre-commit should be executable");

        let perms = fs::metadata(hooks_dir.join("post-merge"))?.permissions();
        assert!(perms.mode() & 0o111 != 0, "post-merge should be executable");

        Ok(())
    }
}
