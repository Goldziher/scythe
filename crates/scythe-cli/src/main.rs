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
        /// Allow a `[[sql.gen]]` `output` directory to resolve outside the
        /// project root (via `../` traversal or an absolute path). Off by
        /// default: without it, such a path is rejected before anything is
        /// written.
        #[arg(long)]
        allow_output_escape: bool,
        /// Validate each target's generated output with the real
        /// compiler/linter for its language (`poly`, `tsc`, `javac`,
        /// `kotlinc`, `gofmt`, `ruby`, ...), reporting per target whether it
        /// was validated, skipped (no validator for that language, or the
        /// tool it needs is not installed), or failed. Exits 2 -- not 1 --
        /// if any target fails.
        ///
        /// Off by default: it shells out to external toolchains that may not
        /// be installed, so making it the default would break `generate` for
        /// anyone missing one.
        #[arg(long)]
        validate_output: bool,
    },
    /// Migrate from sqlc to scythe format
    Migrate {
        /// Path to sqlc config file
        #[arg(default_value = "sqlc.yaml")]
        sqlc_config: String,
    },
    /// Validate SQL without generating code. Exits 0 clean, 2 on
    /// error-severity findings (unless --exit-zero) -- including an
    /// unparseable query (SC-PARSE01/02) or a `[sql.gen]` target that
    /// `scythe generate` would refuse to construct (SC-PRV09), both
    /// reported without discarding findings collected from the rest of the
    /// config -- 1 on operational failure (unreadable config, or other I/O
    /// error).
    Check {
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
        /// Verify inferred query types and detect schema drift against a live
        /// database.
        ///
        /// Each query is prepared server-side (never executed) and the reported
        /// result columns and parameters are diffed against static inference
        /// (SC-VER*). The committed schema is then compared against the live
        /// catalog for missing tables and columns, type mismatches, nullability
        /// mismatches and enum drift (SC-DRF*). PostgreSQL only. Without this
        /// flag `check` needs no database.
        ///
        /// Preparing a statement cannot report nullability, so SC-DRF06 —
        /// reading the catalog directly — is the only check that can tell you a
        /// `NOT NULL` in your DDL is not true in the database.
        ///
        /// Opt-in by design: the URL is never picked up from the environment,
        /// so `scythe check` cannot start requiring a database just because
        /// `DATABASE_URL` happens to be set.
        #[arg(long)]
        database_url: Option<String>,
        /// Output format: human, sarif, or json
        #[arg(long, default_value = "human")]
        format: String,
        /// Write findings to a file instead of stdout
        #[arg(short, long)]
        output: Option<String>,
        /// Exit 0 even if error-severity findings are present
        #[arg(long)]
        exit_zero: bool,
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
    /// Lint SQL files (scythe rules + sqruff rules). Exits 2 on any
    /// error-severity finding (unless --exit-zero), 1 on operational
    /// failure (unreadable config, unparseable SQL, invalid [lint.sqruff]).
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
        /// Database URL for the auto-run `inspect` pass (live operational
        /// checks: missing FK indexes, disabled RLS, duplicate indexes).
        ///
        /// Opt-in by design, like `check`'s `--database-url`: without this
        /// flag (or a `[inspect].database_url` in scythe.toml), `lint` never
        /// connects to a database, even if `$DATABASE_URL` happens to be set.
        #[arg(long)]
        database_url: Option<String>,
        /// Output format: human (default), sarif, json
        #[arg(long, default_value = "human")]
        format: String,
        /// Write reporter output to file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<String>,
        /// Exit 0 even if error-severity findings are present
        #[arg(long)]
        exit_zero: bool,
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
    /// tokio-postgres or mysql_async, chosen by the resolved dialect.
    /// Exits 2 on error-severity findings unless --exit-zero.
    ///
    /// Connection URL is resolved in order: positional argument, then
    /// $DATABASE_URL, then $SCYTHE_DATABASE_URL, then [inspect].database_url
    /// in scythe.toml.
    Inspect {
        /// Database URL (e.g. postgres://user:pass@host/db)
        database_url: Option<String>,
        /// Output format: human (default), sarif, json
        #[arg(long, default_value = "human")]
        format: String,
        /// Print the check catalog (id, name, severity, description) and exit 0
        #[arg(long)]
        list_checks: bool,
        /// Print full rationale and remediation for a single check ID, then exit 0
        #[arg(long, value_name = "CHECK_ID")]
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
        /// Engine to target: postgres|postgresql or mysql|mariadb.
        /// Default: parsed from URL scheme.
        #[arg(long)]
        dialect: Option<String>,
        /// Path to config file (default: scythe.toml)
        #[arg(short, long, default_value = "scythe.toml")]
        config: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Generate {
            config,
            allow_output_escape,
            validate_output,
        } => commands::generate::run_generate(&config, allow_output_escape, validate_output),
        Commands::Migrate { sqlc_config } => commands::migrate::run_migrate(std::path::Path::new(&sqlc_config))
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Commands::Check {
            config,
            database_url,
            format,
            output,
            exit_zero,
        } => commands::generate::run_check(commands::generate::RunCheckOpts {
            config_path: config,
            database_url,
            format,
            output,
            exit_zero,
        }),
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
            database_url,
            format,
            output,
            exit_zero,
        } => commands::lint_cmd::run_lint(commands::lint_cmd::RunLintOpts {
            config_path: config,
            fix,
            dialect,
            files,
            database_url,
            format,
            output,
            exit_zero,
        }),
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
            explain,
            severity,
            exit_zero,
            output,
            dialect,
            config,
        } => commands::inspect::run_inspect(commands::inspect::RunInspectOpts {
            database_url,
            format,
            list_checks,
            explain,
            severity,
            exit_zero,
            output,
            dialect,
            config_path: config,
        }),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
