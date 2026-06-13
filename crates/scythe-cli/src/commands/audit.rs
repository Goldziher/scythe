//! `scythe audit` — run security-category lint rules with security-flavoured
//! defaults (all-on, error-by-default) and emit findings in human / SARIF /
//! JSON format.
//!
//! Supports:
//! - Inline `-- scythe-audit: ignore[ID]` suppression annotations.
//! - User-supplied rules via `[[audit.rule]]` in `scythe.toml` and optional
//!   `extra_rules = [...]` TOML files.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use scythe_core::analyzer::AnalyzedQuery;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::Annotations;
use scythe_lint::reporters::{Finding, Format};
use scythe_lint::{
    AuditConfigError, LintContext, LintRule, MatcherRegistry, RuleCategory, RuleRegistry, RuleSpec,
    Severity, SuppressionSet, default_registry, emit_findings, extract_cwe, load_rules_from_file,
    register_user_rules,
};

use super::shared::resolve_globs;

const TOOL_NAME: &str = "scythe-audit";
const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Inputs to `run_audit`. Mirrors the clap `Commands::Audit` shape so the CLI
/// layer can forward fields by name instead of long positional argument lists.
pub struct RunAuditOpts {
    pub config_path: String,
    pub format: String,
    pub list_rules: bool,
    pub explain: Option<String>,
    pub severity: Option<String>,
    pub exit_zero: bool,
    pub output: Option<String>,
    pub ignore_suppressions: bool,
    pub dialect: Option<String>,
    pub files: Vec<String>,
}

