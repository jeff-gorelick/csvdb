use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use csvdb::commands::{checksum, diff, init, to_csv, to_duckdb, to_parquetdb, to_sqlite, validate};
use csvdb::{OrderMode, NullMode, TableFilter};

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
        /// Directory containing CSV files
        source: PathBuf,

        /// Disable automatic primary key detection
        #[arg(long)]
        no_pk_detection: bool,
    },

    /// Convert SQLite/DuckDB to .csvdb directory (schema.sql + CSVs)
    ToCsvdb {
        /// Path to input file (.sqlite, .duckdb)
        input: PathBuf,

        /// Output directory (default: <input>.csvdb)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// How to order rows in CSV output
        #[arg(long, value_enum, default_value_t = OrderMode::Pk)]
        order: OrderMode,

        /// How to represent NULL values in CSV.
        /// 'marker' (default): use \N - lossless, preserves empty strings.
        /// 'empty': use empty string - LOSSY, cannot distinguish NULL from "".
        /// 'literal': use string "NULL" - LOSSY, cannot distinguish NULL from "NULL".
        #[arg(long, value_enum, default_value_t = NullMode::Marker)]
        null_mode: NullMode,

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

        /// How to order rows in output
        #[arg(long, value_enum, default_value_t = OrderMode::Pk)]
        order: OrderMode,

        /// How to represent NULL values (for csvdb output compatibility)
        #[arg(long, value_enum, default_value_t = NullMode::Marker)]
        null_mode: NullMode,

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init { source, no_pk_detection } => run_init(&source, no_pk_detection),
        Commands::ToCsvdb { input, output, order, null_mode, pipe, force, tables, exclude } => {
            let filter = TableFilter::new(tables, exclude);
            run_to_csvdb(&input, output.as_deref(), order, null_mode, pipe, force, &filter)
        }
        Commands::ToSqlite { input, force, tables, exclude } => {
            let filter = TableFilter::new(tables, exclude);
            run_to_sqlite(&input, force, &filter)
        }
        Commands::ToDuckdb { input, force, tables, exclude } => {
            let filter = TableFilter::new(tables, exclude);
            run_to_duckdb(&input, force, &filter)
        }
        Commands::ToParquetdb { input, output, order, null_mode, pipe, force, tables, exclude } => {
            let filter = TableFilter::new(tables, exclude);
            run_to_parquetdb(&input, output.as_deref(), order, null_mode, pipe, force, &filter)
        }
        Commands::Validate { path } => run_validate(&path),
        Commands::Diff { left, right, summary } => run_diff(&left, &right, summary),
        Commands::Checksum { path, tables, exclude } => {
            let filter = TableFilter::new(tables, exclude);
            run_checksum(&path, &filter)
        }
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {:#}", e);
            ExitCode::FAILURE
        }
    }
}

fn run_init(source: &PathBuf, no_pk_detection: bool) -> Result<ExitCode> {
    let config = init::InferConfig {
        detect_pk: !no_pk_detection,
        ..Default::default()
    };

    let result = init::init_csvdb(source, &config)?;

    // Print warnings
    for warning in &result.warnings {
        eprintln!("Warning: {}", warning);
    }

    // Print summary
    println!("Created: {}", result.output_dir.display());
    println!();
    for table in &result.tables {
        let pk_info = match &table.suggested_pk {
            Some(pk) => format!("PK: {}", pk),
            None => "no PK".to_string(),
        };
        println!(
            "  {} ({} rows, {} columns, {})",
            table.name,
            table.row_count,
            table.columns.len(),
            pk_info
        );
    }

    Ok(ExitCode::SUCCESS)
}

fn run_to_csvdb(input: &PathBuf, output: Option<&Path>, order: OrderMode, null_mode: NullMode, pipe: bool, force: bool, filter: &TableFilter) -> Result<ExitCode> {
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
        let stem = input.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("csvdb");
        output_path = Some(std::env::temp_dir().join(format!("{}.csvdb", stem)));
        output_path.as_deref()
    } else {
        output
    };

    let csvdir = to_csv::to_csv(input, order, null_mode, effective_output, pipe || force, filter)?;

    if pipe {
        // Quiet mode: just output the path for piping
        // Use forward slashes for cross-platform compatibility in pipes
        let path_str = csvdir.to_string_lossy().replace('\\', "/");
        println!("{}", path_str);
    } else {
        println!("Created: {}", csvdir.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn run_to_sqlite(csvdir: &PathBuf, force: bool, filter: &TableFilter) -> Result<ExitCode> {
    let db_path = to_sqlite::to_sqlite(csvdir, force, filter)?;
    println!("Created: {}", db_path.display());
    Ok(ExitCode::SUCCESS)
}

fn run_to_duckdb(csvdir: &PathBuf, force: bool, filter: &TableFilter) -> Result<ExitCode> {
    let db_path = to_duckdb::to_duckdb(csvdir, force, filter)?;
    println!("Created: {}", db_path.display());
    Ok(ExitCode::SUCCESS)
}

fn run_to_parquetdb(input: &PathBuf, output: Option<&Path>, order: OrderMode, null_mode: NullMode, pipe: bool, force: bool, filter: &TableFilter) -> Result<ExitCode> {
    let output_path: Option<PathBuf>;
    let effective_output = if pipe && output.is_none() {
        let stem = input.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("parquetdb");
        output_path = Some(std::env::temp_dir().join(format!("{}.parquetdb", stem)));
        output_path.as_deref()
    } else {
        output
    };

    let parquetdb = to_parquetdb::to_parquetdb(input, order, null_mode, effective_output, pipe || force, filter)?;

    if pipe {
        let path_str = parquetdb.to_string_lossy().replace('\\', "/");
        println!("{}", path_str);
    } else {
        println!("Created: {}", parquetdb.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn run_validate(path: &PathBuf) -> Result<ExitCode> {
    let result = validate::validate(path)?;
    if !result.errors.is_empty() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_diff(left: &PathBuf, right: &PathBuf, summary: bool) -> Result<ExitCode> {
    let has_differences = diff::diff(left, right, summary)?;
    if has_differences {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_checksum(path: &PathBuf, filter: &TableFilter) -> Result<ExitCode> {
    let hash = checksum::checksum(path, filter)?;
    println!("{}", hash);
    Ok(ExitCode::SUCCESS)
}
