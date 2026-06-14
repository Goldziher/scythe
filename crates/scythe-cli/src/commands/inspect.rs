//! `scythe inspect` — connect to a live database and run operational health
//! checks, emitting findings in the same reporter shapes used by `scythe
//! audit` (human / SARIF / JSON).
//!
//! Phase 0 ships PostgreSQL checks only. MySQL is stubbed in scythe-inspect.

use std::fs::File;
use std::io::Write;

use scythe_inspect::{DbDriver, InspectError, MysqlDriver, PostgresDriver};
use scythe_lint::reporters::{Finding, Format};
use scythe_lint::{Severity, emit_findings};

const TOOL_NAME: &str = "scythe-inspect";
const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Inputs to [`run_inspect`]. Mirrors the clap `Commands::Inspect` shape.
pub struct RunInspectOpts {
    /// Positional database URL, if supplied.
    pub database_url: Option<String>,
    /// Reporter format string (human / sarif / json).
    pub format: String,
    /// `--list-checks` flag: print the catalog and exit 0.
    pub list_checks: bool,
    /// Severity floor — drop findings below this level.
    pub severity: Option<String>,
    /// `--exit-zero` flag: always exit 0 even with error findings.
    pub exit_zero: bool,
    /// Output path; `None` means stdout.
    pub output: Option<String>,
    /// Engine override. Defaults to the URL scheme (`postgres` if no URL).
    pub dialect: Option<String>,
}

pub fn run_inspect(opts: RunInspectOpts) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve engine first so `--list-checks` works without a URL.
    let engine = resolve_engine(opts.dialect.as_deref(), opts.database_url.as_deref());
    let mut driver = build_driver(&engine);

    if opts.list_checks {
        let mut out = open_output(opts.output.as_deref())?;
        print_check_catalog(driver.as_ref(), out.as_mut())?;
        return Ok(());
    }

    let format = Format::parse(&opts.format).ok_or_else(|| {
        format!(
            "unknown --format '{}' (expected human|sarif|json)",
            opts.format
        )
    })?;

    let severity_floor = match opts.severity.as_deref() {
        Some(s) => Some(
            Severity::parse_cli(s)
                .ok_or_else(|| format!("unknown --severity '{}' (expected off|warn|error)", s))?,
        ),
        None => None,
    };

    let url = resolve_url(opts.database_url.as_deref())?;

    // Build a single-threaded tokio runtime per invocation — keeps the rest
    // of the CLI (`lint`, `audit`, `generate`) synchronous.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let findings: Vec<Finding> = rt.block_on(async {
        driver.connect(&url).await?;
        driver.run_all().await
    })?;

    let mut findings = findings;
    if let Some(floor) = severity_floor {
        findings.retain(|f| f.severity >= floor);
    }

    let mut out = open_output(opts.output.as_deref())?;
    emit_findings(format, TOOL_NAME, TOOL_VERSION, &findings, out.as_mut())?;
    out.flush().ok();

    let error_count = findings
        .iter()
        .filter(|f| matches!(f.severity, Severity::Error))
        .count();
    if error_count > 0 && !opts.exit_zero {
        std::process::exit(2);
    }
    Ok(())
}

/// Resolve engine from explicit `--dialect`, else from URL scheme, else
/// default to `postgres`.
fn resolve_engine(dialect: Option<&str>, url: Option<&str>) -> String {
    if let Some(d) = dialect {
        return d.to_string();
    }
    if let Some(u) = url
        && let Some(scheme) = u.split("://").next()
    {
        // `postgres` and `postgresql` are aliases.
        return match scheme {
            "postgres" | "postgresql" => "postgres".to_string(),
            "mysql" | "mariadb" => "mysql".to_string(),
            other => other.to_string(),
        };
    }
    "postgres".to_string()
}

fn build_driver(engine: &str) -> Box<dyn DbDriver> {
    match engine {
        "postgres" | "postgresql" => Box::new(PostgresDriver::new()),
        "mysql" | "mariadb" => Box::new(MysqlDriver::new()),
        // Unknown engines surface via the trait stub (empty catalog +
        // connect errors at run time). Keeps `--list-checks` informative.
        _ => Box::new(MysqlDriver::new()),
    }
}

