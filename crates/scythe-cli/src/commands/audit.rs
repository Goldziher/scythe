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
    AuditConfigError, KNOWN_ENGINE_ALIASES, LintContext, LintRule, MatcherRegistry, RuleCategory, RuleRegistry,
    RuleSpec, Severity, SuppressionSet, default_registry, emit_findings, load_rules_from_file, parse_engine_dialect,
    register_user_rules,
};

use super::shared::{config_dir, resolve_globs};

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
    // Every input that can be wrong is validated *before* any output sink is
    // opened -- `--format`/`--severity`/`--dialect`/`--explain`'s rule id --
    // so a bad flag is reported as a bad flag instead of truncating
    // `--output`'s file to 0 bytes and then erroring (#212): `File::create`
    // truncates immediately on open, so opening it ahead of validation
    // destroys any existing report before the error is even known.
    let format = Format::parse(&opts.format)
        .ok_or_else(|| format!("unknown --format '{}' (expected human|sarif|json)", opts.format))?;

    let severity_floor = match opts.severity.as_deref() {
        Some(s) => Some(
            Severity::parse_cli(s).ok_or_else(|| format!("unknown --severity '{}' (expected off|warn|error)", s))?,
        ),
        None => None,
    };

    // Validated even though `--list-rules`/`--explain` do not use it,
    // because clap parses every flag on the same invocation regardless of
    // which subcommand-like mode is active -- `--list-rules --dialect
    // klingon` must still fail rather than silently ignore `--dialect`.
    let dialect = match opts.dialect.as_deref() {
        Some(raw) => Some(validate_audit_dialect(raw)?),
        None => None,
    };

    if opts.list_rules || opts.explain.is_some() {
        let registry = load_registry(&opts.config_path)?;

        // Validated before `open_output` opens (and, for a file path,
        // truncates) the sink: `--explain <typo'd-id> --output report.json`
        // used to truncate `report.json` to 0 bytes and only then report
        // the unknown id, destroying whatever the file held before. See
        // #212.
        if let Some(id) = &opts.explain
            && !registry.all_rules().iter().any(|(r, _)| r.id() == id.as_str())
        {
            return Err(format!("no rule with id '{}' — try `scythe audit --list-rules`", id).into());
        }

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

    let mut findings: Vec<Finding> = Vec::new();

    if opts.files.is_empty() {
        findings.extend(audit_from_config(&opts.config_path, opts.ignore_suppressions)?);
    } else {
        let engine = dialect.as_deref().unwrap_or("postgres");
        findings.extend(audit_explicit_files(
            &opts.files,
            engine,
            opts.ignore_suppressions,
            &opts.config_path,
        )?);
    }

    if let Some(floor) = severity_floor {
        findings.retain(|f| f.severity >= floor);
    }

    let mut out: Box<dyn Write> = open_output(opts.output.as_deref())?;
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

/// Validates against [`KNOWN_ENGINE_ALIASES`] -- the same list
/// [`parse_engine_dialect`] validates the config's `engine = "..."` against
/// (#165, item 3) -- so `--dialect` and `engine` reject exactly the same
/// set, with one list to keep in sync with `SqlDialect::from_str` instead of
/// two drifting independently.
///
/// Before this, an unrecognized `--dialect` (a typo, or literal gibberish
/// like `klingon`) fell through `SqlDialect::from_str(..).unwrap_or(PostgreSQL)`
/// and silently became PostgreSQL -- identical output to `--dialect
/// postgres`, with no error either way. See #212.
fn validate_audit_dialect(raw: &str) -> Result<String, String> {
    let lower = raw.to_ascii_lowercase();
    if KNOWN_ENGINE_ALIASES.contains(&lower.as_str()) {
        Ok(lower)
    } else {
        Err(format!(
            "unknown --dialect '{raw}' (expected {})",
            KNOWN_ENGINE_ALIASES.join("|")
        ))
    }
}

/// Open the output sink for a reporter. `None` → stdout. `Some(path)` →
/// truncating file write. Parent directory must already exist.
fn open_output(path: Option<&str>) -> Result<Box<dyn Write>, Box<dyn std::error::Error>> {
    match path {
        None => Ok(Box::new(std::io::stdout())),
        Some(p) => {
            let f = File::create(p).map_err(|e| format!("failed to open output file '{}': {}", p, e))?;
            Ok(Box::new(std::io::BufWriter::new(f)))
        }
    }
}

/// Build the rule registry from `scythe.toml`'s `[lint]` and `[audit]`
/// sections: canonical rules plus severity overrides, plus any user-defined
/// rules from `[[audit.rule]]` and `extra_rules`.
///
/// Used two ways:
/// - Discovery (`--list-rules` / `--explain`): the registry is never
///   executed, so the catalog reflects severities and rules as configured.
/// - Explicit-file mode (`scythe audit <file>...`): the *same* registry is
///   executed by [`audit_explicit_files`] -- before this, explicit-file mode
///   built a bare `default_registry()` and ignored `scythe.toml` entirely,
///   so a rule configured `"off"` still fired and `[[audit.rule]]` entries
///   never ran (issue #206, item 2).
///
/// Returns the canonical registry, unmodified, when `config_path` does not
/// exist -- explicit-file mode is meant to work without a config at all.
fn load_registry(config_path: &str) -> Result<RuleRegistry, Box<dyn std::error::Error>> {
    let mut registry = default_registry();
    if !Path::new(config_path).exists() {
        return Ok(registry);
    }
    let config_str = std::fs::read_to_string(config_path)?;
    let parsed: toml::Value = toml::from_str(&config_str)?;
    if let Some(lint_section) = parsed.get("lint")
        && let Ok(lint_config) = lint_section.clone().try_into::<scythe_lint::types::LintConfig>()
    {
        registry.apply_config(&lint_config);
    }
    if let Some(audit_section) = parsed.get("audit") {
        let base_dir = config_dir(config_path);
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
                    let abs_path = base_dir.join(rel_path);
                    let path_str = abs_path.display().to_string();
                    let specs = load_rules_from_file(&abs_path).map_err(|e: AuditConfigError| e.to_string())?;
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
    // `all_rules`, not `active_rules`: this is a catalog listing, not the set
    // of rules that will fire. SC-A02 and SC-C01 are `Off` by default but are
    // still real, registered rules — omitting them from `--list-rules` would
    // undercount the documented rule set (see `RuleRegistry::all_rules`).
    let mut rows: Vec<(&dyn LintRule, Severity)> = registry.all_rules();
    rows.sort_by_key(|(r, _)| (r.category() as u8, r.id()));

    let id_w = rows.iter().map(|(r, _)| r.id().len()).max().unwrap_or(2).max(2);
    let name_w = rows.iter().map(|(r, _)| r.name().len()).max().unwrap_or(4).max(4);

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
    // Same reasoning as `print_rule_catalog`: `--explain` must be able to
    // explain an off-by-default rule like SC-A02 or SC-C01, not just the
    // ones that would currently fire.
    let rules = registry.all_rules();
    let Some((rule, sev)) = rules.iter().find(|(r, _)| r.id() == rule_id) else {
        return Err(format!("no rule with id '{}' — try `scythe audit --list-rules`", rule_id).into());
    };
    writeln!(out, "{} — {}", rule.id(), rule.name())?;
    writeln!(out, "  category: {}", rule.category())?;
    writeln!(out, "  severity: {}", severity_label(*sev))?;
    // `LintRule::cwe()`, not a regex scrape of `description()`. The rule
    // declares its CWE list (`cwe = ["CWE-78"]`); every canonical rule also
    // repeats those ids in its description prose, so the scrape happened to
    // agree -- a second derivation of one fact, held in sync by hand and by
    // nothing else. It does not agree for a user rule that declares `cwe`
    // without restating it in prose, which then reported no CWE at all.
    // `MatcherRule::cwe` keeps the description scrape as its own fallback for
    // the reverse case.
    let cwes = rule.cwe();
    if !cwes.is_empty() {
        writeln!(out, "  cwe:      {}", cwes.join(", "))?;
    }
    writeln!(out)?;
    writeln!(out, "{}", rule.description())?;
    Ok(())
}

/// Select the rules `scythe audit` will actually execute against `dialect`,
/// and report -- out loud -- everything that was dropped on the way.
///
/// Two filters decide what runs, and before this neither was observable:
///
/// - **Category.** `audit` only executes Security, Migration and Antipattern
///   rules; the rest of the registry is irrelevant here.
/// - **Dialect.** [`LintRule::is_applicable_to`] gates a rule to the dialects
///   its spec declares. Most canonical migration rules (and several security
///   rules) declare `dialects = ["postgres"]`, so auditing a MySQL project
///   silently ran a much smaller rule set than the one `--list-rules`
///   advertises. `MatcherRule::check_query` applies the same gate internally
///   and returns an empty `Vec`, which is indistinguishable from "the rule
///   ran and found nothing" -- exactly the invisible skip #167 describes.
///
/// Returns the rules to run plus, when *nothing* is left to run, an
/// error-severity [`Finding`]. A zero-rule audit that prints no findings and
/// exits 0 is a verification apparatus reporting success without verifying:
/// the finding turns it into an exit-2 failure that names the cause.
/// `--exit-zero` still suppresses the exit code, as for any other finding.
fn prepare_audit_rules<'a>(
    rules: &[(&'a dyn LintRule, Severity)],
    dialect: SqlDialect,
    engine_label: &str,
) -> (Vec<(&'a dyn LintRule, Severity)>, Option<Finding>) {
    let in_category: Vec<(&'a dyn LintRule, Severity)> = rules
        .iter()
        .filter(|(rule, _)| {
            matches!(
                rule.category(),
                RuleCategory::Security | RuleCategory::Migration | RuleCategory::Antipattern
            )
        })
        .copied()
        .collect();

    let mut applicable: Vec<(&'a dyn LintRule, Severity)> = Vec::with_capacity(in_category.len());
    let mut skipped: Vec<&'static str> = Vec::new();
    for (rule, severity) in in_category {
        if rule.is_applicable_to(dialect) {
            applicable.push((rule, severity));
        } else {
            skipped.push(rule.id());
        }
    }

    if !skipped.is_empty() {
        eprintln!(
            "audit: {} rule(s) skipped: not applicable to engine '{}' ({})",
            skipped.len(),
            engine_label,
            skipped.join(", ")
        );
    }

    if applicable.is_empty() {
        let finding = Finding {
            file: String::new(),
            query_name: None,
            rule_id: "SC-AUDIT00".to_string(),
            rule_name: Some("no-applicable-rules".to_string()),
            rule_description: Some(
                "scythe audit had no rule to run, so it examined nothing. Reported as an error rather \
                 than an empty (clean-looking) report so an audit that verified nothing cannot pass a \
                 CI gate."
                    .to_string(),
            ),
            severity: Severity::Error,
            message: format!(
                "no audit rule applies to engine '{}': {} rule(s) are restricted to other dialects and \
                 the rest are configured off — audit examined 0 rules and cannot report on this project",
                engine_label,
                skipped.len()
            ),
            line: None,
            column: None,
            cwe: Vec::new(),
            source: Some("audit".to_string()),
        };
        return (applicable, Some(finding));
    }

    (applicable, None)
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Off => "off",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}

fn audit_from_config(config_path: &str, ignore_suppressions: bool) -> Result<Vec<Finding>, Box<dyn std::error::Error>> {
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

    let config_str =
        std::fs::read_to_string(config_path).map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }

    let base_dir = config_dir(config_path);
    let matcher_registry = MatcherRegistry::canonical();

    let mut user_specs: Vec<(RuleSpec, String)> = config
        .audit
        .rules
        .into_iter()
        .map(|spec| (spec, config_path.to_string()))
        .collect();

    for rel_path in &config.audit.extra_rules {
        let abs_path = base_dir.join(rel_path);
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
        // Per `[[sql]]` block, because the dialect gate is per block: the
        // same registry can yield a different executable rule set for a
        // postgres block and a mysql block in one config.
        let engine_label = if sql_config.engine.is_empty() {
            "postgres"
        } else {
            sql_config.engine.as_str()
        };
        // `parse_engine_dialect`, not `SqlDialect::from_str(..).unwrap_or(PostgreSQL)`:
        // a typo'd engine (`mysql8`) used to be silently audited as
        // PostgreSQL -- wrong catalog parsing and a wrong dialect-gated rule
        // set, with `scythe audit` reporting success regardless (#165, item
        // 3). `--dialect` already rejects this same mistake via
        // `validate_audit_dialect`; the config's `engine` field did not.
        let sql_dialect = parse_engine_dialect(engine_label).map_err(|e| format!("[{}] {}", sql_config.name, e))?;
        let (rules, no_rules_finding) = prepare_audit_rules(&rules, sql_dialect, engine_label);
        if let Some(finding) = no_rules_finding {
            findings.push(finding);
        }

        let schema_files = resolve_globs(&sql_config.schema, base_dir, &format!("[{}] schema", sql_config.name))?;
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| std::fs::read_to_string(p).map_err(|e| format!("failed to read schema file '{}': {}", p, e)))
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &sql_dialect)?;

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

        let query_files = resolve_globs(&sql_config.queries, base_dir, &format!("[{}] queries", sql_config.name))?;
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
    config_path: &str,
) -> Result<Vec<Finding>, Box<dyn std::error::Error>> {
    // `parse_engine_dialect`, not `SqlDialect::from_str(..).unwrap_or(PostgreSQL)`
    // (#165, item 3). `engine` is always already-validated here in practice
    // -- `run_audit` passes either its `--dialect` value (validated by
    // `validate_audit_dialect`) or the literal default `"postgres"` -- but
    // this function is `pub(crate)`, so a future in-crate caller that skips
    // that validation must still get an error naming the bad value instead
    // of a silent PostgreSQL fallback.
    let sql_dialect = parse_engine_dialect(engine).map_err(|e| format!("audit: {e}"))?;

    let catalog = Catalog::from_ddl_with_dialect(&[], &sql_dialect)
        .unwrap_or_else(|_| Catalog::from_ddl_with_dialect(&[], &SqlDialect::PostgreSQL).expect("empty catalog"));

    // Loads `scythe.toml`'s `[lint]`/`[audit]` sections when `config_path`
    // exists, falling back to the canonical registry when it does not --
    // explicit-file mode must still work with no config at all. Before
    // this, explicit-file mode always used a bare `default_registry()`, so
    // a rule turned `"off"` in `scythe.toml` still fired and `[[audit.rule]]`
    // user rules never ran (#206, item 2).
    let registry = load_registry(config_path)?;
    let all_active = registry.active_rules();
    let (rules, no_rules_finding) = prepare_audit_rules(&all_active, sql_dialect, engine);

    let mut findings = Vec::new();
    if let Some(finding) = no_rules_finding {
        findings.push(finding);
    }
    for path in files {
        let content = std::fs::read_to_string(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;
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

/// One statement [`parse_statements_lenient`] could not parse: its
/// approximate 1-based starting line and the parser's error message.
struct StatementParseFailure {
    line: usize,
    message: String,
}

/// Parse `sql` into individual statements, recovering from a parse error on
/// one statement by skipping to the next top-level `;` (or EOF) and
/// continuing, instead of discarding every statement already parsed.
///
/// [`sqlparser::parser::Parser::parse_sql`] parses the whole input as one
/// unit and fails on the first unparseable statement, taking every
/// statement before it down too. For `scythe audit`, that meant a single
/// statement using SQL syntax the parser does not yet support silently
/// disabled every finding in the rest of the file -- including real
/// security findings -- with only a stderr line (not visible to a CI job
/// checking the exit code) marking what happened. See issue #208.
///
/// Returns each parsed statement paired with its 1-based starting line
/// (found via the token span at the point `parse_statement` was called, so
/// it stays correct even when earlier statements were skipped -- unlike a
/// separate post-hoc token walk keyed by statement *index*, which would
/// drift out of sync with `sql`'s real line numbers the moment any
/// statement is skipped), plus every statement that could not be parsed.
fn parse_statements_lenient(
    dialect: &dyn sqlparser::dialect::Dialect,
    sql: &str,
) -> (Vec<(sqlparser::ast::Statement, usize)>, Vec<StatementParseFailure>) {
    use sqlparser::parser::Parser;
    use sqlparser::tokenizer::Token;

    let mut parser = match Parser::new(dialect).try_with_sql(sql) {
        Ok(p) => p,
        Err(e) => {
            return (
                Vec::new(),
                vec![StatementParseFailure {
                    line: 1,
                    message: e.to_string(),
                }],
            );
        }
    };

    let mut statements = Vec::new();
    let mut failures = Vec::new();

    loop {
        while parser.consume_token(&Token::SemiColon) {}
        if matches!(parser.peek_token_ref().token, Token::EOF) {
            break;
        }

        let start_line = (parser.peek_token_ref().span.start.line as usize).max(1);

        match parser.parse_statement() {
            Ok(stmt) => statements.push((stmt, start_line)),
            Err(e) => {
                failures.push(StatementParseFailure {
                    line: start_line,
                    message: e.to_string(),
                });
                // Recover by skipping to the next top-level `;` (or EOF) so
                // the statements after this one still get a chance to parse
                // and be audited, rather than aborting the whole file.
                loop {
                    match parser.peek_token_ref().token {
                        Token::EOF | Token::SemiColon => break,
                        _ => {
                            parser.next_token();
                        }
                    }
                }
            }
        }
    }

    (statements, failures)
}

/// Parse `sql` statement-by-statement and run every security rule over each.
///
/// # Recovery
///
/// Uses [`parse_statements_lenient`], not
/// [`sqlparser::parser::Parser::parse_sql`]: a statement that fails to parse
/// is skipped (and reported as its own `SC-PARSE01` error finding, naming
/// the line and the parser's message) rather than discarding every
/// statement already parsed. See #208.
///
/// # Rule set
///
/// `rules` must already be narrowed by [`prepare_audit_rules`] -- category
/// *and* dialect. Filtering here as well would be a second derivation of the
/// same fact, and the dialect half of it would once more be invisible: the
/// count of skipped rules has to be reported where the audit knows what it is
/// about to run, not buried in a per-statement loop.
///
/// # Suppression
///
/// A `SuppressionSet` is built once from the full SQL string and looked up by
/// each statement's 0-based enumeration index (`idx`), matching how
/// `SuppressionSet::parse` counts statements internally -- see its module
/// docs. Looking it up by source line instead used to over-suppress: two
/// statements sharing one physical line resolved to the same line-keyed
/// entry, so a suppression meant only for the first silently covered the
/// second too.
///
/// # Reported line numbers
///
/// Statement start lines (used for [`Finding::line`], *not* for suppression
/// lookup) come from [`parse_statements_lenient`], which reads them from the
/// parser's own token positions. This avoids re-splitting on `;` (which is
/// quote-unsafe) and gives accurate 1-based line numbers even for multi-line
/// statements.
pub(crate) fn run_security_rules_over_sql(
    path: &str,
    sql: &str,
    dialect: &SqlDialect,
    catalog: &Catalog,
    rules: &[(&dyn scythe_lint::LintRule, Severity)],
    ignore_suppressions: bool,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    let suppressions = SuppressionSet::parse(sql);

    let parser_dialect = dialect.to_sqlparser_dialect();
    let (statements, failures) = parse_statements_lenient(parser_dialect.as_ref(), sql);

    for failure in &failures {
        eprintln!(
            "warning: failed to parse a statement in '{}' at line {}: {}",
            path, failure.line, failure.message
        );
        findings.push(Finding {
            file: path.to_string(),
            query_name: None,
            rule_id: "SC-PARSE01".to_string(),
            rule_name: Some("unparseable statement".to_string()),
            rule_description: Some(
                "A statement could not be parsed and was skipped, so no security rule could examine it. \
                 This is reported as an error rather than only a stderr warning so a parser gap cannot \
                 silently disable auditing for part of a file."
                    .to_string(),
            ),
            severity: Severity::Error,
            message: format!(
                "failed to parse statement at line {}: {}",
                failure.line, failure.message
            ),
            line: Some(failure.line),
            column: None,
            cwe: Vec::new(),
            source: Some("audit".to_string()),
        });
    }

    let empty_annotations = Annotations::default();
    let empty_analyzed = AnalyzedQuery::default();

    for (idx, (stmt, stmt_line)) in statements.iter().enumerate() {
        let stmt_line = *stmt_line;

        let ctx = LintContext {
            sql,
            stmt,
            analyzed: &empty_analyzed,
            catalog,
            annotations: &empty_annotations,
            dialect: *dialect,
        };

        for (rule, severity) in rules {
            for violation in rule.check_query(&ctx) {
                // `SuppressionSet` is keyed by 0-based statement index, not
                // by source line (scythe-lint #140: two statements sharing
                // one physical line previously resolved to the same
                // line-keyed entry and a suppression meant for the first
                // silently covered the second too).
                if !ignore_suppressions
                    && !suppressions.is_empty()
                    && suppressions.is_suppressed(&violation.rule_id, idx)
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
                    line: Some(stmt_line),
                    column: None,
                    // `LintRule::cwe()`, not `extract_cwe(rule.description())`
                    // -- the rule's declaration rather than a regex over its
                    // prose. See `print_rule_explanation` for what the scrape
                    // got wrong.
                    cwe: rule.cwe(),
                    source: Some("audit".to_string()),
                });
            }
        }
    }

    findings
}
