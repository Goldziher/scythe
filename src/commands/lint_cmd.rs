use std::borrow::Cow;
use std::path::Path;

use crate::lint::sqruff_adapter;
use crate::lint::types::Severity;

use super::shared::{resolve_globs, split_query_file};

/// A combined lint violation that can come from either scythe rules or sqruff.
struct FileViolation {
    file: String,
    query_name: Option<String>,
    rule_id: Cow<'static, str>,
    severity: Severity,
    message: String,
    line_no: Option<usize>,
    line_pos: Option<usize>,
}

/// Run the `lint` command.
///
/// - If files are provided without a valid config, run sqruff-only linting.
/// - If a config is available, run both scythe rules (with schema context) and sqruff rules.
/// - `--fix`: apply sqruff auto-fixes to files.
pub fn run_lint(
    config_path: &str,
    fix: bool,
    dialect: Option<&str>,
    files: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let dialect = dialect.unwrap_or("ansi");

    // Determine if we have a config file available
    let has_config = Path::new(config_path).exists();

    if !files.is_empty() {
        // Files explicitly provided: run sqruff on those files
        return lint_files(files, dialect, fix);
    }

    if !has_config {
        return Err(format!(
            "No files specified and config '{}' not found. Provide files or a config path.",
            config_path
        )
        .into());
    }

    // Run full lint: scythe rules + sqruff rules using config
    lint_from_config(config_path, dialect, fix)
}

/// Lint specific files using sqruff only (no scythe schema-aware rules).
fn lint_files(
    files: &[String],
    dialect: &str,
    fix: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut all_violations: Vec<FileViolation> = Vec::new();

    for path in files {
        let sql = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read '{}': {}", path, e))?;

        if fix {
            let (violations, fixed) = sqruff_adapter::lint_and_fix_sql(&sql, dialect);
            for sv in &violations {
                all_violations.push(FileViolation {
                    file: path.clone(),
                    query_name: None,
                    rule_id: sv.violation.rule_id.clone(),
                    severity: Severity::Warn,
                    message: sv.violation.message.clone(),
                    line_no: Some(sv.line_no),
                    line_pos: Some(sv.line_pos),
                });
            }
            if fixed != sql {
                std::fs::write(path, &fixed)
                    .map_err(|e| format!("failed to write '{}': {}", path, e))?;
                eprintln!("fixed {}", path);
            }
        } else {
            let violations = sqruff_adapter::lint_sql(&sql, dialect);
            for sv in &violations {
                all_violations.push(FileViolation {
                    file: path.clone(),
                    query_name: None,
                    rule_id: sv.violation.rule_id.clone(),
                    severity: Severity::Warn,
                    message: sv.violation.message.clone(),
                    line_no: Some(sv.line_no),
                    line_pos: Some(sv.line_pos),
                });
            }
        }
    }

    print_violations(&all_violations)
}

/// Lint from config: run both scythe rules and sqruff rules.
fn lint_from_config(
    config_path: &str,
    dialect: &str,
    fix: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use serde::Deserialize;

    use crate::analyzer::analyze;
    use crate::catalog::Catalog;
    use crate::lint::{LintContext, LintEngine, default_registry};
    use crate::parser::parse_query;

    #[derive(Deserialize)]
    struct ScytheConfig {
        sql: Vec<SqlConfig>,
        #[serde(default)]
        lint: Option<crate::lint::types::LintConfig>,
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

    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig = toml::from_str(&config_str)
        .map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    // Build lint engine
    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }
    let engine = LintEngine::new(registry);

    let mut all_violations: Vec<FileViolation> = Vec::new();

    for sql_config in &config.sql {
        eprintln!("[{}] Parsing schema...", sql_config.name);

        // Resolve schema files
        let schema_files = resolve_globs(&sql_config.schema)?;
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| {
                std::fs::read_to_string(p)
                    .map_err(|e| format!("failed to read schema file '{}': {}", p, e))
            })
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

        let catalog = Catalog::from_ddl(&schema_refs)?;

        // Resolve query files
        let query_files = resolve_globs(&sql_config.queries)?;

        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;

            // Run sqruff on the entire file
            if fix {
                let (sq_violations, fixed) = sqruff_adapter::lint_and_fix_sql(&content, dialect);
                for sv in &sq_violations {
                    all_violations.push(FileViolation {
                        file: query_file.clone(),
                        query_name: None,
                        rule_id: sv.violation.rule_id.clone(),
                        severity: Severity::Warn,
                        message: sv.violation.message.clone(),
                        line_no: Some(sv.line_no),
                        line_pos: Some(sv.line_pos),
                    });
                }
                if fixed != content {
                    std::fs::write(query_file, &fixed)
                        .map_err(|e| format!("failed to write '{}': {}", query_file, e))?;
                    eprintln!("fixed {}", query_file);
                }
            } else {
                let sq_violations = sqruff_adapter::lint_sql(&content, dialect);
                for sv in &sq_violations {
                    all_violations.push(FileViolation {
                        file: query_file.clone(),
                        query_name: None,
                        rule_id: sv.violation.rule_id.clone(),
                        severity: Severity::Warn,
                        message: sv.violation.message.clone(),
                        line_no: Some(sv.line_no),
                        line_pos: Some(sv.line_pos),
                    });
                }
            }

            // Run scythe rules on individual query blocks
            let blocks = split_query_file(&content);
            for block in &blocks {
                let parsed = match parse_query(block) {
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
                    });
                }
            }
        }

        // Catalog-level checks
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
            });
        }
    }

    print_violations(&all_violations)
}

/// Print violations grouped by file and return an error if there are errors.
fn print_violations(violations: &[FileViolation]) -> Result<(), Box<dyn std::error::Error>> {
    if violations.is_empty() {
        eprintln!("No lint violations found.");
        return Ok(());
    }

    let mut error_count = 0usize;
    let mut warning_count = 0usize;

    // Group by file
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

        let location = match (v.line_no, v.line_pos) {
            (Some(line), Some(pos)) => format!("{}:{}", line, pos),
            _ => match &v.query_name {
                Some(name) => format!("query:{}", name),
                None => String::new(),
            },
        };

        if location.is_empty() {
            eprintln!("  {}: [{}] {}", severity_str, v.rule_id, v.message);
        } else {
            eprintln!(
                "  {} {}: [{}] {}",
                location, severity_str, v.rule_id, v.message
            );
        }
    }

    eprintln!();
    if error_count > 0 {
        Err(format!(
            "lint: {} error(s), {} warning(s)",
            error_count, warning_count
        )
        .into())
    } else {
        if warning_count > 0 {
            eprintln!("lint: {} warning(s)", warning_count);
        }
        Ok(())
    }
}
