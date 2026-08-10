use std::borrow::Cow;
use std::path::Path;

use scythe_inspect::parse_inspect_section;
use scythe_lint::reporters::{Finding, Format};
use scythe_lint::sqruff_adapter;
use scythe_lint::types::Severity;
use scythe_lint::{emit_findings, extract_cwe};

use super::inspect::{build_driver_with_config, build_registry};
use super::shared::{config_dir, engine_to_sqruff_dialect, resolve_globs, split_query_file, validate_dialect};

/// A combined lint violation that can come from either scythe rules, sqruff,
/// or live-DB inspect checks.
struct FileViolation {
    file: String,
    query_name: Option<String>,
    rule_id: Cow<'static, str>,
    severity: Severity,
    message: String,
    line_no: Option<usize>,
    line_pos: Option<usize>,
    /// Source sub-tool (`"lint"`, `"audit"`, `"inspect"`).  `None` means lint.
    source: Option<String>,
}

/// Inputs to [`run_lint`]. Mirrors the clap `Commands::Lint` shape.
pub struct RunLintOpts {
    pub config_path: String,
    pub fix: bool,
    pub dialect: Option<String>,
    pub files: Vec<String>,
    /// Explicit live-DB URL for the auto-run `inspect` pass. Opt-in only --
    /// see [`try_run_inspect`]'s doc comment for why this is no longer
    /// resolved from `$DATABASE_URL`/`$SCYTHE_DATABASE_URL` on its own (#210).
    pub database_url: Option<String>,
    pub format: String,
    pub output: Option<String>,
    pub exit_zero: bool,
}

/// Run the `lint` command.
///
/// - If files are provided without a valid config, run sqruff-only linting.
/// - If a config is available, run both scythe rules (with schema context) and sqruff rules.
/// - `--fix`: apply sqruff auto-fixes to files.
///
/// Findings are emitted via `emit_findings` (human/sarif/json, matching
/// `audit`/`check`/`inspect`) to stdout or `--output`; per-violation detail
/// and progress messages stay on stderr. Exits 2 on any error-severity
/// finding unless `--exit-zero` is set -- previously `lint` returned a plain
/// `Err`, which fell through to `main`'s generic exit(1) path and made a
/// lint failure indistinguishable from an operational failure (a bad
/// config, an unreadable file). See #212.
pub fn run_lint(opts: RunLintOpts) -> Result<(), Box<dyn std::error::Error>> {
    let RunLintOpts {
        config_path,
        fix,
        dialect,
        files,
        database_url,
        format,
        output,
        exit_zero,
    } = opts;
    let config_path = config_path.as_str();

    let format =
        Format::parse(&format).ok_or_else(|| format!("unknown --format '{}' (expected human|sarif|json)", format))?;

    let has_config = Path::new(config_path).exists();

    let violations = if !files.is_empty() {
        let config_dialect = if has_config {
            super::shared::dialect_from_config(config_path)?
        } else {
            None
        };
        let d = match dialect.as_deref() {
            Some(raw) => validate_dialect(raw)?,
            None => config_dialect.unwrap_or_else(|| "ansi".to_string()),
        };
        lint_files(&files, &d, fix, config_path, database_url.as_deref())?
    } else if !has_config {
        return Err(format!(
            "No files specified and config '{}' not found. Provide files or a config path.",
            config_path
        )
        .into());
    } else {
        lint_from_config(config_path, dialect.as_deref(), fix, database_url.as_deref())?
    };

    report_violations(&violations, format, output.as_deref(), exit_zero)
}

