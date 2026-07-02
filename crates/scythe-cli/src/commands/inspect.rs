//! `scythe inspect` — connect to a live database and run operational health
//! checks, emitting findings in the same reporter shapes used by `scythe
//! audit` (human / SARIF / JSON).
//!
//! Phase 1C additions:
//! - `[inspect]` config section in `scythe.toml` (database_url, api_schemas,
//!   extra_rules, severity_overrides, suppression rules, inline checks).
//! - `--explain <CHECK_ID>` flag: prints full rationale without a DB connection.
//! - Suppression engine applied to findings before emission.
//! - Severity overrides applied to the registry before running.
//! - URL precedence: CLI positional > `$DATABASE_URL` > `$SCYTHE_DATABASE_URL`
//!   > `[inspect].database_url` in `scythe.toml`.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use scythe_inspect::{
    CheckRegistry, DbDriver, InspectConfig, InspectError, MysqlDriver, PostgresDriver, SuppressionEngine,
    parse_inspect_section,
};
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
    /// `--explain <CHECK_ID>` flag: print the check's rationale and exit 0.
    pub explain: Option<String>,
    /// Severity floor — drop findings below this level.
    pub severity: Option<String>,
    /// `--exit-zero` flag: always exit 0 even with error findings.
    pub exit_zero: bool,
    /// Output path; `None` means stdout.
    pub output: Option<String>,
    /// Engine override. Defaults to the URL scheme (`postgres` if no URL).
    pub dialect: Option<String>,
    /// Path to `scythe.toml` (default: `"scythe.toml"`).
    pub config_path: String,
}

