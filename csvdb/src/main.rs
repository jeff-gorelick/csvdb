#![allow(clippy::too_many_arguments)]

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use csvdb::commands::{
    checksum, diff, hooks, init, sql, to_csv, to_duckdb, to_parquetdb, to_sqlite, validate, watch,
};
use csvdb::{CsvdbConfig, InputFormat, NullMode, OrderMode, TableFilter};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SqlFormat {
    Csv,
    Table,
}

#[derive(Parser)]
#[command(name = "csvdb")]
#[command(about = "Convert between SQLite/DuckDB databases and CSV directories")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a .csvdb directory from raw CSV files (infers schema)
    Init {
        /// CSV file or directory containing CSV files
        source: PathBuf,

        /// Output directory (default: <source>.csvdb)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Overwrite existing output directory
        #[arg(long)]
        force: bool,

        /// Disable automatic primary key detection
        #[arg(long)]
        no_pk_detection: bool,

        /// Disable automatic foreign key detection
        #[arg(long)]
        no_fk_detection: bool,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Convert SQLite/DuckDB to .csvdb directory (schema.sql + CSVs)
    ToCsvdb {
        /// Path to input file (.sqlite, .duckdb)
        input: PathBuf,

        /// Output directory (default: <input>.csvdb)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// How to order rows in CSV output (default: pk).
        /// When re-exporting from .csvdb, reads from csvdb.toml if not specified.
        #[arg(long, value_enum)]
        order: Option<OrderMode>,

        /// How to represent NULL values in CSV (default: marker).
        /// 'marker': use \N - lossless. 'empty': use empty string - LOSSY. 'literal': use "NULL" - LOSSY.
        /// When re-exporting from .csvdb, reads from csvdb.toml if not specified.
        #[arg(long, value_enum)]
        null_mode: Option<NullMode>,

        /// Sort string PKs naturally (e.g. "item2" before "item10"). Implies --order=pk.
        #[arg(long)]
        natural_sort: bool,

        /// Custom ORDER BY clause (e.g. "created_at DESC"). Conflicts with --order.
        #[arg(long, conflicts_with = "order")]
        order_by: Option<String>,

        /// Compress CSV files with gzip (.csv.gz)
        #[arg(long)]
        compress: bool,

        /// Only re-export tables whose data has changed (stores checksums in csvdb.toml)
        #[arg(long)]
        incremental: bool,

        /// Write to temp directory and output only the path (for piping)
        #[arg(long)]
        pipe: bool,

        /// Overwrite existing output directory
        #[arg(long)]
        force: bool,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Convert any format to SQLite database
    ToSqlite {
        /// Path to input file or directory
        input: PathBuf,

        /// Output file (default: <input>.sqlite)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Overwrite existing output file
        #[arg(long)]
        force: bool,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Convert any format to DuckDB database
    ToDuckdb {
        /// Path to input file or directory
        input: PathBuf,

        /// Output file (default: <input>.duckdb)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Overwrite existing output file
        #[arg(long)]
        force: bool,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Convert any format to .parquetdb directory
    ToParquetdb {
        /// Path to input file or directory
        input: PathBuf,

        /// Output directory (default: <input>.parquetdb)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// How to order rows in output (default: pk).
        /// When input is .csvdb/.parquetdb, reads from csvdb.toml if not specified.
        #[arg(long, value_enum)]
        order: Option<OrderMode>,

        /// How to represent NULL values (default: marker).
        /// When input is .csvdb/.parquetdb, reads from csvdb.toml if not specified.
        #[arg(long, value_enum)]
        null_mode: Option<NullMode>,

        /// Sort string PKs naturally (e.g. "item2" before "item10"). Implies --order=pk.
        #[arg(long)]
        natural_sort: bool,

        /// Custom ORDER BY clause (e.g. "created_at DESC"). Conflicts with --order.
        #[arg(long, conflicts_with = "order")]
        order_by: Option<String>,

        /// Write to temp directory and output only the path (for piping)
        #[arg(long)]
        pipe: bool,

        /// Overwrite existing output directory
        #[arg(long)]
        force: bool,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Validate a .csvdb directory for structural integrity
    Validate {
        /// Path to .csvdb directory
        path: PathBuf,
    },

    /// Compare two databases or csvdb directories
    Diff {
        /// Left (base) path
        left: PathBuf,
        /// Right (changed) path
        right: PathBuf,
        /// Only show summary counts, not individual rows
        #[arg(long)]
        summary: bool,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Compute checksum of database or csvdb directory
    Checksum {
        /// Path to database (.sqlite, .duckdb) or .csvdb directory
        path: PathBuf,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Run a read-only SQL query against any supported format
    Sql {
        /// SQL query to execute (SELECT only)
        query: String,

        /// Path to database or directory
        path: PathBuf,

        /// Output format (default: table for TTY, csv for pipe)
        #[arg(long, value_enum)]
        format: Option<SqlFormat>,
    },

    /// Watch a .csvdb directory and rebuild target on changes
    Watch {
        /// Path to .csvdb directory
        path: PathBuf,

        /// Target format to build
        #[arg(long, value_enum)]
        target: watch::WatchTarget,

        /// How to order rows in output (default: pk). Only used for parquetdb target.
        #[arg(long, value_enum)]
        order: Option<OrderMode>,

        /// How to represent NULL values (default: marker). Only used for parquetdb target.
        #[arg(long, value_enum)]
        null_mode: Option<NullMode>,

        /// Custom ORDER BY clause. Only used for parquetdb target.
        #[arg(long, conflicts_with = "order")]
        order_by: Option<String>,

        /// Debounce interval in milliseconds (default: 500)
        #[arg(long, default_value_t = 500)]
        debounce: u64,

        /// Only include these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "exclude")]
        tables: Vec<String>,

        /// Exclude these tables (comma-separated)
        #[arg(long, value_delimiter = ',', conflicts_with = "tables")]
        exclude: Vec<String>,
    },

    /// Install/uninstall git hooks for .csvdb directories
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },
}

#[derive(Subcommand)]
enum HooksAction {
    /// Install pre-commit and post-merge hooks
    Install {
        /// Overwrite existing hooks
        #[arg(long)]
        force: bool,
    },
    /// Remove csvdb git hooks
    Uninstall,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init {
            source,
            output,
            force,
            no_pk_detection,
            no_fk_detection,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            run_init(
                &source,
                output.as_deref(),
                force,
                &filter,
                no_pk_detection,
                no_fk_detection,
            )
        }
        Commands::ToCsvdb {
            input,
            output,
            order,
            null_mode,
            natural_sort,
            order_by,
            compress,
            incremental,
            pipe,
            force,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            let (order, null_mode, order_by) = resolve_config(&input, order, null_mode, order_by);
            if incremental {
                run_to_csvdb_incremental(
                    &input,
                    output.as_deref(),
                    order,
                    null_mode,
                    natural_sort,
                    order_by.as_deref(),
                    compress,
                    pipe,
                    &filter,
                )
            } else {
                run_to_csvdb(
                    &input,
                    output.as_deref(),
                    order,
                    null_mode,
                    natural_sort,
                    order_by.as_deref(),
                    compress,
                    pipe,
                    force,
                    &filter,
                )
            }
        }
        Commands::ToSqlite {
            input,
            output,
            force,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            run_to_sqlite(&input, output.as_deref(), force, &filter)
        }
        Commands::ToDuckdb {
            input,
            output,
            force,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            run_to_duckdb(&input, output.as_deref(), force, &filter)
        }
        Commands::ToParquetdb {
            input,
            output,
            order,
            null_mode,
            natural_sort: _,
            order_by,
            pipe,
            force,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            let (order, null_mode, order_by) = resolve_config(&input, order, null_mode, order_by);
            run_to_parquetdb(
                &input,
                output.as_deref(),
                order,
                null_mode,
                order_by.as_deref(),
                pipe,
                force,
                &filter,
            )
        }
        Commands::Validate { path } => run_validate(&path),
        Commands::Diff {
            left,
            right,
            summary,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            run_diff(&left, &right, summary, &filter)
        }
        Commands::Checksum {
            path,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            run_checksum(&path, &filter)
        }
        Commands::Sql {
            query,
            path,
            format,
        } => {
            let format = format.map(|f| match f {
                SqlFormat::Csv => sql::OutputFormat::Csv,
                SqlFormat::Table => sql::OutputFormat::Table,
            });
            run_sql(&path, &query, format)
        }
        Commands::Watch {
            path,
            target,
            order,
            null_mode,
            order_by,
            debounce,
            tables,
            exclude,
        } => {
            let filter = TableFilter::new(tables, exclude);
            let (order, null_mode, order_by) = resolve_config(&path, order, null_mode, order_by);
            run_watch(
                &path,
                target,
                order,
                null_mode,
                order_by.as_deref(),
                debounce,
                &filter,
            )
        }
        Commands::Hooks { action } => run_hooks(action),
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Resolve order, null_mode, and order_by from CLI flags, falling back to config file then defaults.
fn resolve_config(
    input: &Path,
    cli_order: Option<OrderMode>,
    cli_null_mode: Option<NullMode>,
    cli_order_by: Option<String>,
) -> (OrderMode, NullMode, Option<String>) {
    // Try to load config from input directory (if it's a csvdb/parquetdb directory)
    let config = if let Ok(format) = InputFormat::from_path(input) {
        if format.is_directory() {
            CsvdbConfig::load(input).ok()
        } else {
            None
        }
    } else {
        None
    };

    // order_by takes precedence: CLI flag -> config -> None
    let order_by = cli_order_by.or_else(|| config.as_ref().and_then(|c| c.order_by.clone()));

    let order = if order_by.is_some() {
        // When order_by is set, order mode is irrelevant but we still resolve it
        cli_order.unwrap_or(OrderMode::Pk)
    } else {
        cli_order
            .or_else(|| config.as_ref().and_then(|c| c.order_mode()))
            .unwrap_or(OrderMode::Pk)
    };

    let null_mode = cli_null_mode
        .or_else(|| config.as_ref().and_then(|c| c.null_mode()))
        .unwrap_or(NullMode::Marker);

    (order, null_mode, order_by)
}

fn run_init(
    source: &Path,
    output: Option<&Path>,
    force: bool,
    filter: &TableFilter,
    no_pk_detection: bool,
    no_fk_detection: bool,
) -> Result<ExitCode> {
    let config = init::InferConfig {
        detect_pk: !no_pk_detection,
        detect_fk: !no_fk_detection,
        ..Default::default()
    };

    let result = init::init_csvdb(source, output, force, filter, &config)?;

    // Print warnings
    for warning in &result.warnings {
        eprintln!("Warning: {warning}");
    }

    // Print summary
    println!("Created: {}", result.output_dir.display());
    println!();
    for table in &result.tables {
        let pk_info = match &table.suggested_pk {
            Some(pk) => format!("PK: {pk}"),
            None => "no PK".to_string(),
        };
        let fk_count = table.suggested_fks.len();
        let fk_info = if fk_count > 0 {
            format!(", {} FK{}", fk_count, if fk_count == 1 { "" } else { "s" })
        } else {
            String::new()
        };
        println!(
            "  {} ({} rows, {} columns, {}{})",
            table.name,
            table.row_count,
            table.columns.len(),
            pk_info,
            fk_info
        );
    }

    Ok(ExitCode::SUCCESS)
}

fn run_to_csvdb(
    input: &Path,
    output: Option<&Path>,
    order: OrderMode,
    null_mode: NullMode,
    natural_sort: bool,
    order_by: Option<&str>,
    compress: bool,
    pipe: bool,
    force: bool,
    filter: &TableFilter,
) -> Result<ExitCode> {
    // Warn about lossy null modes unless in pipe mode (quiet)
    if null_mode.is_lossy() && !pipe {
        match null_mode {
            NullMode::Empty => {
                eprintln!("Warning: --null-mode=empty is LOSSY and cannot distinguish NULL from empty string.");
                eprintln!("         Use --null-mode=marker (default) for lossless roundtrips.");
            }
            NullMode::Literal => {
                eprintln!("Warning: --null-mode=literal is LOSSY and cannot distinguish NULL from the string \"NULL\".");
                eprintln!("         Use --null-mode=marker (default) for lossless roundtrips.");
            }
            _ => {}
        }
    }

    let output_path: Option<PathBuf>;
    let effective_output = if pipe && output.is_none() {
        // Create temp directory path based on input filename
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("csvdb");
        output_path = Some(std::env::temp_dir().join(format!("{stem}.csvdb")));
        output_path.as_deref()
    } else {
        output
    };

    let csvdir = to_csv::to_csv(
        input,
        order,
        null_mode,
        natural_sort,
        order_by,
        compress,
        effective_output,
        pipe || force,
        filter,
    )?;

    if pipe {
        // Quiet mode: just output the path for piping
        // Use forward slashes for cross-platform compatibility in pipes
        let path_str = csvdir.to_string_lossy().replace('\\', "/");
        println!("{path_str}");
    } else {
        println!("Created: {}", csvdir.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn run_to_csvdb_incremental(
    input: &Path,
    output: Option<&Path>,
    order: OrderMode,
    null_mode: NullMode,
    natural_sort: bool,
    order_by: Option<&str>,
    compress: bool,
    pipe: bool,
    filter: &TableFilter,
) -> Result<ExitCode> {
    let output_path: Option<PathBuf>;
    let effective_output = if pipe && output.is_none() {
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("csvdb");
        output_path = Some(std::env::temp_dir().join(format!("{stem}.csvdb")));
        output_path.as_deref()
    } else {
        output
    };

    let (csvdir, summary) = to_csv::to_csv_incremental(
        input,
        order,
        null_mode,
        natural_sort,
        order_by,
        compress,
        effective_output,
        filter,
    )?;

    if pipe {
        let path_str = csvdir.to_string_lossy().replace('\\', "/");
        println!("{path_str}");
    } else {
        // Print incremental summary
        for name in &summary.unchanged {
            eprintln!("  {name}: unchanged");
        }
        for name in &summary.updated {
            eprintln!("  {name}: updated");
        }
        for name in &summary.added {
            eprintln!("  {name}: new");
        }
        for name in &summary.removed {
            eprintln!("  {name}: removed");
        }
        println!("Created: {}", csvdir.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn run_to_sqlite(
    csvdir: &Path,
    output: Option<&Path>,
    force: bool,
    filter: &TableFilter,
) -> Result<ExitCode> {
    let db_path = to_sqlite::to_sqlite(csvdir, output, force, filter)?;
    println!("Created: {}", db_path.display());
    Ok(ExitCode::SUCCESS)
}

fn run_to_duckdb(
    csvdir: &Path,
    output: Option<&Path>,
    force: bool,
    filter: &TableFilter,
) -> Result<ExitCode> {
    let db_path = to_duckdb::to_duckdb(csvdir, output, force, filter)?;
    println!("Created: {}", db_path.display());
    Ok(ExitCode::SUCCESS)
}

fn run_to_parquetdb(
    input: &Path,
    output: Option<&Path>,
    order: OrderMode,
    null_mode: NullMode,
    order_by: Option<&str>,
    pipe: bool,
    force: bool,
    filter: &TableFilter,
) -> Result<ExitCode> {
    let output_path: Option<PathBuf>;
    let effective_output = if pipe && output.is_none() {
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("parquetdb");
        output_path = Some(std::env::temp_dir().join(format!("{stem}.parquetdb")));
        output_path.as_deref()
    } else {
        output
    };

    let parquetdb = to_parquetdb::to_parquetdb(
        input,
        order,
        null_mode,
        order_by,
        effective_output,
        pipe || force,
        filter,
    )?;

    if pipe {
        let path_str = parquetdb.to_string_lossy().replace('\\', "/");
        println!("{path_str}");
    } else {
        println!("Created: {}", parquetdb.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn run_validate(path: &Path) -> Result<ExitCode> {
    let result = validate::validate(path)?;
    if !result.errors.is_empty() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_diff(left: &Path, right: &Path, summary: bool, filter: &TableFilter) -> Result<ExitCode> {
    let has_differences = diff::diff(left, right, summary, filter)?;
    if has_differences {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_checksum(path: &Path, filter: &TableFilter) -> Result<ExitCode> {
    let hash = checksum::checksum(path, filter)?;
    println!("{hash}");
    Ok(ExitCode::SUCCESS)
}

fn run_sql(path: &Path, query: &str, format: Option<sql::OutputFormat>) -> Result<ExitCode> {
    sql::sql(path, query, format)?;
    Ok(ExitCode::SUCCESS)
}

fn run_watch(
    path: &Path,
    target: watch::WatchTarget,
    order: OrderMode,
    null_mode: NullMode,
    order_by: Option<&str>,
    debounce: u64,
    filter: &TableFilter,
) -> Result<ExitCode> {
    watch::watch(path, target, order, null_mode, order_by, debounce, filter)?;
    Ok(ExitCode::SUCCESS)
}

fn run_hooks(action: HooksAction) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    match action {
        HooksAction::Install { force } => {
            if force {
                hooks::install_force(&cwd)?;
            } else {
                hooks::install(&cwd)?;
            }
        }
        HooksAction::Uninstall => {
            hooks::uninstall(&cwd)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
