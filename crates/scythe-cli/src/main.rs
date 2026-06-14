use clap::{Parser, Subcommand};

mod commands;

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
    /// Audit SQL files for security issues (privilege grants, dangerous
    /// functions, cartesian joins, unbounded LIKE, SECURITY DEFINER misuse).
    /// Exits with code 2 on any error-severity finding (unless --exit-zero).
    Audit {
        /// Path to config file
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
        /// Output format: human (default), sarif, json
        #[arg(long, default_value = "human")]
        format: String,
        /// Print the rule catalog (id, name, severity, category) and exit 0
        #[arg(long)]
        list_rules: bool,
        /// Print the description and CWE refs for a rule by id, then exit 0
        #[arg(long, value_name = "RULE_ID")]
        explain: Option<String>,
        /// Drop findings below this severity (off|warn|error)
        #[arg(long, value_name = "LEVEL")]
        severity: Option<String>,
        /// Exit 0 even if error-severity findings are present
        #[arg(long)]
        exit_zero: bool,
        /// Write reporter output to file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<String>,
        /// Disable inline `-- scythe-audit: ignore[...]` annotations
        #[arg(long)]
        ignore_suppressions: bool,
        /// SQL dialect for explicit-file mode (postgres|mysql|sqlite|mssql|oracle|snowflake)
        #[arg(long)]
        dialect: Option<String>,
        /// SQL files to audit (if empty, uses config schema + queries)
        files: Vec<String>,
    },
    /// Inspect a live database for operational issues (missing FK indexes,
    /// disabled RLS with policies, duplicate indexes). Connects via
    /// tokio-postgres. Exits 2 on error-severity findings unless --exit-zero.
    ///
    /// Connection URL is resolved in order: positional argument, then
    /// $DATABASE_URL, then $SCYTHE_DATABASE_URL.
    Inspect {
        /// Database URL (e.g. postgres://user:pass@host/db)
        database_url: Option<String>,
        /// Output format: human (default), sarif, json
        #[arg(long, default_value = "human")]
        format: String,
        /// Print the check catalog (id, name, severity, description) and exit 0
        #[arg(long)]
        list_checks: bool,
        /// Drop findings below this severity (off|warn|error)
        #[arg(long, value_name = "LEVEL")]
        severity: Option<String>,
        /// Exit 0 even if error-severity findings are present
        #[arg(long)]
        exit_zero: bool,
        /// Write reporter output to file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<String>,
        /// Engine to target (postgres only at Phase 0; mysql is stubbed).
        /// Default: parsed from URL scheme.
        #[arg(long)]
        dialect: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Generate { config } => commands::generate::run_generate(&config),
        Commands::Migrate { sqlc_config } => {
            commands::migrate::run_migrate(std::path::Path::new(&sqlc_config))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
        }
        Commands::Check { config } => commands::generate::run_check(&config),
        Commands::Fmt {
            config,
            check,
            diff,
            dialect,
            files,
        } => commands::fmt::run_fmt(&config, check, diff, dialect.as_deref(), &files),
        Commands::Lint {
            config,
            fix,
            dialect,
            files,
        } => commands::lint_cmd::run_lint(&config, fix, dialect.as_deref(), &files),
        Commands::Audit {
            config,
            format,
            list_rules,
            explain,
            severity,
            exit_zero,
            output,
            ignore_suppressions,
            dialect,
            files,
        } => commands::audit::run_audit(commands::audit::RunAuditOpts {
            config_path: config,
            format,
            list_rules,
            explain,
            severity,
            exit_zero,
            output,
            ignore_suppressions,
            dialect,
            files,
        }),
        Commands::Inspect {
            database_url,
            format,
            list_checks,
            severity,
            exit_zero,
            output,
            dialect,
        } => commands::inspect::run_inspect(commands::inspect::RunInspectOpts {
            database_url,
            format,
            list_checks,
            severity,
            exit_zero,
            output,
            dialect,
        }),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