/// Lint specific files using sqruff only (no scythe schema-aware rules).
///
/// Also auto-runs inspect when a DB URL is configured via
/// `[inspect].database_url` (if scythe.toml exists at `config_path`) or the
/// explicit `database_url` argument (`--database-url`). See
/// [`try_run_inspect`]'s doc comment for why a bare environment variable is
/// no longer enough on its own (#210).
fn lint_files(
    files: &[String],
    dialect: &str,
    fix: bool,
    config_path: &str,
    database_url: Option<&str>,
) -> Result<Vec<FileViolation>, Box<dyn std::error::Error>> {
    let mut all_violations: Vec<FileViolation> = Vec::new();

    for path in files {
        let sql = std::fs::read_to_string(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

        if fix {
            let (_pre_fix_violations, fixed) = sqruff_adapter::lint_and_fix_sql(&sql, dialect, None)
                .map_err(|e| format!("sqruff config error on '{}': {}", path, e))?;
            if fixed != sql {
                std::fs::write(path, &fixed).map_err(|e| format!("failed to write '{}': {}", path, e))?;
                eprintln!("fixed {}", path);

                // Re-lint the fixed content rather than reporting the
                // pre-fix violation list: those violations were just
                // resolved by the write above, and reporting them as
                // current findings fails CI on issues that no longer exist
                // (#210).
                let remaining = sqruff_adapter::lint_sql(&fixed, dialect, None)
                    .map_err(|e| format!("sqruff config error on '{}': {}", path, e))?;
                for sv in &remaining {
                    all_violations.push(FileViolation {
                        file: path.clone(),
                        query_name: None,
                        rule_id: sv.violation.rule_id.clone(),
                        severity: Severity::Warn,
                        message: sv.violation.message.clone(),
                        line_no: Some(sv.line_no),
                        line_pos: Some(sv.line_pos),
                        source: None,
                    });
                }
            } else {
                // Nothing changed, so the pre-fix violation list is still
                // exactly what a plain (non-fixing) lint would report.
                for sv in &_pre_fix_violations {
                    all_violations.push(FileViolation {
                        file: path.clone(),
                        query_name: None,
                        rule_id: sv.violation.rule_id.clone(),
                        severity: Severity::Warn,
                        message: sv.violation.message.clone(),
                        line_no: Some(sv.line_no),
                        line_pos: Some(sv.line_pos),
                        source: None,
                    });
                }
            }
        } else {
            let violations = sqruff_adapter::lint_sql(&sql, dialect, None)
                .map_err(|e| format!("sqruff config error on '{}': {}", path, e))?;
            for sv in &violations {
                all_violations.push(FileViolation {
                    file: path.clone(),
                    query_name: None,
                    rule_id: sv.violation.rule_id.clone(),
                    severity: Severity::Warn,
                    message: sv.violation.message.clone(),
                    line_no: Some(sv.line_no),
                    line_pos: Some(sv.line_pos),
                    source: None,
                });
            }
        }
    }

    let inspect_violations = try_run_inspect(config_path, database_url, None);
    all_violations.extend(inspect_violations);

    Ok(all_violations)
}

/// Lint from config: run both scythe rules and sqruff rules, then auto-run
/// inspect when a database URL is configured (via `[inspect].database_url`
/// in `scythe.toml` or the explicit `database_url` argument). See
/// [`try_run_inspect`]'s doc comment for why a bare environment variable is
/// no longer enough on its own (#210).
fn lint_from_config(
    config_path: &str,
    cli_dialect: Option<&str>,
    fix: bool,
    database_url: Option<&str>,
) -> Result<Vec<FileViolation>, Box<dyn std::error::Error>> {
    use serde::Deserialize;

    use scythe_core::analyzer::analyze;
    use scythe_core::catalog::Catalog;
    use scythe_core::dialect::SqlDialect;
    use scythe_core::parser::parse_query_with_dialect;
    use scythe_lint::{LintContext, LintEngine, default_registry};

    #[derive(Deserialize)]
    struct ScytheConfig {
        sql: Vec<SqlConfig>,
        #[serde(default)]
        lint: Option<scythe_lint::types::LintConfig>,
    }

    #[derive(Deserialize)]
    struct SqlConfig {
        name: String,
        schema: Vec<String>,
        queries: Vec<String>,
        #[allow(dead_code)]
        #[serde(default)]
        engine: String,
    }

    let config_str =
        std::fs::read_to_string(config_path).map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }
    let engine = LintEngine::new(registry);

    let sqruff_config = config.lint.as_ref().and_then(|lc| lc.sqruff.as_ref());

    let mut all_violations: Vec<FileViolation> = Vec::new();

    let base_dir = config_dir(config_path);

    for sql_config in &config.sql {
        eprintln!("[{}] Parsing schema...", sql_config.name);

        // An explicit `--dialect` is validated (and, for a scythe engine
        // alias like `postgresql`, translated to sqruff's own spelling)
        // before it can reach sqruff-lib, which panics on an unrecognized
        // dialect string rather than returning an error (#205). A
        // config-derived dialect is already a sqruff-native name (produced
        // by `engine_to_sqruff_dialect`), so it needs no re-validation.
        let sqruff_dialect = match cli_dialect {
            Some(raw) => validate_dialect(raw)?,
            None => engine_to_sqruff_dialect(&sql_config.engine).to_string(),
        };
        let sqruff_dialect = sqruff_dialect.as_str();

        // Validate [lint.sqruff] before reading any query file. The per-file
        // calls below return Err on a bad config, and that Err aborts the whole
        // run -- including every scythe-native finding already collected. Failing
        // here reports a config mistake as a config mistake, not as an error
        // against an arbitrary query file.
        sqruff_adapter::validate_config(sqruff_dialect, sqruff_config)
            .map_err(|e| format!("[{}] invalid [lint.sqruff] configuration: {}", sql_config.name, e))?;

        let schema_files = resolve_globs(&sql_config.schema, base_dir, &format!("[{}] schema", sql_config.name))?;
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| std::fs::read_to_string(p).map_err(|e| format!("failed to read schema file '{}': {}", p, e)))
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

        let sql_dialect = SqlDialect::from_str(&sql_config.engine).unwrap_or(SqlDialect::PostgreSQL);
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &sql_dialect)?;

        let query_files = resolve_globs(&sql_config.queries, base_dir, &format!("[{}] queries", sql_config.name))?;

        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;

            if super::shared::has_unannotated_sql(&content) {
                return Err(format!(
                    "[{}] query file '{}' has SQL content with no `-- name:` / `-- @name` annotation above it; \
                     that content is silently skipped, not linted. Add an annotation above every statement \
                     (e.g. `-- name: MyQuery :one`), or move non-query SQL out of the `queries` glob",
                    sql_config.name, query_file
                )
                .into());
            }

            if fix {
                let (pre_fix_violations, fixed) =
                    sqruff_adapter::lint_and_fix_sql(&content, sqruff_dialect, sqruff_config)
                        .map_err(|e| format!("sqruff config error on '{}': {}", query_file, e))?;
                if fixed != content {
                    std::fs::write(query_file, &fixed)
                        .map_err(|e| format!("failed to write '{}': {}", query_file, e))?;
                    eprintln!("fixed {}", query_file);

                    // Re-lint the fixed content: `pre_fix_violations` names
                    // violations the write above just resolved, and
                    // reporting them as current findings fails CI on issues
                    // that no longer exist (#210).
                    let remaining = sqruff_adapter::lint_sql(&fixed, sqruff_dialect, sqruff_config)
                        .map_err(|e| format!("sqruff config error on '{}': {}", query_file, e))?;
                    for sv in &remaining {
                        all_violations.push(FileViolation {
                            file: query_file.clone(),
                            query_name: None,
                            rule_id: sv.violation.rule_id.clone(),
                            severity: Severity::Warn,
                            message: sv.violation.message.clone(),
                            line_no: Some(sv.line_no),
                            line_pos: Some(sv.line_pos),
                            source: None,
                        });
                    }
                } else {
                    for sv in &pre_fix_violations {
                        all_violations.push(FileViolation {
                            file: query_file.clone(),
                            query_name: None,
                            rule_id: sv.violation.rule_id.clone(),
                            severity: Severity::Warn,
                            message: sv.violation.message.clone(),
                            line_no: Some(sv.line_no),
                            line_pos: Some(sv.line_pos),
                            source: None,
                        });
                    }
                }
            } else {
                let sq_violations = sqruff_adapter::lint_sql(&content, sqruff_dialect, sqruff_config)
                    .map_err(|e| format!("sqruff config error on '{}': {}", query_file, e))?;
                for sv in &sq_violations {
                    all_violations.push(FileViolation {
                        file: query_file.clone(),
                        query_name: None,
                        rule_id: sv.violation.rule_id.clone(),
                        severity: Severity::Warn,
                        message: sv.violation.message.clone(),
                        line_no: Some(sv.line_no),
                        line_pos: Some(sv.line_pos),
                        source: None,
                    });
                }
            }

            let blocks = split_query_file(&content);
            for block in &blocks {
                let parsed = match parse_query_with_dialect(block, &sql_dialect) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("warning: failed to parse query in '{}': {}", query_file, e);
                        continue;
                    }
                };
                let analyzed = match analyze(&catalog, &parsed) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!(
                            "warning: failed to analyze query '{}' in '{}': {}",
                            parsed.annotations.name, query_file, e
                        );
                        continue;
                    }
                };

                let ctx = LintContext {
                    sql: &parsed.sql,
                    stmt: &parsed.stmt,
                    analyzed: &analyzed,
                    catalog: &catalog,
                    annotations: &parsed.annotations,
                    dialect: sql_dialect,
                };

                let violations = engine.check_query(&ctx);
                for (v, sev) in violations {
                    all_violations.push(FileViolation {
                        file: query_file.clone(),
                        query_name: Some(analyzed.name.clone()),
                        rule_id: v.rule_id,
                        severity: sev,
                        message: v.message,
                        line_no: None,
                        line_pos: None,
                        source: None,
                    });
                }
            }
        }

        let cat_violations = engine.check_catalog(&catalog);
        for (v, sev) in cat_violations {
            all_violations.push(FileViolation {
                file: String::new(),
                query_name: None,
                rule_id: v.rule_id,
                severity: sev,
                message: v.message,
                line_no: None,
                line_pos: None,
                source: None,
            });
        }
    }

    // The engine of the first `[[sql]]` block, not a hardcoded "postgres":
    // `try_run_inspect` used to always build a PostgreSQL driver regardless
    // of what the config declared (#210). A config with more than one
    // engine still only gets one inspect pass -- inspect (like `scythe
    // inspect` itself) connects to a single database, so there is no
    // single "right" engine for a genuinely mixed-engine config; the first
    // block's is the best available default.
    let inspect_engine = config
        .sql
        .first()
        .map(|s| engine_to_sqruff_dialect(&s.engine))
        .filter(|e| *e == "postgres" || *e == "mysql")
        .unwrap_or("postgres");

    let inspect_findings = try_run_inspect(config_path, database_url, Some(inspect_engine));
    all_violations.extend(inspect_findings);

    Ok(all_violations)
}

