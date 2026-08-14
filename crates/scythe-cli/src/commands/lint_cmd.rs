use std::borrow::Cow;
use std::path::Path;

use scythe_inspect::parse_inspect_section;
use scythe_lint::reporters::{Finding, Format};
use scythe_lint::sqruff_adapter;
use scythe_lint::types::Severity;
use scythe_lint::{SuppressionSet, emit_findings};

use super::inspect::{build_driver_with_config, build_registry};
use super::shared::{
    ParseSeverities, config_dir, engine_to_sqruff_dialect, resolve_globs, split_query_file, validate_dialect,
};

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
    /// CWE ids for the rule that produced this violation, taken from
    /// [`scythe_lint::LintRule::cwe`] at the point the rule set is known.
    /// Empty for sqruff, inspect, and parse/analyze failures, none of which
    /// map to a CWE.
    cwe: Vec<String>,
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

/// Resolve a `[[sql]] engine = "..."` value to a [`scythe_core::dialect::SqlDialect`],
/// treating an empty string (the field's default when the key is omitted) as
/// `"postgresql"` -- scythe's own documented default -- and rejecting
/// anything else [`scythe_lint::parse_engine_dialect`] does not recognize.
///
/// Before this, both call sites below used
/// `SqlDialect::from_str(engine).unwrap_or(SqlDialect::PostgreSQL)`: a typo'd
/// engine (`mysql8`) was silently analyzed as PostgreSQL -- wrong catalog
/// parsing, wrong dialect-gated rule set -- with `scythe lint` reporting
/// success regardless (#165, item 3).
fn resolve_sql_dialect(engine: &str) -> Result<scythe_core::dialect::SqlDialect, String> {
    let engine = if engine.is_empty() { "postgresql" } else { engine };
    scythe_lint::parse_engine_dialect(engine)
}

/// Schema-aware context for running scythe-native lint rules against
/// explicitly-listed files, built from `scythe.toml` when one is present and
/// declares a resolvable schema. See [`load_native_lint_context`].
struct NativeLintContext {
    engine: scythe_lint::LintEngine,
    catalog: scythe_core::catalog::Catalog,
    sql_dialect: scythe_core::dialect::SqlDialect,
    /// Effective `SC-PARSE01`/`SC-PARSE02` severities for this run, resolved
    /// from the same `[lint]` table applied to `engine`'s registry -- see
    /// [`ParseSeverities`].
    parse_severities: ParseSeverities,
}

/// Build a [`NativeLintContext`] from `scythe.toml` at `config_path`, when
/// one is present and its first `[[sql]]` block has a resolvable schema.
///
/// Returns `Ok(None)` -- not an error -- when the config file does not
/// exist, mirroring `dialect_from_config`'s tolerance for a missing config:
/// explicit-file mode is valid with no config at all (pure sqruff formatting
/// of arbitrary SQL), and that must keep working exactly as it did before
/// this function existed. It also returns `Ok(None)` when a config exists
/// but declares no schema for its first block, for the same reason: no
/// schema means no catalog to analyze against.
///
/// The *first* `[[sql]]` block is used, not every block: explicit-file mode
/// gives no indication which block (if any) a given file belongs to, so
/// there is no principled way to pick a schema per file. This matches the
/// same first-block fallback `dialect_from_config` and the inspect-engine
/// default already use elsewhere in this module -- on a single-engine
/// project (the common case) it is exactly right, and on a mixed-engine
/// project it is the best available default rather than running no native
/// rules at all.
///
/// Before this function existed, explicit-file mode never attempted to
/// build a catalog or a [`scythe_lint::LintEngine`] at all -- the call site
/// below hardcoded a comment saying so -- so `scythe lint foo.sql` ran zero
/// scythe-native rules even when `scythe.toml` (and its schema) sat right
/// next to `foo.sql`: silently checking far less than a bare `scythe lint`
/// run against the same project.
fn load_native_lint_context(config_path: &str) -> Result<Option<NativeLintContext>, Box<dyn std::error::Error>> {
    use scythe_core::catalog::Catalog;
    use scythe_lint::{LintEngine, default_registry};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct MinConfig {
        #[serde(default)]
        sql: Vec<MinSqlConfig>,
        #[serde(default)]
        lint: Option<scythe_lint::types::LintConfig>,
    }

    #[derive(Deserialize)]
    struct MinSqlConfig {
        #[serde(default)]
        engine: Option<String>,
        #[serde(default)]
        schema: Vec<String>,
    }

    let config_str = match std::fs::read_to_string(config_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("failed to read config '{}': {}", config_path, e).into()),
    };
    let config: MinConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let Some(first) = config.sql.first() else {
        return Ok(None);
    };

    let base_dir = config_dir(config_path);
    let schema_files = resolve_globs(&first.schema, base_dir, "[native lint] schema")?;
    if schema_files.is_empty() {
        return Ok(None);
    }

    let schema_contents: Vec<String> = schema_files
        .iter()
        .map(|p| std::fs::read_to_string(p).map_err(|e| format!("failed to read schema file '{}': {}", p, e)))
        .collect::<Result<_, _>>()?;
    let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

    let sql_dialect =
        resolve_sql_dialect(first.engine.as_deref().unwrap_or("")).map_err(|e| format!("[native lint] {}", e))?;
    let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &sql_dialect)?;

    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }
    let engine = LintEngine::new(registry);

    // Same `[lint]` table, applied to `SC-PARSE01`/`SC-PARSE02`'s own
    // registry -- see `ParseSeverities` for why a query that fails to parse
    // or analyze cannot pick up severity through `registry` above.
    let mut parse_rules = scythe_lint::parse_registry();
    if let Some(ref lint_config) = config.lint {
        parse_rules.apply_config(lint_config);
    }
    let parse_severities = ParseSeverities::from_registry(&parse_rules);

    Ok(Some(NativeLintContext {
        engine,
        catalog,
        sql_dialect,
        parse_severities,
    }))
}