pub fn run_audit(opts: RunAuditOpts) -> Result<(), Box<dyn std::error::Error>> {
    // ------------------------------------------------------------------
    // Discovery flags: --list-rules / --explain operate on the rule
    // catalog (including any user rules loaded via scythe.toml) and exit
    // before any SQL is audited.
    // ------------------------------------------------------------------
    if opts.list_rules || opts.explain.is_some() {
        let registry = load_registry_for_discovery(&opts.config_path)?;
        let mut out: Box<dyn Write> = open_output(opts.output.as_deref())?;
        if opts.list_rules {
            print_rule_catalog(&registry, out.as_mut())?;
            return Ok(());
        }
        if let Some(id) = &opts.explain {
            print_rule_explanation(&registry, id, out.as_mut())?;
            return Ok(());
        }
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

    let mut findings: Vec<Finding> = Vec::new();

    if opts.files.is_empty() {
        findings.extend(audit_from_config(
            &opts.config_path,
            opts.ignore_suppressions,
        )?);
    } else {
        let engine = opts.dialect.as_deref().unwrap_or("postgres");
        findings.extend(audit_explicit_files(
            &opts.files,
            engine,
            opts.ignore_suppressions,
        )?);
    }

    if let Some(floor) = severity_floor {
        findings.retain(|f| f.severity >= floor);
    }

    let mut out: Box<dyn Write> = open_output(opts.output.as_deref())?;
    emit_findings(format, TOOL_NAME, TOOL_VERSION, &findings, out.as_mut())?;
    // Ensure file-backed writers flush before we potentially exit.
    out.flush().ok();

    let error_count = findings
        .iter()
        .filter(|f| matches!(f.severity, Severity::Error))
        .count();
    if error_count > 0 && !opts.exit_zero {
        // Distinct exit code so CI can tell apart "lint warnings" from
        // "security violations". 2 = audit failure.
        std::process::exit(2);
    }
    Ok(())
}

/// Open the output sink for a reporter. `None` → stdout. `Some(path)` →
/// truncating file write. Parent directory must already exist.
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

/// Build a registry for discovery commands (`--list-rules` / `--explain`).
/// Loads canonical rules plus any user rules from `scythe.toml` if it exists.
/// The registry is never executed; severity overrides are applied so the
/// catalog reflects the user's effective configuration.
fn load_registry_for_discovery(
    config_path: &str,
) -> Result<RuleRegistry, Box<dyn std::error::Error>> {
    let mut registry = default_registry();
    if !Path::new(config_path).exists() {
        return Ok(registry);
    }
    let config_str = std::fs::read_to_string(config_path)?;
    let parsed: toml::Value = toml::from_str(&config_str)?;
    // Apply [lint] severity overrides if present.
    if let Some(lint_section) = parsed.get("lint")
        && let Ok(lint_config) = lint_section
            .clone()
            .try_into::<scythe_lint::types::LintConfig>()
    {
        registry.apply_config(&lint_config);
    }
    // Load user rules from [audit] section if present.
    if let Some(audit_section) = parsed.get("audit") {
        let config_dir = Path::new(config_path).parent().unwrap_or(Path::new("."));
        let matcher_registry = MatcherRegistry::canonical();
        let mut user_specs: Vec<(RuleSpec, String)> = Vec::new();
        if let Some(rules) = audit_section.get("rule").and_then(|v| v.as_array()) {
            for r in rules {
                if let Ok(spec) = r.clone().try_into::<RuleSpec>() {
                    user_specs.push((spec, config_path.to_string()));
                }
            }
        }
        if let Some(extras) = audit_section.get("extra_rules").and_then(|v| v.as_array()) {
            for v in extras {
                if let Some(rel_path) = v.as_str() {
                    let abs_path = config_dir.join(rel_path);
                    let path_str = abs_path.display().to_string();
                    let specs = load_rules_from_file(&abs_path)
                        .map_err(|e: AuditConfigError| e.to_string())?;
                    for spec in specs {
                        user_specs.push((spec, path_str.clone()));
                    }
                }
            }
        }
        if !user_specs.is_empty() {
            register_user_rules(&mut registry, &matcher_registry, &user_specs)
                .map_err(|e: AuditConfigError| e.to_string())?;
        }
    }
    Ok(registry)
}

/// Print the rule catalog, grouped by category, in a fixed-width table.
pub(crate) fn print_rule_catalog(
    registry: &RuleRegistry,
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rows: Vec<(&dyn LintRule, Severity)> = registry.active_rules();
    rows.sort_by_key(|(r, _)| (r.category() as u8, r.id()));

    let id_w = rows
        .iter()
        .map(|(r, _)| r.id().len())
        .max()
        .unwrap_or(2)
        .max(2);
    let name_w = rows
        .iter()
        .map(|(r, _)| r.name().len())
        .max()
        .unwrap_or(4)
        .max(4);

    let mut current_category: Option<RuleCategory> = None;
    for (rule, sev) in &rows {
        let cat = rule.category();
        if Some(cat) != current_category {
            if current_category.is_some() {
                writeln!(out)?;
            }
            writeln!(out, "[{}]", cat)?;
            current_category = Some(cat);
        }
        writeln!(
            out,
            "  {id:<id_w$}  {name:<name_w$}  {sev:<5}  {desc}",
            id = rule.id(),
            name = rule.name(),
            sev = severity_label(*sev),
            desc = rule.description(),
            id_w = id_w,
            name_w = name_w,
        )?;
    }
    Ok(())
}

/// Print the full description and CWE references for a rule, looked up by id.
pub(crate) fn print_rule_explanation(
    registry: &RuleRegistry,
    rule_id: &str,
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let rules = registry.active_rules();
    let Some((rule, sev)) = rules.iter().find(|(r, _)| r.id() == rule_id) else {
        return Err(format!(
            "no rule with id '{}' — try `scythe audit --list-rules`",
            rule_id
        )
        .into());
    };
    writeln!(out, "{} — {}", rule.id(), rule.name())?;
    writeln!(out, "  category: {}", rule.category())?;
    writeln!(out, "  severity: {}", severity_label(*sev))?;
    let cwes = extract_cwe(rule.description());
    if !cwes.is_empty() {
        writeln!(out, "  cwe:      {}", cwes.join(", "))?;
    }
    writeln!(out)?;
    writeln!(out, "{}", rule.description())?;
    Ok(())
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Off => "off",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}

fn audit_from_config(
    config_path: &str,
    ignore_suppressions: bool,
) -> Result<Vec<Finding>, Box<dyn std::error::Error>> {
    use serde::Deserialize;

    #[derive(Deserialize, Default)]
    struct AuditConfig {
        #[serde(default)]
        extra_rules: Vec<String>,
        #[serde(default, rename = "rule")]
        rules: Vec<RuleSpec>,
    }

    #[derive(Deserialize)]
    struct ScytheConfig {
        sql: Vec<SqlConfig>,
        #[serde(default)]
        lint: Option<scythe_lint::types::LintConfig>,
        #[serde(default)]
        audit: AuditConfig,
    }

    #[derive(Deserialize)]
    struct SqlConfig {
        name: String,
        schema: Vec<String>,
        queries: Vec<String>,
        #[serde(default)]
        engine: String,
    }

    if !Path::new(config_path).exists() {
        return Err(format!("no files specified and config '{}' not found", config_path).into());
    }

    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig = toml::from_str(&config_str)
        .map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }

    // ------------------------------------------------------------------
    // User-supplied rules
    // ------------------------------------------------------------------
    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let matcher_registry = MatcherRegistry::canonical();

    // Collect (spec, source) pairs from inline [[audit.rule]] stanzas.
    let mut user_specs: Vec<(RuleSpec, String)> = config
        .audit
        .rules
        .into_iter()
        .map(|spec| (spec, config_path.to_string()))
        .collect();

    // Collect specs from extra_rules files.
    for rel_path in &config.audit.extra_rules {
        let abs_path = config_dir.join(rel_path);
        let path_str = abs_path.display().to_string();
        let specs = load_rules_from_file(&abs_path).map_err(|e: AuditConfigError| e.to_string())?;
        for spec in specs {
            user_specs.push((spec, path_str.clone()));
        }
    }

    if !user_specs.is_empty() {
        register_user_rules(&mut registry, &matcher_registry, &user_specs)
            .map_err(|e: AuditConfigError| e.to_string())?;
    }

    let rules = registry.active_rules();

    let mut findings = Vec::new();

    for sql_config in &config.sql {
        let sql_dialect =
            SqlDialect::from_str(&sql_config.engine).unwrap_or(SqlDialect::PostgreSQL);

        let schema_files = resolve_globs(&sql_config.schema)?;
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| {
                std::fs::read_to_string(p)
                    .map_err(|e| format!("failed to read schema file '{}': {}", p, e))
            })
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &sql_dialect)?;

        // Run security rules against schema files (DDL: GRANT, CREATE FUNCTION, etc.)
        for (path, content) in schema_files.iter().zip(schema_contents.iter()) {
            findings.extend(run_security_rules_over_sql(
                path,
                content,
                &sql_dialect,
                &catalog,
                &rules,
                ignore_suppressions,
            ));
        }

        // Run security rules against query files (DML).
        let query_files = resolve_globs(&sql_config.queries)?;
        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;
            findings.extend(run_security_rules_over_sql(
                query_file,
                &content,
                &sql_dialect,
                &catalog,
                &rules,
                ignore_suppressions,
            ));
        }

        eprintln!(
            "[{}] audited {} schema file(s), {} query file(s)",
            sql_config.name,
            schema_files.len(),
            query_files.len()
        );
    }

    Ok(findings)
}

