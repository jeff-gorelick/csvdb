use anyhow::{bail, Result};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::{NullMode, OrderMode, TableFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum WatchTarget {
    Sqlite,
    Duckdb,
    Parquetdb,
}

pub fn watch(
    path: &Path,
    target: WatchTarget,
    order: OrderMode,
    null_mode: NullMode,
    order_by: Option<&str>,
    debounce_ms: u64,
    filter: &TableFilter,
) -> Result<()> {
    // Validate path is a csvdb directory
    if !path.join("schema.sql").exists() {
        bail!(
            "Not a valid .csvdb directory (missing schema.sql): {}",
            path.display()
        );
    }

    let debounce = Duration::from_millis(debounce_ms);

    // Do an initial build
    eprintln!("Watching: {}", path.display());
    match rebuild(path, target, order, null_mode, order_by, filter) {
        Ok(output) => eprintln!("Built: {}", output.display()),
        Err(e) => eprintln!("Build error: {e:#}"),
    }

    // Set up file watcher
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(path, RecursiveMode::NonRecursive)?;

    eprintln!("Waiting for changes...");

    loop {
        // Block until we get an event
        match rx.recv() {
            Ok(Ok(event)) => {
                if !is_relevant_event(&event) {
                    continue;
                }
            }
            Ok(Err(e)) => {
                eprintln!("Watch error: {e}");
                continue;
            }
            Err(_) => {
                // Channel closed, watcher dropped
                break;
            }
        }

        // Debounce: drain additional events within the window
        let deadline = Instant::now() + debounce;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(_) => {} // drain
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        // Rebuild
        eprintln!("Change detected, rebuilding...");
        match rebuild(path, target, order, null_mode, order_by, filter) {
            Ok(output) => eprintln!("Built: {}", output.display()),
            Err(e) => eprintln!("Build error: {e:#}"),
        }
        eprintln!("Waiting for changes...");
    }

    Ok(())
}

fn is_relevant_event(event: &Event) -> bool {
    // Only care about create/modify/remove events
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {}
        _ => return false,
    }

    // Filter to relevant file extensions
    event.paths.iter().any(|p| {
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            name.ends_with(".csv")
                || name.ends_with(".csv.gz")
                || name.ends_with(".sql")
                || name.ends_with(".toml")
        } else {
            false
        }
    })
}

fn rebuild(
    path: &Path,
    target: WatchTarget,
    order: OrderMode,
    null_mode: NullMode,
    order_by: Option<&str>,
    filter: &TableFilter,
) -> Result<PathBuf> {
    match target {
        WatchTarget::Sqlite => {
            crate::commands::to_sqlite::to_sqlite(path, None, true, filter)
        }
        WatchTarget::Duckdb => {
            crate::commands::to_duckdb::to_duckdb(path, None, true, filter)
        }
        WatchTarget::Parquetdb => {
            crate::commands::to_parquetdb::to_parquetdb(
                path,
                order,
                null_mode,
                order_by,
                None,
                true,
                filter,
            )
        }
    }
}