/// Try to run live-DB inspect checks and return them as `FileViolation`s.
///
/// Connects only when a URL is explicitly configured: `explicit_url`
/// (`--database-url`) or `[inspect].database_url` in `scythe.toml`. Unlike
/// `scythe inspect`, this deliberately does **not** fall back to a bare
/// `$DATABASE_URL`/`$SCYTHE_DATABASE_URL` environment variable -- before
/// this, `scythe lint` opened an outbound database connection purely
/// because an environment variable happened to be set, with no flag to
/// request it, no mention in `--help`, and (absent a `tracing-subscriber`,
/// which `scythe-cli` never installs) no visible diagnostic if the
/// connection failed. That combination -- ambient network I/O from a linter,
/// invisible either way -- is what #210 closes. `scythe check`'s
/// `--database-url` already established this opt-in-only precedent; this
/// brings `lint` in line with it.
///
/// Still intentionally infallible given a URL:
/// - If no database URL is configured (the common case), emit one
///   `tracing::debug!` line and return an empty vec.
/// - If a URL is found but the connection fails, print an `eprintln!`
///   warning (host only — never the full URL) and return an empty vec, so
///   the failure is visible without needing a `tracing` subscriber. Lint's
///   exit code is unaffected by a failed inspect connection.
///
/// Inspect findings that do come back carry `source: Some("inspect")` so
/// mixed output is distinguishable.
fn try_run_inspect(config_path: &str, explicit_url: Option<&str>, engine_override: Option<&str>) -> Vec<FileViolation> {
    let inspect_config = parse_inspect_section(Path::new(config_path)).unwrap_or_else(|e| {
        tracing::debug!("scythe lint: could not parse [inspect] config: {e}");
        None
    });

    // Explicit sources only: `--database-url`, then `[inspect].database_url`.
    // No `$DATABASE_URL`/`$SCYTHE_DATABASE_URL` fallback -- see this
    // function's doc comment.
    let url = match explicit_url
        .map(str::to_string)
        .or_else(|| inspect_config.as_ref().and_then(|c| c.database_url.clone()))
    {
        Some(u) => u,
        None => {
            tracing::debug!("scythe lint: no database URL configured — skipping live inspect");
            return Vec::new();
        }
    };

    let engine = engine_override.unwrap_or("postgres");

    let registry = match build_registry(config_path, engine, &inspect_config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("scythe lint: warning: failed to build inspect registry: {e}");
            return Vec::new();
        }
    };

    let mut driver = build_driver_with_config(engine, registry, &inspect_config);

    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("scythe lint: warning: failed to build tokio runtime for inspect: {e}");
            return Vec::new();
        }
    };

    let findings = rt.block_on(async {
        if let Err(e) = driver.connect(&url).await {
            let host = url
                .split("://")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .and_then(|authority| authority.split('@').next_back())
                .unwrap_or("<unknown>");
            eprintln!("scythe lint: warning: could not connect to inspect database (host: {host}): {e}");
            return Vec::new();
        }
        match driver.run_all().await {
            Ok(f) => f,
            Err(e) => {
                eprintln!("scythe lint: warning: inspect checks failed: {e}");
                Vec::new()
            }
        }
    });

    findings
        .into_iter()
        .map(|f| FileViolation {
            file: f.file,
            query_name: f.query_name,
            rule_id: Cow::Owned(f.rule_id),
            severity: f.severity,
            message: f.message,
            line_no: f.line,
            line_pos: f.column,
            source: f.source,
        })
        .collect()
}