pub(crate) fn audit_explicit_files(
    files: &[String],
    engine: &str,
    ignore_suppressions: bool,
) -> Result<Vec<Finding>, Box<dyn std::error::Error>> {
    let sql_dialect = SqlDialect::from_str(engine).unwrap_or(SqlDialect::PostgreSQL);

    // No schema context — security rules don't strictly need a populated
    // catalog (none of the Phase 1 rules consult catalog tables).
    let catalog = Catalog::from_ddl_with_dialect(&[], &sql_dialect).unwrap_or_else(|_| {
        Catalog::from_ddl_with_dialect(&[], &SqlDialect::PostgreSQL).expect("empty catalog")
    });

    let registry = default_registry();
    let rules = registry.active_rules();

    let mut findings = Vec::new();
    for path in files {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read '{}': {}", path, e))?;
        findings.extend(run_security_rules_over_sql(
            path,
            &content,
            &sql_dialect,
            &catalog,
            &rules,
            ignore_suppressions,
        ));
    }
    Ok(findings)
}

/// Parse `sql` statement-by-statement and run every security rule over each.
///
/// # Suppression
///
/// A `SuppressionSet` is built once from the full SQL string. Statement start
/// lines are approximated by scanning the sqlparser token stream: for each
/// parsed statement we find the minimum source-location line number among its
/// tokens. This avoids re-splitting on `;` (which is quote-unsafe) and gives
/// accurate 1-based line numbers even for multi-line statements.
pub(crate) fn run_security_rules_over_sql(
    path: &str,
    sql: &str,
    dialect: &SqlDialect,
    catalog: &Catalog,
    rules: &[(&dyn scythe_lint::LintRule, Severity)],
    ignore_suppressions: bool,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Build suppression set once for the whole file. When --ignore-suppressions
    // is set, we still parse (cheap, and we may surface diagnostics later) but
    // skip the application step below.
    let suppressions = SuppressionSet::parse(sql);

    let parser_dialect = dialect.to_sqlparser_dialect();
    let statements = match sqlparser::parser::Parser::parse_sql(parser_dialect.as_ref(), sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("warning: failed to parse '{}': {}", path, e);
            return findings;
        }
    };

    // Compute per-statement start lines via the tokenizer so we have accurate
    // line info without unsafe `;` splitting. We tokenize once and walk
    // the token stream in parallel with the statement list. Because
    // sqlparser's `Parser` consumes tokens left-to-right, each statement
    // corresponds to a contiguous prefix of the token stream.
    let stmt_start_lines = compute_stmt_start_lines(sql, parser_dialect.as_ref(), statements.len());

    let empty_annotations = Annotations::default();
    let empty_analyzed = AnalyzedQuery::default();

    for (idx, stmt) in statements.iter().enumerate() {
        let stmt_line = stmt_start_lines.get(idx).copied().unwrap_or(1);

        let ctx = LintContext {
            sql,
            stmt,
            analyzed: &empty_analyzed,
            catalog,
            annotations: &empty_annotations,
            dialect: *dialect,
        };

        for (rule, severity) in rules {
            if !matches!(
                rule.category(),
                RuleCategory::Security | RuleCategory::Migration
            ) {
                continue;
            }
            for violation in rule.check_query(&ctx) {
                // Apply suppression: if the rule is suppressed on the statement
                // start line, drop this finding. Skip entirely under
                // --ignore-suppressions.
                if !ignore_suppressions
                    && !suppressions.is_empty()
                    && suppressions.is_suppressed(&violation.rule_id, stmt_line)
                {
                    continue;
                }

                findings.push(Finding {
                    file: path.to_string(),
                    query_name: None,
                    rule_id: violation.rule_id.to_string(),
                    rule_name: Some(rule.name().to_string()),
                    rule_description: Some(rule.description().to_string()),
                    severity: *severity,
                    message: violation.message,
                    // line is tracked internally for suppression but not
                    // emitted to preserve byte-identical baseline output.
                    line: None,
                    column: None,
                    cwe: extract_cwe(rule.description()),
                });
            }
        }
    }

    findings
}