/// Read `[lint.sqruff]` out of `scythe.toml` for explicit-file lint mode, when a
/// config file is present.
///
/// Mirrors `fmt.rs`'s function of the same name and for the same reason (#206):
/// returns `Ok(None)` -- not an error -- when `config_path` does not exist, since
/// `scythe lint some/file.sql` with no `scythe.toml` at all is a valid, unaffected
/// mode. Only the top-level `[lint]` table is required; unlike
/// [`load_native_lint_context`], this has no need of `[[sql]]` or a schema, since
/// `[lint.sqruff]` is not scoped to a block.
fn sqruff_config_from_config(config_path: &str) -> Result<Option<scythe_lint::types::SqruffConfig>, String> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct MinConfig {
        #[serde(default)]
        lint: Option<scythe_lint::types::LintConfig>,
    }

    let config_str = match std::fs::read_to_string(config_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("failed to read config '{config_path}': {e}")),
    };
    let config: MinConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{config_path}': {e}"))?;

    Ok(config.lint.and_then(|lc| lc.sqruff))
}

/// Lint specific files using sqruff, plus scythe-native rules when
/// `scythe.toml` (and a resolvable schema) is present alongside them -- see
/// [`load_native_lint_context`].
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
    use scythe_core::analyzer::analyze;
    use scythe_core::parser::parse_query_with_dialect;
    use scythe_lint::LintContext;

    let mut all_violations: Vec<FileViolation> = Vec::new();

    // One linter for the whole file list, not one per file: building it
    // compiles the dialect's lexer, which is what a multi-file run actually
    // spends its time on (#130). `sqruff_config_from_config` mirrors
    // `fmt.rs`'s own fix for the same gap: explicit-file mode used to always
    // pass `None` here, silently dropping a user's `[lint.sqruff]` rule
    // table whenever files were named directly on the command line instead
    // of resolved from `scythe.toml`.
    let sqruff_config = sqruff_config_from_config(config_path)?;
    let linter = sqruff_adapter::SqruffLinter::new(dialect, sqruff_config.as_ref())
        .map_err(|e| format!("sqruff rejected dialect '{}': {}", dialect, e))?;

    // scythe-native rules, run alongside sqruff when a schema is available
    // to build a catalog from -- see `load_native_lint_context`'s doc
    // comment for why explicit-file mode previously ran none of these at
    // all, unlike `lint_from_config`.
    let native = load_native_lint_context(config_path)?;

    /// One query that parsed and analyzed against the native context's
    /// catalog, kept alive so `LintEngine::build_report` can borrow every
    /// `LintContext` at once -- mirrors `lint_from_config`'s own
    /// `PreparedQuery`.
    struct PreparedQuery {
        file: String,
        query: scythe_core::parser::Query,
        analyzed: scythe_core::analyzer::AnalyzedQuery,
        suppressions: SuppressionSet,
    }
    let mut prepared: Vec<PreparedQuery> = Vec::new();

    for path in files {
        let sql = std::fs::read_to_string(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

        if fix {
            let (_pre_fix_violations, fixed) = linter
                .lint_and_fix(&sql)
                .map_err(|e| format!("sqruff error on '{}': {}", path, e))?;
            if fixed != sql {
                std::fs::write(path, &fixed).map_err(|e| format!("failed to write '{}': {}", path, e))?;
                eprintln!("fixed {}", path);

                // Re-lint the fixed content rather than reporting the
                // pre-fix violation list: those violations were just
                // resolved by the write above, and reporting them as
                // current findings fails CI on issues that no longer exist
                // (#210).
                let remaining = linter
                    .lint(&fixed)
                    .map_err(|e| format!("sqruff error on '{}': {}", path, e))?;
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
                        cwe: Vec::new(),
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
                        cwe: Vec::new(),
                    });
                }
            }
        } else {
            let violations = linter
                .lint(&sql)
                .map_err(|e| format!("sqruff error on '{}': {}", path, e))?;
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
                    cwe: Vec::new(),
                });
            }
        }

        if let Some(ctx) = &native {
            // A file with no `-- name:` / `-- @name` annotation yields zero
            // blocks here and is silently skipped for native analysis --
            // unlike `lint_from_config`, this is not treated as a hard
            // error: explicit-file mode's whole purpose is linting
            // arbitrary SQL (a bare schema file, a snippet), and it must
            // keep doing that even when a `scythe.toml` happens to sit
            // alongside it.
            for block in split_query_file(&sql) {
                match parse_query_with_dialect(&block, &ctx.sql_dialect) {
                    Ok(parsed) => match analyze(&ctx.catalog, &parsed) {
                        Ok(analyzed) => prepared.push(PreparedQuery {
                            file: path.clone(),
                            suppressions: SuppressionSet::parse(&block),
                            query: parsed,
                            analyzed,
                        }),
                        Err(e) => {
                            let name = parsed.annotations.name.clone();
                            all_violations.push(FileViolation {
                                file: path.clone(),
                                query_name: Some(name.clone()),
                                rule_id: Cow::Borrowed("SC-PARSE02"),
                                severity: ctx.parse_severities.unanalyzable,
                                message: format!(
                                    "failed to analyze query '{name}': {e} — no lint rule could examine it"
                                ),
                                line_no: None,
                                line_pos: None,
                                source: None,
                                cwe: Vec::new(),
                            });
                        }
                    },
                    Err(e) => {
                        all_violations.push(FileViolation {
                            file: path.clone(),
                            query_name: None,
                            rule_id: Cow::Borrowed("SC-PARSE01"),
                            severity: ctx.parse_severities.unparseable,
                            message: format!("failed to parse query: {e} — no lint rule could examine it"),
                            line_no: None,
                            line_pos: None,
                            source: None,
                            cwe: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    if let Some(ctx) = native {
        // `build_report`, not a hand-rolled `check_query` loop: the
        // engine's own report is the one place that also runs the
        // cross-query checks (duplicate `@name` detection, SC-C03),
        // matching `lint_from_config`.
        let contexts: Vec<LintContext<'_>> = prepared
            .iter()
            .map(|p| LintContext {
                sql: &p.query.sql,
                stmt: &p.query.stmt,
                analyzed: &p.analyzed,
                catalog: &ctx.catalog,
                annotations: &p.query.annotations,
                dialect: ctx.sql_dialect,
            })
            .collect();
        let report = ctx.engine.build_report(contexts.into_iter(), &ctx.catalog);

        eprintln!(
            "[native] checked {} query(ies) against {} active rule(s)",
            report.queries_checked, report.rules_active
        );

        let mut prepared_by_name: ahash::AHashMap<&str, usize> = ahash::AHashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            prepared_by_name.entry(p.analyzed.name.as_str()).or_insert(i);
        }

        for violation in report.violations {
            let origin = prepared_by_name
                .get(violation.query_name.as_str())
                .map(|&i| &prepared[i]);

            // Honour the same `-- scythe-audit: ignore[ID]` annotations
            // `lint_from_config` and `scythe audit` honour -- see that
            // function's identical check for why the suppression index is
            // always 0.
            if let Some(origin) = origin
                && origin.suppressions.is_suppressed(&violation.rule_id, 0)
            {
                continue;
            }

            all_violations.push(FileViolation {
                file: origin.map(|p| p.file.clone()).unwrap_or_default(),
                query_name: (!violation.query_name.is_empty()).then(|| violation.query_name.clone()),
                rule_id: violation.rule_id,
                severity: violation.severity,
                message: violation.message,
                line_no: None,
                line_pos: None,
                source: None,
                cwe: Vec::new(),
            });
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

    /// One query that parsed and analyzed, kept alive for the whole
    /// `[[sql]]` block so [`LintEngine::build_report`] can borrow every
    /// [`LintContext`] at once -- it needs the full set in one call to make
    /// its cross-query checks (duplicate `@name`, SC-C03).
    struct PreparedQuery {
        file: String,
        query: scythe_core::parser::Query,
        analyzed: scythe_core::analyzer::AnalyzedQuery,
        /// Suppressions parsed from this query's own block text. A block is
        /// one statement, so the only meaningful statement index is 0 -- see
        /// where this is consulted for why the index, not the source line,
        /// is the key.
        suppressions: SuppressionSet,
    }

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

    // Same `[lint]` table applied to `SC-PARSE01`/`SC-PARSE02`'s own
    // registry below -- see `ParseSeverities` for why these two cannot pick
    // up severity through `registry` the way a SQL rule does.
    let mut parse_rules = scythe_lint::parse_registry();
    if let Some(ref lint_config) = config.lint {
        parse_rules.apply_config(lint_config);
    }
    let parse_severities = ParseSeverities::from_registry(&parse_rules);

    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }

    // Both snapshots are taken before the registry moves into the engine,
    // which consumes it.
    //
    // `cwe_by_rule` replaces a regex scrape of the violation *message*. No
    // rule message template mentions a CWE id -- they are declared on the
    // rule (`cwe = ["CWE-200"]`) and repeated only in its description -- so
    // the scrape returned an empty list for every violation lint has ever
    // reported and `--format sarif` carried no CWE tags at all.
    let cwe_by_rule: ahash::AHashMap<String, Vec<String>> = registry
        .all_rules()
        .iter()
        .map(|(rule, _)| (rule.id().to_string(), rule.cwe()))
        .collect();

    // Resolved once, per block, before any rule-applicability or catalog
    // work: `resolve_sql_dialect` errors on an engine `SqlDialect::from_str`
    // does not recognize instead of silently defaulting to PostgreSQL
    // (#165, item 3), and every later use of a block's dialect (the
    // applicability filter just below, and the catalog build in the loop)
    // reuses this same resolved value rather than re-parsing `sc.engine` and
    // risking the two falling out of sync.
    let block_dialects: Vec<SqlDialect> = config
        .sql
        .iter()
        .map(|sc| resolve_sql_dialect(&sc.engine).map_err(|e| format!("[{}] {}", sc.name, e)))
        .collect::<Result<_, _>>()?;

    // Rules that will not fire for a `[[sql]]` block because
    // `LintRule::is_applicable_to` excludes the block's engine -- most
    // canonical security and migration rules declare `dialects =
    // ["postgres"]`. `MatcherRule::check_query` applies the same gate
    // internally and returns an empty `Vec`, which is indistinguishable from
    // "the rule ran and found nothing"; reporting the ids here is what makes
    // the difference visible (#167). Indexed in `config.sql` order.
    let inapplicable_rules_per_block: Vec<Vec<&'static str>> = block_dialects
        .iter()
        .map(|&block_dialect| {
            registry
                .active_rules()
                .iter()
                .filter(|(rule, _)| !rule.is_applicable_to(block_dialect))
                .map(|(rule, _)| rule.id())
                .collect()
        })
        .collect();

    let engine = LintEngine::new(registry);

    let sqruff_config = config.lint.as_ref().and_then(|lc| lc.sqruff.as_ref());

    let mut all_violations: Vec<FileViolation> = Vec::new();

    let base_dir = config_dir(config_path);

    for (block_index, sql_config) in config.sql.iter().enumerate() {
        eprintln!("[{}] Parsing schema...", sql_config.name);

        let inapplicable = inapplicable_rules_per_block
            .get(block_index)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if !inapplicable.is_empty() {
            let engine_label = if sql_config.engine.is_empty() {
                "postgres"
            } else {
                sql_config.engine.as_str()
            };
            eprintln!(
                "[{}] {} rule(s) skipped: not applicable to engine '{}' ({})",
                sql_config.name,
                inapplicable.len(),
                engine_label,
                inapplicable.join(", ")
            );
        }

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

        // One linter for the whole `[[sql]]` block, built before any query
        // file is read. Two reasons, and they are the same reason:
        //
        // - Construction is what is expensive. It compiles the dialect's
        //   lexer; the per-file calls this replaces paid that cost once per
        //   query file, which on a large project is most of what `scythe
        //   lint` does (#130).
        // - Construction is what validates. `SqruffLinter` probes the
        //   assembled configuration as it is built, so a bad `[lint.sqruff]`
        //   fails here, naming the block, rather than as an error against
        //   whichever query file happened to be read first -- an error that
        //   aborts the run and discards every scythe-native finding already
        //   collected, security rules included.
        //
        // `for_linting`, not `new`: under `enabled = false` the lint path
        // never reaches sqruff, so a configuration it will never read must
        // not fail the run. `None` is that case, and the call sites below
        // reproduce what the disabled path returned -- no violations, and
        // the query file left untouched.
        let sqruff_linter = sqruff_adapter::SqruffLinter::for_linting(sqruff_dialect, sqruff_config)
            .map_err(|e| format!("[{}] invalid [lint.sqruff] configuration: {}", sql_config.name, e))?;

        let schema_files = resolve_globs(&sql_config.schema, base_dir, &format!("[{}] schema", sql_config.name))?;
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| std::fs::read_to_string(p).map_err(|e| format!("failed to read schema file '{}': {}", p, e)))
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

        let sql_dialect = block_dialects[block_index];
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &sql_dialect)?;

        let query_files = resolve_globs(&sql_config.queries, base_dir, &format!("[{}] queries", sql_config.name))?;

        // Every query in this block, held until the whole block has been
        // read: `build_report` takes the complete set in one call so it can
        // run its cross-query checks. Reading the file with the sqruff pass
        // and linting it immediately, as this used to, made those checks
        // unreachable.
        let mut prepared: Vec<PreparedQuery> = Vec::new();

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
                let (pre_fix_violations, fixed) = match &sqruff_linter {
                    Some(linter) => linter
                        .lint_and_fix(&content)
                        .map_err(|e| format!("sqruff error on '{}': {}", query_file, e))?,
                    None => (Vec::new(), content.clone()),
                };
                if fixed != content {
                    std::fs::write(query_file, &fixed)
                        .map_err(|e| format!("failed to write '{}': {}", query_file, e))?;
                    eprintln!("fixed {}", query_file);

                    // Re-lint the fixed content: `pre_fix_violations` names
                    // violations the write above just resolved, and
                    // reporting them as current findings fails CI on issues
                    // that no longer exist (#210).
                    // Reachable only when the content changed, which only a
                    // present linter can do.
                    let remaining = match &sqruff_linter {
                        Some(linter) => linter
                            .lint(&fixed)
                            .map_err(|e| format!("sqruff error on '{}': {}", query_file, e))?,
                        None => Vec::new(),
                    };
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
                            cwe: Vec::new(),
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
                            cwe: Vec::new(),
                        });
                    }
                }
            } else {
                let sq_violations = match &sqruff_linter {
                    Some(linter) => linter
                        .lint(&content)
                        .map_err(|e| format!("sqruff error on '{}': {}", query_file, e))?,
                    None => Vec::new(),
                };
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
                        cwe: Vec::new(),
                    });
                }
            }

            let blocks = split_query_file(&content);
            for block in &blocks {
                // A parse or analyze failure is an error-severity finding,
                // not a `continue`. Skipping meant every query-level rule --
                // security, safety, performance, naming -- was silently
                // dropped for that query while lint went on to print "No
                // lint violations found." and exit 0, so a CI gate on
                // `scythe lint` read an unanalysable project as clean even
                // though `scythe check` on the same project exited 1 (#158).
                // The rule ids match the ones `scythe check` already uses for
                // the same two failures, so the two commands agree.
                let parsed = match parse_query_with_dialect(block, &sql_dialect) {
                    Ok(p) => p,
                    Err(e) => {
                        all_violations.push(FileViolation {
                            file: query_file.clone(),
                            query_name: None,
                            rule_id: Cow::Borrowed("SC-PARSE01"),
                            severity: parse_severities.unparseable,
                            message: format!("failed to parse query: {e} — no lint rule could examine it"),
                            line_no: None,
                            line_pos: None,
                            source: None,
                            cwe: Vec::new(),
                        });
                        continue;
                    }
                };
                let analyzed = match analyze(&catalog, &parsed) {
                    Ok(a) => a,
                    Err(e) => {
                        let name = parsed.annotations.name.clone();
                        all_violations.push(FileViolation {
                            file: query_file.clone(),
                            query_name: Some(name.clone()),
                            rule_id: Cow::Borrowed("SC-PARSE02"),
                            severity: parse_severities.unanalyzable,
                            message: format!("failed to analyze query '{name}': {e} — no lint rule could examine it"),
                            line_no: None,
                            line_pos: None,
                            source: None,
                            cwe: Vec::new(),
                        });
                        continue;
                    }
                };

                prepared.push(PreparedQuery {
                    file: query_file.clone(),
                    suppressions: SuppressionSet::parse(block),
                    query: parsed,
                    analyzed,
                });
            }
        }

        // `build_report`, not a hand-rolled `check_query` loop plus a
        // separate `check_catalog` call: the engine's own report is the one
        // place that also runs the cross-query checks -- duplicate `@name`
        // detection (SC-C03), routed through the registry's configured
        // severity. Deriving the same walk here left `build_report` with no
        // caller at all and left `scythe lint` unable to report a duplicate
        // query name that `scythe check` fails on.
        let contexts: Vec<LintContext<'_>> = prepared
            .iter()
            .map(|p| LintContext {
                sql: &p.query.sql,
                stmt: &p.query.stmt,
                analyzed: &p.analyzed,
                catalog: &catalog,
                annotations: &p.query.annotations,
                dialect: sql_dialect,
            })
            .collect();
        let report = engine.build_report(contexts.into_iter(), &catalog);

        // Printed unconditionally so an empty rule set (every rule
        // configured `off`) or an empty query set cannot look like a clean
        // project: "0 query(ies) against 0 active rule(s)" says plainly that
        // nothing was verified.
        eprintln!(
            "[{}] checked {} query(ies) against {} active rule(s)",
            sql_config.name, report.queries_checked, report.rules_active
        );

        // `build_report` reports a query by name; lint reports it by file.
        // First occurrence wins, which matters only for a duplicate name --
        // and in that case no single file is the right answer anyway.
        let mut prepared_by_name: ahash::AHashMap<&str, usize> = ahash::AHashMap::new();
        for (i, p) in prepared.iter().enumerate() {
            prepared_by_name.entry(p.analyzed.name.as_str()).or_insert(i);
        }

        for violation in report.violations {
            // Catalog-level violations carry no query name, so they cannot
            // be attributed to a file -- matching what this reported before.
            let origin = prepared_by_name
                .get(violation.query_name.as_str())
                .map(|&i| &prepared[i]);

            // Honour the same `-- scythe-audit: ignore[ID]` annotations
            // `scythe audit` honours, keyed by statement index exactly as
            // `SuppressionSet` documents: the set is parsed from the query's
            // own block, which is a single statement, so the index is 0.
            // Keying by source line instead over-suppresses -- two
            // statements sharing one physical line resolve to the same entry
            // (scythe-lint #140).
            if let Some(origin) = origin
                && origin.suppressions.is_suppressed(&violation.rule_id, 0)
            {
                continue;
            }

            let cwe = cwe_by_rule.get(violation.rule_id.as_ref()).cloned().unwrap_or_default();
            all_violations.push(FileViolation {
                file: origin.map(|p| p.file.clone()).unwrap_or_default(),
                query_name: (!violation.query_name.is_empty()).then(|| violation.query_name.clone()),
                rule_id: violation.rule_id,
                severity: violation.severity,
                message: violation.message,
                line_no: None,
                line_pos: None,
                source: None,
                cwe,
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
            cwe: Vec::new(),
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
            cwe: v.cwe.clone(),
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