/// Report violations two ways:
///
/// - The full finding set, converted to `scythe_lint::reporters::Finding`
///   and written via `emit_findings` (human/sarif/json) to stdout or
///   `--output` -- matching `audit`/`check`/`inspect`. Before this, every
///   `scythe lint` finding went only to stderr with no `--format`/`--output`,
///   so `scythe lint 2>/dev/null` produced empty stdout even with real
///   findings, and there was no machine-readable output at all. See #212.
/// - The same per-file, per-violation detail lint has always printed to
///   stderr, plus a final tally, unchanged in wording so existing tooling
///   that greps stderr keeps working.
///
/// Exits 2 (not the plain-`Err` exit 1 this used to fall through to) when
/// any error-severity finding is present, unless `exit_zero` is set --
/// aligning `lint`'s exit code with `audit`/`check`/`inspect` so exit 1 is
/// reserved for operational failures (bad config, unreadable file) and
/// exit 2 unambiguously means "findings present." See #212.
fn report_violations(
    violations: &[FileViolation],
    format: Format,
    output_path: Option<&str>,
    exit_zero: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let findings: Vec<Finding> = violations
        .iter()
        .filter(|v| !matches!(v.severity, Severity::Off))
        .map(|v| Finding {
            file: v.file.clone(),
            query_name: v.query_name.clone(),
            rule_id: v.rule_id.to_string(),
            rule_name: None,
            rule_description: None,
            severity: v.severity,
            message: v.message.clone(),
            line: v.line_no,
            column: v.line_pos,
            cwe: extract_cwe(&v.message),
            source: v.source.clone(),
        })
        .collect();

    let mut out: Box<dyn std::io::Write> = match output_path {
        Some(path) => Box::new(std::io::BufWriter::new(
            std::fs::File::create(path).map_err(|e| format!("failed to open '{}': {}", path, e))?,
        )),
        None => Box::new(std::io::stdout()),
    };
    emit_findings(
        format,
        "scythe-lint",
        env!("CARGO_PKG_VERSION"),
        &findings,
        out.as_mut(),
    )?;
    out.flush().ok();

    if violations.is_empty() {
        eprintln!("No lint violations found.");
        return Ok(());
    }

    let mut error_count = 0usize;
    let mut warning_count = 0usize;

    let mut current_file: Option<&str> = None;
    for v in violations {
        let file = v.file.as_str();
        if current_file != Some(file) {
            if !file.is_empty() {
                eprintln!("\n{}:", file);
            }
            current_file = Some(file);
        }

        let severity_str = match v.severity {
            Severity::Error => {
                error_count += 1;
                "error"
            }
            Severity::Warn => {
                warning_count += 1;
                "warning"
            }
            Severity::Off => continue,
        };

        let source_tag = match v.source.as_deref() {
            Some(s) if !s.is_empty() => format!("[{}] ", s),
            _ => String::new(),
        };

        let location = match (v.line_no, v.line_pos) {
            (Some(line), Some(pos)) => format!("{}:{}", line, pos),
            _ => match &v.query_name {
                Some(name) => format!("query:{}", name),
                None => String::new(),
            },
        };

        if location.is_empty() {
            eprintln!("  {}{}: [{}] {}", source_tag, severity_str, v.rule_id, v.message);
        } else {
            eprintln!(
                "  {} {}{}: [{}] {}",
                location, source_tag, severity_str, v.rule_id, v.message
            );
        }
    }

    eprintln!();
    if error_count > 0 {
        eprintln!("lint: {} error(s), {} warning(s)", error_count, warning_count);
        if !exit_zero {
            std::process::exit(2);
        }
        Ok(())
    } else {
        if warning_count > 0 {
            eprintln!("lint: {} warning(s)", warning_count);
        }
        Ok(())
    }
}