/// Compute the 1-based start line of each parsed statement by tokenizing the
/// SQL and tracking the line at which each statement's first meaningful token
/// appears.
///
/// Strategy: tokenize the full SQL with sqlparser. Walk tokens in order,
/// keeping a running line counter.  For each statement slot (0..n_stmts) we
/// record the line of its first token. Statement boundaries are identified by
/// the `SemiColon` token — each `;` advances the statement index by one.
fn compute_stmt_start_lines(
    sql: &str,
    dialect: &dyn sqlparser::dialect::Dialect,
    n_stmts: usize,
) -> Vec<usize> {
    use sqlparser::tokenizer::{Token, Tokenizer};

    let mut start_lines = vec![1usize; n_stmts];

    if n_stmts == 0 {
        return start_lines;
    }

    let tokens = match Tokenizer::new(dialect, sql).tokenize_with_location() {
        Ok(t) => t,
        // On tokenizer failure fall back to line 1 for every statement.
        Err(_) => return start_lines,
    };

    let mut stmt_idx: usize = 0;
    let mut recorded = false; // have we recorded the first token for stmt_idx?

    for tok_with_span in &tokens {
        let line = tok_with_span.span.start.line as usize;
        let token = &tok_with_span.token;

        match token {
            // Whitespace and comments are not the "first meaningful token".
            Token::Whitespace(_) => continue,
            Token::SemiColon => {
                stmt_idx += 1;
                recorded = false;
                if stmt_idx >= n_stmts {
                    break;
                }
                continue;
            }
            _ => {
                if !recorded {
                    start_lines[stmt_idx] = line;
                    recorded = true;
                }
            }
        }
    }

    start_lines
}
