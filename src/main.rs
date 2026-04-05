use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "scythe", version, about = "SQL-to-code generator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate code from SQL schema and queries
    Generate {
        /// Path to config file
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
    },
    /// Migrate from sqlc to scythe format
    Migrate {
        /// Path to sqlc config file
        #[arg(default_value = "sqlc.yaml")]
        sqlc_config: String,
    },
    /// Validate SQL without generating code
    Check {
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
    },
    /// Format SQL files using sqruff
    Fmt {
        /// Path to config file
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
        /// Check formatting without modifying files
        #[arg(long)]
        check: bool,
        /// Show diff of formatting changes
        #[arg(long)]
        diff: bool,
        /// SQL dialect (e.g. ansi, postgres, mysql, bigquery)
        #[arg(long)]
        dialect: Option<String>,
        /// SQL files to format (if empty, uses config)
        files: Vec<String>,
    },
    /// Lint SQL files (scythe rules + sqruff rules)
    Lint {
        /// Path to config file
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
        /// Auto-fix violations where possible
        #[arg(long)]
        fix: bool,
        /// SQL dialect (e.g. ansi, postgres, mysql, bigquery)
        #[arg(long)]
        dialect: Option<String>,
        /// SQL files to lint (if empty, uses config)
        files: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Generate { config } => scythe::commands::generate::run_generate(&config),
        Commands::Migrate { sqlc_config } => {
            scythe::commands::migrate::run_migrate(std::path::Path::new(&sqlc_config))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
        }
        Commands::Check { config } => scythe::commands::generate::run_check(&config),
        Commands::Fmt {
            config,
            check,
            diff,
            dialect,
            files,
        } => scythe::commands::fmt::run_fmt(&config, check, diff, dialect.as_deref(), &files),
        Commands::Lint {
            config,
            fix,
            dialect,
            files,
        } => scythe::commands::lint_cmd::run_lint(&config, fix, dialect.as_deref(), &files),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