pub fn run_inspect(opts: RunInspectOpts) -> Result<(), Box<dyn std::error::Error>> {
    // ------------------------------------------------------------------
    // Load [inspect] config from scythe.toml (best-effort; None if absent).
    // ------------------------------------------------------------------
    let inspect_config: Option<InspectConfig> =
        parse_inspect_section(Path::new(&opts.config_path)).unwrap_or_else(|e| {
            eprintln!("scythe-inspect: warning: could not parse [inspect] config: {e}");
            None
        });

    // ------------------------------------------------------------------
    // Resolve engine first so `--list-checks` / `--explain` work without a URL.
    // ------------------------------------------------------------------
    let engine = resolve_engine(opts.dialect.as_deref(), opts.database_url.as_deref());

    // ------------------------------------------------------------------
    // Build the registry (canonical + user checks + severity overrides).
    // ------------------------------------------------------------------
    let registry = build_registry(opts.config_path.as_str(), &engine, &inspect_config)?;

    // ------------------------------------------------------------------
    // Discovery flags — no DB connection needed.
    // ------------------------------------------------------------------
    if opts.list_checks || opts.explain.is_some() {
        let mut out = open_output(opts.output.as_deref())?;
        if opts.list_checks {
            print_check_catalog_from_registry(&registry, &engine, out.as_mut())?;
            return Ok(());
        }
        if let Some(id) = &opts.explain {
            print_explanation(&registry, &engine, id, out.as_mut())?;
            return Ok(());
        }
    }

    let format = Format::parse(&opts.format)
        .ok_or_else(|| format!("unknown --format '{}' (expected human|sarif|json)", opts.format))?;

    let severity_floor = match opts.severity.as_deref() {
        Some(s) => Some(
            Severity::parse_cli(s).ok_or_else(|| format!("unknown --severity '{}' (expected off|warn|error)", s))?,
        ),
        None => None,
    };

    // ------------------------------------------------------------------
    // Resolve DB URL: CLI positional > $DATABASE_URL > $SCYTHE_DATABASE_URL
    // > [inspect].database_url.
    // ------------------------------------------------------------------
    let config_db_url = inspect_config.as_ref().and_then(|c| c.database_url.as_deref());
    let url = resolve_url(opts.database_url.as_deref(), config_db_url)?;

    // ------------------------------------------------------------------
    // Build the driver with config-aware extras.
    // ------------------------------------------------------------------
    let mut driver = build_driver_with_config(&engine, registry, &inspect_config);

    // Build a single-threaded tokio runtime per invocation — keeps the rest
    // of the CLI (`lint`, `audit`, `generate`) synchronous.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

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

// ---------------------------------------------------------------------------
// Registry builder
// ---------------------------------------------------------------------------

/// Build a [`CheckRegistry`] for the given engine with:
/// 1. The canonical built-in checks.
/// 2. Inline user checks from `[[inspect.check]]`.
/// 3. Extra rules from `extra_rules` paths (already resolved to absolute by
///    `parse_inspect_section`).
/// 4. Severity overrides from `[inspect.severity_overrides]`.
pub(crate) fn build_registry(
    config_path: &str,
    _engine: &str,
    inspect_config: &Option<InspectConfig>,
) -> Result<CheckRegistry, Box<dyn std::error::Error>> {
    let mut registry = CheckRegistry::canonical();

    if let Some(cfg) = inspect_config {
        // Add inline [[inspect.check]] specs (already validated by
        // parse_inspect_section).
        if !cfg.check.is_empty() {
            registry = registry.with_inline_checks(cfg.check.clone());
        }

        // Load extra_rules TOML files (paths already resolved to absolute by
        // parse_inspect_section).
        for abs_path in &cfg.extra_rules {
            registry = registry
                .with_user_checks(Path::new(abs_path))
                .map_err(|e| format!("failed to load extra_rules '{}': {}", abs_path, e))?;
        }

        // Apply severity overrides BEFORE running checks.
        if !cfg.severity_overrides.is_empty() {
            registry.apply_severity_overrides(&cfg.severity_overrides);
        }
    }

    let _ = config_path; // used only for error attribution, resolved paths are in cfg
    Ok(registry)
}

// ---------------------------------------------------------------------------
// Driver builder
// ---------------------------------------------------------------------------

/// Build and configure the right driver for `engine`, wiring in the
/// suppression engine and api_schemas from config.
pub(crate) fn build_driver_with_config(
    engine: &str,
    registry: CheckRegistry,
    inspect_config: &Option<InspectConfig>,
) -> Box<dyn DbDriver> {
    match engine {
        "postgres" | "postgresql" => {
            let mut driver = PostgresDriver::with_registry(registry);

            if let Some(cfg) = inspect_config {
                // Wire suppression rules.
                if !cfg.suppression.is_empty() {
                    driver.set_suppression(SuppressionEngine::new(cfg.suppression.clone()));
                }
                // Wire api_schemas for SC-INS10.
                if !cfg.api_schemas.is_empty() {
                    driver.set_api_schemas(cfg.api_schemas.clone());
                }
            }

            Box::new(driver)
        }
        // MySQL / unknown engines: return the stub (suppression/api_schemas
        // don't apply since there are no MySQL checks yet).
        _ => Box::new(MysqlDriver::new()),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve engine from explicit `--dialect`, else from URL scheme, else
/// default to `postgres`.
fn resolve_engine(dialect: Option<&str>, url: Option<&str>) -> String {
    if let Some(d) = dialect {
        return d.to_string();
    }
    if let Some(u) = url
        && let Some(scheme) = u.split("://").next()
    {
        return match scheme {
            "postgres" | "postgresql" => "postgres".to_string(),
            "mysql" | "mariadb" => "mysql".to_string(),
            other => other.to_string(),
        };
    }
    "postgres".to_string()
}

/// Resolve the database URL with the full precedence chain:
/// CLI positional > `$DATABASE_URL` > `$SCYTHE_DATABASE_URL` >
/// `[inspect].database_url` in scythe.toml.
fn resolve_url(positional: Option<&str>, config_url: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    resolve_url_inner(
        positional,
        std::env::var("DATABASE_URL").ok().as_deref(),
        std::env::var("SCYTHE_DATABASE_URL").ok().as_deref(),
        config_url,
    )
}

/// Resolve the database URL for use from lint — returns `Some(url)` when a
/// URL is found via the full precedence chain, `None` when no URL is
/// configured.
///
/// Precedence: `[inspect].database_url` in `config` <
/// `$SCYTHE_DATABASE_URL` < `$DATABASE_URL`.  There is no CLI positional
/// arg in the lint context.
///
/// This is intentionally infallible so the caller can silently skip
/// inspection when no URL is found (the "no DB configured" path).
pub fn resolve_inspect_url(config: &Option<InspectConfig>) -> Option<String> {
    let config_url = config.as_ref().and_then(|c| c.database_url.as_deref());
    resolve_url_inner_opt(
        std::env::var("DATABASE_URL").ok().as_deref(),
        std::env::var("SCYTHE_DATABASE_URL").ok().as_deref(),
        config_url,
    )
}

/// Pure-function precedence resolver — separated so tests can inject env
/// values without racing on `std::env::set_var`.
fn resolve_url_inner(
    positional: Option<&str>,
    env_database_url: Option<&str>,
    env_scythe_database_url: Option<&str>,
    config_url: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(u) = positional {
        return Ok(u.to_string());
    }
    resolve_url_inner_opt(env_database_url, env_scythe_database_url, config_url)
        .ok_or_else(|| Box::new(InspectError::UrlMissing) as Box<dyn std::error::Error>)
}

/// Pure infallible URL resolver used by both [`resolve_url`] (which wraps
/// it in `Result`) and [`resolve_inspect_url`] (which returns `Option`).
///
/// Precedence (highest → lowest): `$DATABASE_URL` > `$SCYTHE_DATABASE_URL`
/// > config file.
fn resolve_url_inner_opt(
    env_database_url: Option<&str>,
    env_scythe_database_url: Option<&str>,
    config_url: Option<&str>,
) -> Option<String> {
    if let Some(u) = env_database_url {
        return Some(u.to_string());
    }
    if let Some(u) = env_scythe_database_url {
        return Some(u.to_string());
    }
    config_url.map(|u| u.to_string())
}

fn open_output(path: Option<&str>) -> Result<Box<dyn Write>, Box<dyn std::error::Error>> {
    match path {
        None => Ok(Box::new(std::io::stdout())),
        Some(p) => {
            let f = File::create(p).map_err(|e| format!("failed to open output file '{}': {}", p, e))?;
            Ok(Box::new(std::io::BufWriter::new(f)))
        }
    }
}

/// Print a check catalog from a [`CheckRegistry`], grouped by engine.
fn print_check_catalog_from_registry(
    registry: &CheckRegistry,
    engine: &str,
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let checks: Vec<_> = registry.for_engine(engine).collect();
    if checks.is_empty() {
        writeln!(
            out,
            "no checks available for engine `{engine}` — try `scythe inspect --list-checks` with --dialect postgres"
        )?;
        return Ok(());
    }

    let id_w = checks.iter().map(|c| c.id.len()).max().unwrap_or(2).max(2);
    let name_w = checks.iter().map(|c| c.name.len()).max().unwrap_or(4).max(4);

    writeln!(out, "[{engine}]")?;
    for check in checks {
        writeln!(
            out,
            "  {id:<id_w$}  {name:<name_w$}  {sev:<5}  {desc}",
            id = check.id,
            name = check.name,
            sev = severity_label(check.severity),
            desc = check.description,
            id_w = id_w,
            name_w = name_w,
        )?;
    }
    Ok(())
}

/// Print the full explanation for a single check ID.
///
/// Mirrors `audit::print_rule_explanation` for consistency.
///
/// Exits with an error message if the ID is not found.
pub(crate) fn print_explanation(
    registry: &CheckRegistry,
    engine: &str,
    id: &str,
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = registry
        .get(id)
        .filter(|s| s.engines.iter().any(|e| e == engine))
        .ok_or_else(|| {
            format!(
                "no check with id '{id}' for engine '{engine}' — try `scythe inspect --list-checks --dialect {engine}`"
            )
        })?;

    writeln!(out, "{} — {}", spec.id, spec.name)?;
    writeln!(out, "  category: {}", spec.category)?;
    writeln!(out, "  severity: {}", severity_label(spec.severity))?;
    writeln!(out, "  engines:  {}", spec.engines.join(", "))?;
    if !spec.cwe.is_empty() {
        writeln!(out, "  cwe:      {}", spec.cwe.join(", "))?;
    }
    writeln!(out)?;
    writeln!(out, "{}", spec.description)?;
    if let Some(explanation) = &spec.explanation {
        writeln!(out)?;
        writeln!(out, "Explanation")?;
        writeln!(out, "-----------")?;
        writeln!(out, "{explanation}")?;
    }
    if let Some(remediation) = &spec.remediation {
        writeln!(out)?;
        writeln!(out, "Remediation")?;
        writeln!(out, "-----------")?;
        writeln!(out, "{remediation}")?;
    }
    writeln!(out)?;
    writeln!(out, "SQL")?;
    writeln!(out, "---")?;
    writeln!(out, "{}", spec.sql.trim())?;
    Ok(())
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Off => "off",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // resolve_engine
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_engine_explicit_dialect_wins() {
        assert_eq!(resolve_engine(Some("mysql"), Some("postgres://x/y")), "mysql");
    }

    #[test]
    fn resolve_engine_from_url_scheme() {
        assert_eq!(resolve_engine(None, Some("postgres://u@h/db")), "postgres");
        assert_eq!(resolve_engine(None, Some("postgresql://u@h/db")), "postgres");
        assert_eq!(resolve_engine(None, Some("mysql://u@h/db")), "mysql");
        assert_eq!(resolve_engine(None, Some("mariadb://u@h/db")), "mysql");
    }

    #[test]
    fn resolve_engine_defaults_to_postgres() {
        assert_eq!(resolve_engine(None, None), "postgres");
    }

    // -----------------------------------------------------------------------
    // resolve_url
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_url_prefers_positional() {
        let url = resolve_url(Some("postgres://positional/db"), Some("postgres://config/db")).unwrap();
        assert_eq!(url, "postgres://positional/db");
    }

    #[test]
    fn resolve_url_inner_prefers_positional_over_everything() {
        let url = resolve_url_inner(
            Some("postgres://cli/db"),
            Some("postgres://env-db/db"),
            Some("postgres://env-scythe/db"),
            Some("postgres://config/db"),
        )
        .unwrap();
        assert_eq!(url, "postgres://cli/db");
    }

    #[test]
    fn resolve_url_inner_prefers_database_url_over_scythe_and_config() {
        let url = resolve_url_inner(
            None,
            Some("postgres://env-db/db"),
            Some("postgres://env-scythe/db"),
            Some("postgres://config/db"),
        )
        .unwrap();
        assert_eq!(url, "postgres://env-db/db");
    }

    #[test]
    fn resolve_url_inner_prefers_scythe_database_url_over_config() {
        let url = resolve_url_inner(
            None,
            None,
            Some("postgres://env-scythe/db"),
            Some("postgres://config/db"),
        )
        .unwrap();
        assert_eq!(url, "postgres://env-scythe/db");
    }

    #[test]
    fn resolve_url_inner_falls_back_to_config_when_no_cli_or_env() {
        let url = resolve_url_inner(None, None, None, Some("postgres://config/db")).unwrap();
        assert_eq!(url, "postgres://config/db");
    }

    #[test]
    fn resolve_url_inner_returns_clear_error_when_all_sources_absent() {
        let err = resolve_url_inner(None, None, None, None).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("DATABASE_URL") || s.contains("no database URL"));
    }

    // -----------------------------------------------------------------------
    // resolve_inspect_url (infallible Option-returning variant used by lint)
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_inspect_url_returns_none_when_no_url_configured() {
        // With no env vars set (we use a fresh config with no database_url),
        // the function should return None — the "silently skip" signal.
        // We can't unset env vars safely in parallel tests, so we test the
        // helper directly via resolve_url_inner_opt.
        let result = resolve_url_inner_opt(None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_inspect_url_inner_opt_prefers_database_url_over_scythe_and_config() {
        let result = resolve_url_inner_opt(
            Some("postgres://env-db/db"),
            Some("postgres://env-scythe/db"),
            Some("postgres://config/db"),
        );
        assert_eq!(result.as_deref(), Some("postgres://env-db/db"));
    }

    #[test]
    fn resolve_inspect_url_inner_opt_prefers_scythe_database_url_over_config() {
        let result = resolve_url_inner_opt(None, Some("postgres://env-scythe/db"), Some("postgres://config/db"));
        assert_eq!(result.as_deref(), Some("postgres://env-scythe/db"));
    }

    #[test]
    fn resolve_inspect_url_inner_opt_falls_back_to_config() {
        let result = resolve_url_inner_opt(None, None, Some("postgres://config/db"));
        assert_eq!(result.as_deref(), Some("postgres://config/db"));
    }

    #[test]
    fn resolve_inspect_url_returns_none_from_empty_config() {
        let config: Option<InspectConfig> = Some(InspectConfig::default());
        // With no env vars (we control the inner function), this is None.
        let result = resolve_url_inner_opt(None, None, config.as_ref().and_then(|c| c.database_url.as_deref()));
        assert!(result.is_none());
    }

    #[test]
    fn resolve_inspect_url_returns_url_from_config_database_url() {
        let config = InspectConfig {
            database_url: Some("postgres://from-config/db".to_string()),
            ..Default::default()
        };
        let result = resolve_url_inner_opt(None, None, config.database_url.as_deref());
        assert_eq!(result.as_deref(), Some("postgres://from-config/db"));
    }

    // -----------------------------------------------------------------------
    // print_explanation
    // -----------------------------------------------------------------------

    #[test]
    fn explain_known_id_prints_body() {
        let registry = CheckRegistry::canonical();
        let mut buf = Vec::new();
        print_explanation(&registry, "postgres", "SC-INS04", &mut buf).expect("explain SC-INS04");
        let output = String::from_utf8(buf).unwrap();
        assert!(
            output.contains("no-primary-key"),
            "output should contain check name; got:\n{output}"
        );
        assert!(output.contains("SC-INS04"));
    }

    #[test]
    fn explain_unknown_id_errors() {
        let registry = CheckRegistry::canonical();
        let mut buf = Vec::new();
        let result = print_explanation(&registry, "postgres", "SC-NOPE", &mut buf);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("SC-NOPE"),
            "error message should include the unknown ID; got: {msg}"
        );
    }

    /// Engine-aware lookup: requesting a Postgres-only check ID with
    /// `--dialect mysql` must return a clear not-found error rather than
    /// printing the Postgres explanation.
    #[test]
    fn explain_postgres_check_under_mysql_dialect_errors() {
        let registry = CheckRegistry::canonical();
        let mut buf = Vec::new();
        let result = print_explanation(&registry, "mysql", "SC-INS04", &mut buf);
        assert!(
            result.is_err(),
            "SC-INS04 is postgres-only — lookup under mysql should fail"
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("SC-INS04"));
        assert!(msg.contains("mysql"));
    }

    // -----------------------------------------------------------------------
    // build_driver_with_config_dispatches_by_engine
    // -----------------------------------------------------------------------

    #[test]
    fn build_driver_with_config_dispatches_by_engine() {
        let registry = CheckRegistry::canonical();
        let pg = build_driver_with_config("postgres", registry, &None);
        assert_eq!(pg.engine(), "postgres");

        let registry2 = CheckRegistry::canonical();
        let pg2 = build_driver_with_config("postgresql", registry2, &None);
        assert_eq!(pg2.engine(), "postgres");

        let registry3 = CheckRegistry::canonical();
        let my = build_driver_with_config("mysql", registry3, &None);
        assert_eq!(my.engine(), "mysql");
    }
}