fn resolve_url(positional: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(u) = positional {
        return Ok(u.to_string());
    }
    if let Ok(u) = std::env::var("DATABASE_URL") {
        return Ok(u);
    }
    if let Ok(u) = std::env::var("SCYTHE_DATABASE_URL") {
        return Ok(u);
    }
    Err(Box::new(InspectError::UrlMissing))
}

fn open_output(path: Option<&str>) -> Result<Box<dyn Write>, Box<dyn std::error::Error>> {
    match path {
        None => Ok(Box::new(std::io::stdout())),
        Some(p) => {
            let f = File::create(p)
                .map_err(|e| format!("failed to open output file '{}': {}", p, e))?;
            Ok(Box::new(std::io::BufWriter::new(f)))
        }
    }
}

fn print_check_catalog(
    driver: &dyn DbDriver,
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = driver.checks();
    if catalog.is_empty() {
        writeln!(
            out,
            "no checks available for engine `{}` at Phase 0",
            driver.engine()
        )?;
        return Ok(());
    }

    let id_w = catalog.iter().map(|c| c.id.len()).max().unwrap_or(2).max(2);
    let name_w = catalog
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    writeln!(out, "[{}]", driver.engine())?;
    for entry in catalog {
        writeln!(
            out,
            "  {id:<id_w$}  {name:<name_w$}  {sev:<5}  {desc}",
            id = entry.id,
            name = entry.name,
            sev = severity_label(entry.severity),
            desc = entry.description,
            id_w = id_w,
            name_w = name_w,
        )?;
    }
    Ok(())
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Off => "off",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_engine_explicit_dialect_wins() {
        assert_eq!(
            resolve_engine(Some("mysql"), Some("postgres://x/y")),
            "mysql"
        );
    }

    #[test]
    fn resolve_engine_from_url_scheme() {
        assert_eq!(resolve_engine(None, Some("postgres://u@h/db")), "postgres");
        assert_eq!(
            resolve_engine(None, Some("postgresql://u@h/db")),
            "postgres"
        );
        assert_eq!(resolve_engine(None, Some("mysql://u@h/db")), "mysql");
        assert_eq!(resolve_engine(None, Some("mariadb://u@h/db")), "mysql");
    }

    #[test]
    fn resolve_engine_defaults_to_postgres() {
        assert_eq!(resolve_engine(None, None), "postgres");
    }

    #[test]
    fn resolve_url_prefers_positional() {
        // Set both env vars so we can confirm the positional wins.
        // SAFETY: tests run single-threaded by default through cargo's main
        // harness when `#[test]` mutations like env::set_var are involved, but
        // the resolver only reads — the variable is checked, not mutated.
        // We avoid set_var here entirely.
        let positional = Some("postgres://positional/db");
        let url = resolve_url(positional).unwrap();
        assert_eq!(url, "postgres://positional/db");
    }

    #[test]
    fn resolve_url_missing_returns_clear_error() {
        // Run in a process where neither env var is set is hard to guarantee
        // in a unit test; instead, assert the error type when called with
        // None and verify the env-var fallthrough downstream via integration.
        let err = match resolve_url(None) {
            Ok(_) => return, // DATABASE_URL is set in CI, skip.
            Err(e) => e,
        };
        // Either the InspectError::UrlMissing or a wrapper carrying it.
        let s = err.to_string();
        assert!(s.contains("DATABASE_URL") || s.contains("no database URL"));
    }

    #[test]
    fn build_driver_dispatches_by_engine() {
        let pg = build_driver("postgres");
        assert_eq!(pg.engine(), "postgres");
        let pg2 = build_driver("postgresql");
        assert_eq!(pg2.engine(), "postgres");
        let my = build_driver("mysql");
        assert_eq!(my.engine(), "mysql");
        let mar = build_driver("mariadb");
        assert_eq!(mar.engine(), "mysql");
    }
}
