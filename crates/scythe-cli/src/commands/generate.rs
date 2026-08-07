use std::borrow::Cow;
use std::path::Path;

use serde::Deserialize;

use ahash::AHashSet;

use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case};
use scythe_codegen::{
    CodegenBackend, RbsEnumInfo, RbsGenerationContext, RbsQueryInfo, TypeOverride,
    generate_single_enum_def_with_backend, generate_with_backend_and_overrides, get_backend,
};
use scythe_core::analyzer::{AnalyzedQuery, EnumInfo, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::{QueryCommand, parse_query_with_dialect};

use super::shared::{config_dir, redact_url_password, resolve_globs, split_query_file};

#[derive(Debug, Deserialize)]
struct ScytheConfig {
    #[allow(dead_code)]
    scythe: ScytheMeta,
    sql: Vec<SqlConfig>,
    #[serde(default)]
    pub lint: Option<scythe_lint::types::LintConfig>,
}

#[derive(Debug, Deserialize)]
struct ScytheMeta {
    #[allow(dead_code)]
    version: String,
}

#[derive(Debug, Deserialize)]
struct SqlConfig {
    name: String,
    engine: String,
    schema: Vec<String>,
    queries: Vec<String>,
    /// Legacy: output directory (used as default when no gen targets specified)
    #[serde(default)]
    output: Option<String>,
    /// Generation targets via [[sql.gen]] or [sql.gen.rust]
    #[serde(default, rename = "gen")]
    gen_config: Option<GenTargets>,
    #[serde(default)]
    type_overrides: Option<Vec<TypeOverrideConfig>>,
}

/// Supports both legacy `[sql.gen.rust]` and new `[[sql.gen]]` array formats.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GenTargets {
    /// New format: `[[sql.gen]]` array of targets
    Array(Vec<GenTarget>),
    /// Legacy format: `[sql.gen.rust]` with a nested language key
    Legacy(LegacyGenConfig),
}

/// New format: each target specifies a backend and output directory.
/// Extra keys (e.g. `row_type = "pydantic"`) are captured in `options`.
#[derive(Debug, Deserialize)]
struct GenTarget {
    backend: String,
    output: String,
    #[serde(flatten)]
    options: std::collections::HashMap<String, toml::Value>,
}

/// Legacy format: `[sql.gen.rust]` with target field.
#[derive(Debug, Deserialize)]
struct LegacyGenConfig {
    rust: Option<LegacyRustGenConfig>,
    python: Option<LegacyLangGenConfig>,
    typescript: Option<LegacyLangGenConfig>,
    go: Option<LegacyLangGenConfig>,
}

#[derive(Debug, Deserialize)]
struct LegacyRustGenConfig {
    target: String,
    #[allow(dead_code)]
    derive: Option<Vec<String>>,
    #[allow(dead_code)]
    serde: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LegacyLangGenConfig {
    target: String,
}

#[derive(Debug, Deserialize)]
struct TypeOverrideConfig {
    column: Option<String>,
    db_type: Option<String>,
    #[serde(rename = "type")]
    neutral_type: Option<String>,
}

/// A resolved generation target with backend name, output directory, and options.
struct ResolvedGenTarget {
    backend: String,
    output: String,
    options: std::collections::HashMap<String, String>,
}

/// Stringify a toml::Value for passing to backends as flat string options.
fn toml_value_to_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// Convert config into a list of resolved generation targets.
fn resolve_gen_targets(sql_config: &SqlConfig) -> Vec<ResolvedGenTarget> {
    let default_output = sql_config.output.clone().unwrap_or_else(|| "generated".to_string());

    match &sql_config.gen_config {
        Some(GenTargets::Array(targets)) => targets
            .iter()
            .map(|t| {
                let options = t
                    .options
                    .iter()
                    .map(|(k, v)| (k.clone(), toml_value_to_string(v)))
                    .collect();
                ResolvedGenTarget {
                    backend: t.backend.clone(),
                    output: t.output.clone(),
                    options,
                }
            })
            .collect(),
        Some(GenTargets::Legacy(legacy)) => {
            let mut targets = Vec::new();
            if let Some(ref rust) = legacy.rust {
                let backend = match rust.target.as_str() {
                    "tokio-postgres" => "rust-tokio-postgres",
                    _ => "rust-sqlx",
                };
                let mut options = std::collections::HashMap::new();
                if let Some(true) = rust.serde {
                    options.insert("serde".to_string(), "true".to_string());
                }
                if let Some(ref derives) = rust.derive {
                    options.insert("derive".to_string(), derives.join(", "));
                }
                targets.push(ResolvedGenTarget {
                    backend: backend.to_string(),
                    output: default_output.clone(),
                    options,
                });
            }
            if let Some(ref py) = legacy.python {
                targets.push(ResolvedGenTarget {
                    backend: format!("python-{}", py.target),
                    output: default_output.clone(),
                    options: std::collections::HashMap::new(),
                });
            }
            if let Some(ref ts) = legacy.typescript {
                targets.push(ResolvedGenTarget {
                    backend: format!("typescript-{}", ts.target),
                    output: default_output.clone(),
                    options: std::collections::HashMap::new(),
                });
            }
            if let Some(ref go) = legacy.go {
                targets.push(ResolvedGenTarget {
                    backend: format!("go-{}", go.target),
                    output: default_output.clone(),
                    options: std::collections::HashMap::new(),
                });
            }
            if targets.is_empty() {
                targets.push(ResolvedGenTarget {
                    backend: "rust-sqlx".to_string(),
                    output: default_output,
                    options: std::collections::HashMap::new(),
                });
            }
            targets
        }
        None => {
            vec![ResolvedGenTarget {
                backend: "rust-sqlx".to_string(),
                output: default_output,
                options: std::collections::HashMap::new(),
            }]
        }
    }
}

pub fn run_generate(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config_str =
        std::fs::read_to_string(config_path).map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let base_dir = config_dir(config_path);

    for sql_config in &config.sql {
        eprintln!("[{}] Parsing schema...", sql_config.name);

        let schema_files = resolve_globs(&sql_config.schema, base_dir, &format!("[{}] schema", sql_config.name))?;

        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| std::fs::read_to_string(p).map_err(|e| format!("failed to read schema file '{}': {}", p, e)))
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

        let dialect = SqlDialect::from_str(&sql_config.engine).unwrap_or(SqlDialect::PostgreSQL);
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &dialect)?;

        let query_files = resolve_globs(&sql_config.queries, base_dir, &format!("[{}] queries", sql_config.name))?;

        let mut all_query_blocks = Vec::new();
        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;
            let blocks = split_query_file(&content);
            all_query_blocks.extend(blocks);
        }

        eprintln!("[{}] Analyzing {} queries...", sql_config.name, all_query_blocks.len());

        let mut analyzed_queries: Vec<AnalyzedQuery> = Vec::new();
        for block in &all_query_blocks {
            let parsed = parse_query_with_dialect(block, &dialect)?;
            let analyzed = analyze(&catalog, &parsed)?;
            analyzed_queries.push(analyzed);
        }

        let overrides: Vec<TypeOverride> = sql_config
            .type_overrides
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|o| TypeOverride {
                column: o.column.clone(),
                db_type: o.db_type.clone(),
                neutral_type: o.neutral_type.clone(),
            })
            .collect();

        let gen_targets = resolve_gen_targets(sql_config);

        for target in &gen_targets {
            let mut backend = get_backend(&target.backend, &sql_config.engine).map_err(|e| {
                format!(
                    "backend '{}' with engine '{}': {}",
                    target.backend, sql_config.engine, e
                )
            })?;

            if !target.options.is_empty() {
                backend
                    .apply_options(&target.options)
                    .map_err(|e| format!("backend '{}' apply_options failed: {}", target.backend, e))?;
            }

            // `output` is a path, not a glob pattern, so it is resolved via
            // plain `Path::join` (not `rebase_pattern`/`glob::Pattern::escape`)
            // — an output directory literally named `a[1]` must not be
            // mangled. `PathBuf::push` (which `join` uses internally) leaves
            // an already-absolute `target.output` unchanged: pushing an
            // absolute path replaces the buffer instead of appending to it.
            let output_dir = base_dir.join(&target.output).to_string_lossy().into_owned();

            generate_for_backend(&sql_config.name, &*backend, &analyzed_queries, &output_dir, &overrides)?;
        }
    }

    eprintln!("Done.");
    Ok(())
}

/// A single generated query's code alongside the enum definitions it
/// references, kept together so [`assemble_output`] can dedupe enums and
/// interleave (or separate) query code without re-deriving either from
/// `analyzed_queries`.
struct QueryResult {
    code: scythe_codegen::GeneratedCode,
    enums: Vec<EnumInfo>,
}

/// Assemble the full file body for a backend from its per-query results:
/// file header, deduped enum definitions, model/row structs and query
/// functions (ordered per `query_class_header`), file footer, and post
/// footer — joined into the final string, including the "no queries"
/// fallback. Pure and I/O-free so it can be unit tested directly and so
/// post-assembly steps (e.g. rustfmt) stay visibly separate in the caller.
fn assemble_output(backend: &dyn CodegenBackend, results: &[QueryResult]) -> String {
    let mut seen_enums = AHashSet::new();
    let mut unique_enum_defs: Vec<String> = Vec::new();
    for result in results {
        for info in &result.enums {
            if seen_enums.insert(info.sql_name.clone())
                && let Ok(def) = generate_single_enum_def_with_backend(info, backend)
            {
                unique_enum_defs.push(def);
            }
        }
    }

    let mut output_parts: Vec<String> = Vec::new();

    let generated: Vec<scythe_codegen::GeneratedCode> = results.iter().map(|result| result.code.clone()).collect();
    let header = backend.file_header_for_results(&generated);
    if !header.is_empty() {
        output_parts.push(header);
    }

    for def in &unique_enum_defs {
        output_parts.push(def.clone());
    }

    let class_header = backend.query_class_header();
    if class_header.is_empty() {
        for result in results {
            if let Some(ref s) = result.code.model_struct {
                output_parts.push(s.clone());
            }
            if let Some(ref s) = result.code.row_struct {
                output_parts.push(s.clone());
            }
            if let Some(ref s) = result.code.query_fn {
                output_parts.push(s.clone());
            }
        }
    } else {
        for result in results {
            if let Some(ref s) = result.code.model_struct {
                output_parts.push(s.clone());
            }
            if let Some(ref s) = result.code.row_struct {
                output_parts.push(s.clone());
            }
        }
        output_parts.push(class_header);
        for result in results {
            if let Some(ref s) = result.code.query_fn {
                output_parts.push(s.clone());
            }
        }
    }

    let footer = backend.file_footer();
    if !footer.is_empty() {
        output_parts.push(footer);
    }

    let post_footer = backend.post_footer();
    if !post_footer.is_empty() {
        output_parts.push(post_footer);
    }

    if output_parts.is_empty() {
        "// No queries generated.\n".to_string()
    } else {
        output_parts.join("\n\n") + "\n"
    }
}

/// The output filename for a backend's generated queries file. Extracted so
/// the `.java` capitalization rule lives in exactly one place: `run_check`
/// does not read generated artifacts today, but header verification will need
/// to locate the same file this writes, and a second copy of the rule would
/// diverge silently.
fn output_filename(backend: &dyn CodegenBackend) -> String {
    let ext = &backend.manifest().backend.file_extension;
    if ext == "java" {
        format!("Queries.{}", ext)
    } else {
        format!("queries.{}", ext)
    }
}

/// Generate output for a single backend target.
fn generate_for_backend(
    config_name: &str,
    backend: &dyn CodegenBackend,
    analyzed_queries: &[AnalyzedQuery],
    output_dir: &str,
    overrides: &[TypeOverride],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut results: Vec<QueryResult> = Vec::new();
    for analyzed in analyzed_queries {
        let enums = analyzed.enums.clone();
        let code = generate_with_backend_and_overrides(analyzed, backend, overrides)?;
        results.push(QueryResult { code, enums });
    }

    let mut output_content = assemble_output(backend, &results);

    let filename = output_filename(backend);

    let out_path = Path::new(output_dir);
    std::fs::create_dir_all(out_path).map_err(|e| format!("failed to create output dir '{}': {}", output_dir, e))?;

    let output_file = out_path.join(&filename);

    if backend.manifest().backend.file_extension == "rs" {
        output_content = format_rust_code_if_possible(&output_content);
    }

    std::fs::write(&output_file, &output_content)
        .map_err(|e| format!("failed to write output file '{}': {}", output_file.display(), e))?;

    eprintln!(
        "[{}] Writing {} output to {}",
        config_name,
        backend.name(),
        output_file.display()
    );

    generate_rbs_if_supported(config_name, backend, analyzed_queries, overrides, out_path)?;

    Ok(())
}

/// Determine the struct name for a query, matching the logic in scythe_codegen.
fn determine_struct_name(analyzed: &AnalyzedQuery, naming: &scythe_backend::naming::NamingConfig) -> String {
    if let Some(ref table_name) = analyzed.source_table {
        let singular = scythe_codegen::singularize(table_name);
        to_pascal_case(&singular).into_owned()
    } else {
        row_struct_name(&analyzed.name, naming)
    }
}

/// Generate an RBS type signature file alongside the Ruby output file,
/// if the backend supports RBS generation (Ruby backends only).
fn generate_rbs_if_supported(
    config_name: &str,
    backend: &dyn CodegenBackend,
    analyzed_queries: &[AnalyzedQuery],
    overrides: &[TypeOverride],
    out_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let empty_context = RbsGenerationContext {
        queries: vec![],
        enums: vec![],
    };
    if backend.generate_rbs_file(&empty_context).is_none() {
        return Ok(());
    }

    let manifest = backend.manifest();
    let naming = &manifest.naming;

    let mut rbs_queries: Vec<RbsQueryInfo> = Vec::new();
    let mut seen_enums = AHashSet::new();
    let mut rbs_enums: Vec<RbsEnumInfo> = Vec::new();

    for analyzed in analyzed_queries {
        let source_table = analyzed.source_table.as_deref().unwrap_or("");
        let columns = scythe_codegen::resolve::resolve_columns(&analyzed.columns, manifest, overrides, source_table)?;
        let params = scythe_codegen::resolve::resolve_params(&analyzed.params, manifest, overrides, source_table)?;

        let func = fn_name(&analyzed.name, naming);
        let struct_name = determine_struct_name(analyzed, naming);

        let needs_struct = matches!(
            analyzed.command,
            QueryCommand::One | QueryCommand::Many | QueryCommand::Grouped
        ) && !analyzed.columns.is_empty();

        let command = if analyzed.command == QueryCommand::Grouped {
            QueryCommand::Many
        } else {
            analyzed.command.clone()
        };

        rbs_queries.push(RbsQueryInfo {
            func_name: func,
            struct_name: if needs_struct { Some(struct_name) } else { None },
            columns,
            params,
            command,
        });

        for enum_info in &analyzed.enums {
            if seen_enums.insert(enum_info.sql_name.clone()) {
                let type_name = enum_type_name(&enum_info.sql_name, naming);
                let values: Vec<String> = enum_info.values.iter().map(|v| enum_variant_name(v, naming)).collect();
                rbs_enums.push(RbsEnumInfo { type_name, values });
            }
        }
    }

    let context = RbsGenerationContext {
        queries: rbs_queries,
        enums: rbs_enums,
    };

    if let Some(rbs_content) = backend.generate_rbs_file(&context) {
        let rbs_file = out_path.join("queries.rbs");
        std::fs::write(&rbs_file, &rbs_content)
            .map_err(|e| format!("failed to write RBS file '{}': {}", rbs_file.display(), e))?;
        eprintln!(
            "[{}] Writing {} RBS signatures to {}",
            config_name,
            backend.name(),
            rbs_file.display()
        );
    }

    Ok(())
}

/// Inputs to [`run_check`]. Mirrors the clap `Commands::Check` shape.
pub struct RunCheckOpts {
    /// Path to `scythe.toml` (default: `"scythe.toml"`).
    pub config_path: String,
    /// Optional database URL. When present, each query is additionally
    /// prepared server-side and the reported shape is diffed against static
    /// inference. When absent, `check` needs no database at all.
    pub database_url: Option<String>,
    /// Reporter format string (human / sarif / json).
    pub format: String,
    /// Output path; `None` means stdout.
    pub output: Option<String>,
}

pub fn run_check(opts: RunCheckOpts) -> Result<(), Box<dyn std::error::Error>> {
    use scythe_lint::reporters::{Finding, Format};
    use scythe_lint::{LintContext, LintEngine, QueryViolation, Severity, default_registry, emit_findings};

    let config_path = opts.config_path.as_str();

    let format = Format::parse(&opts.format)
        .ok_or_else(|| format!("unknown --format '{}' (expected human|sarif|json)", opts.format))?;

    let config_str =
        std::fs::read_to_string(config_path).map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }
    let engine = LintEngine::new(registry);

    let mut all_violations: Vec<QueryViolation> = Vec::new();
    // Queries are grouped per `[[sql]]` block so verification connects once and
    // labels findings with the block's engine. `engine` is carried alongside
    // the queries so `verify_against_database` can skip non-PostgreSQL blocks
    // instead of running every query through the PostgreSQL wire protocol.
    let mut verifiable: Vec<VerifiableBlock> = Vec::new();

    let base_dir = config_dir(config_path);

    for sql_config in &config.sql {
        eprintln!("[{}] Parsing schema...", sql_config.name);

        let schema_files = resolve_globs(&sql_config.schema, base_dir, &format!("[{}] schema", sql_config.name))?;
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| std::fs::read_to_string(p).map_err(|e| format!("failed to read schema file '{}': {}", p, e)))
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

        let dialect = SqlDialect::from_str(&sql_config.engine).unwrap_or(SqlDialect::PostgreSQL);
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &dialect)?;

        let query_files = resolve_globs(&sql_config.queries, base_dir, &format!("[{}] queries", sql_config.name))?;
        let mut all_query_blocks = Vec::new();
        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;
            let blocks = split_query_file(&content);
            all_query_blocks.extend(blocks);
        }

        eprintln!("[{}] Checking {} queries...", sql_config.name, all_query_blocks.len());

        let mut query_names: Vec<String> = Vec::new();
        let mut analyzed_queries: Vec<scythe_core::analyzer::AnalyzedQuery> = Vec::new();

        for block in &all_query_blocks {
            let parsed = parse_query_with_dialect(block, &dialect)?;
            let analyzed = analyze(&catalog, &parsed)?;

            query_names.push(analyzed.name.clone());

            let ctx = LintContext {
                sql: &parsed.sql,
                stmt: &parsed.stmt,
                analyzed: &analyzed,
                catalog: &catalog,
                annotations: &parsed.annotations,
                dialect,
            };
            let violations = engine.check_query(&ctx);
            for (v, sev) in violations {
                all_violations.push(QueryViolation {
                    query_name: analyzed.name.clone(),
                    rule_id: v.rule_id.clone(),
                    severity: sev,
                    message: v.message,
                });
            }

            // Retained only for live-database verification, which requires
            // `--database-url` (see `should_retain_for_verification`).
            // Without it, cloning and keeping every analyzed query (columns,
            // params, source tables, ...) around for the rest of the process
            // is pure waste on the overwhelmingly common no-database `scythe
            // check` path.
            if should_retain_for_verification(opts.database_url.as_deref()) {
                analyzed_queries.push(analyzed);
            }
        }

        if should_retain_for_verification(opts.database_url.as_deref()) {
            verifiable.push(VerifiableBlock {
                name: sql_config.name.clone(),
                engine: sql_config.engine.clone(),
                queries: analyzed_queries,
            });
        }

        let cat_violations = engine.check_catalog(&catalog);
        for (v, sev) in cat_violations {
            all_violations.push(QueryViolation {
                query_name: String::new(),
                rule_id: v.rule_id.clone(),
                severity: sev,
                message: v.message,
            });
        }

        let mut seen_names: AHashSet<String> = AHashSet::new();
        for name in &query_names {
            if !seen_names.insert(name.clone()) {
                all_violations.push(QueryViolation {
                    query_name: name.clone(),
                    rule_id: Cow::Borrowed("SC-C03"),
                    severity: Severity::Error,
                    message: format!("duplicate query name: \"{}\"", name),
                });
            }
        }

        eprintln!("[{}] All queries valid.", sql_config.name);
    }

    let mut findings: Vec<Finding> = all_violations
        .iter()
        .filter(|qv| !matches!(qv.severity, Severity::Off))
        .map(|qv| Finding {
            file: config_path.to_string(),
            query_name: Some(qv.query_name.clone()),
            rule_id: qv.rule_id.to_string(),
            rule_name: None,
            rule_description: None,
            severity: qv.severity,
            message: qv.message.clone(),
            line: None,
            column: None,
            cwe: scythe_lint::reporters::extract_cwe(&qv.message),
            source: Some("check".to_string()),
        })
        .collect();

    if let Some(url) = opts.database_url.as_deref() {
        findings.extend(verify_against_database(url, &verifiable)?);
    }

    // Findings go to stdout (matching `scythe inspect`) so `--format json` can
    // be redirected to a file; progress messages stay on stderr.
    let mut out: Box<dyn std::io::Write> = match opts.output.as_deref() {
        Some(path) => Box::new(std::io::BufWriter::new(
            std::fs::File::create(path).map_err(|e| format!("failed to open '{}': {}", path, e))?,
        )),
        None => Box::new(std::io::stdout()),
    };
    emit_findings(
        format,
        "scythe-check",
        env!("CARGO_PKG_VERSION"),
        &findings,
        out.as_mut(),
    )?;
    out.flush().ok();

    let error_count = findings
        .iter()
        .filter(|f| matches!(f.severity, Severity::Error))
        .count();
    let warning_count = findings.iter().filter(|f| matches!(f.severity, Severity::Warn)).count();

    if error_count > 0 {
        return Err(format!("check: {} error(s), {} warning(s)", error_count, warning_count).into());
    }

    Ok(())
}

/// A `[[sql]]` block queued for live-database verification: its display
/// name, its configured engine (still in whatever alias form the user wrote
/// in `scythe.toml`, e.g. `"pg"` or `"cockroachdb"`), and its analyzed
/// queries.
#[derive(Debug)]
struct VerifiableBlock {
    name: String,
    engine: String,
    queries: Vec<scythe_core::analyzer::AnalyzedQuery>,
}

/// Whether per-query analysis results collected while checking a `[[sql]]`
/// block should be retained for later live-database verification.
///
/// `verify_against_database` is the only consumer of the retained
/// `AnalyzedQuery` data, and it only runs when `--database-url` is supplied.
/// Without it, `scythe check` runs entirely offline, so retaining (cloning
/// and keeping alive) every analyzed query for the rest of the process would
/// be pure waste. Kept as a small pure predicate, like
/// `partition_verifiable_blocks` below, so the retention decision is
/// unit-testable without a database, config file, or SQL source.
fn should_retain_for_verification(database_url: Option<&str>) -> bool {
    database_url.is_some()
}

/// Split verifiable blocks into ones whose engine speaks the PostgreSQL wire
/// protocol (eligible for live verification via `tokio-postgres`) and ones
/// that must be skipped.
///
/// Pure and database-free by design: `verify_against_database` is the only
/// caller that needs a live connection, so the classification logic that
/// decides *which* blocks reach it is kept separate and unit-testable
/// without a database.
fn partition_verifiable_blocks(blocks: &[VerifiableBlock]) -> (Vec<&VerifiableBlock>, Vec<&VerifiableBlock>) {
    blocks
        .iter()
        .partition(|b| scythe_codegen::backends::normalize_engine(&b.engine) == "postgresql")
}

/// Prepare every analyzed query against a live database and report where the
/// server disagrees with static inference.
///
/// Only PostgreSQL is supported today — it is the engine whose extended query
/// protocol lets us describe a statement without executing it. Other engines
/// are skipped with a warning rather than failing the run, so `--database-url`
/// stays harmless in a mixed-engine config.
fn verify_against_database(
    url: &str,
    verifiable: &[VerifiableBlock],
) -> Result<Vec<scythe_lint::reporters::Finding>, Box<dyn std::error::Error>> {
    let (pg_blocks, skipped_blocks) = partition_verifiable_blocks(verifiable);

    for block in &skipped_blocks {
        eprintln!(
            "[{}] Skipping database verification: engine '{}' is not PostgreSQL-compatible \
             (only PostgreSQL supports live verification); --database-url is ignored for this block.",
            block.name, block.engine
        );
    }

    if pg_blocks.is_empty() {
        return Ok(Vec::new());
    }

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .map_err(|e| format!("failed to connect to '{}': {}", redact_url_password(url), e))?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("scythe check: database connection error: {e}");
            }
        });

        let mut findings = Vec::new();
        for block in pg_blocks {
            eprintln!(
                "[{}] Verifying {} queries against the database...",
                block.name,
                block.queries.len()
            );
            findings.extend(scythe_inspect::verify_queries(&client, &block.name, &block.queries).await);
        }
        Ok(findings)
    })
}

/// Format Rust code using rustfmt if available.
/// If rustfmt is not found or fails, returns the original code unchanged.
/// This ensures that generated code is rustfmt-compliant without requiring
/// downstream users to run `cargo fmt` and create unnecessary diffs.
fn format_rust_code_if_possible(code: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let Ok(mut child) = Command::new("rustfmt")
        .arg("--edition=2024")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return code.to_string();
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(code.as_bytes());
    }

    match child.wait_with_output() {
        Ok(output) if output.status.success() => match String::from_utf8(output.stdout) {
            Ok(formatted) => formatted,
            Err(_) => code.to_string(),
        },
        _ => code.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_query_file_basic() {
        let content = "\
-- name: GetUser :one
SELECT * FROM users WHERE id = $1;

-- name: ListUsers :many
SELECT * FROM users;
";
        let blocks = split_query_file(content);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("GetUser"));
        assert!(blocks[1].contains("ListUsers"));
    }

    #[test]
    fn test_split_query_file_with_preamble() {
        let content = "\
-- This is a comment at the top
-- Another comment

-- name: GetUser :one
SELECT * FROM users WHERE id = $1;
";
        let blocks = split_query_file(content);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("GetUser"));
    }

    #[test]
    fn test_split_query_file_at_annotation() {
        let content = "\
-- @name GetUser :one
SELECT * FROM users WHERE id = $1;
";
        let blocks = split_query_file(content);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("GetUser"));
    }

    #[test]
    fn test_split_query_file_empty() {
        let content = "-- just a comment\n";
        let blocks = split_query_file(content);
        assert_eq!(blocks.len(), 0);
    }

    /// Without `--database-url`, `verify_against_database` never runs, so
    /// analyzed queries must not be retained for it.
    #[test]
    fn should_retain_for_verification_is_false_without_database_url() {
        assert!(!should_retain_for_verification(None));
    }

    /// With `--database-url`, `verify_against_database` is the consumer of
    /// the retained analyzed queries, so retention must still happen.
    #[test]
    fn should_retain_for_verification_is_true_with_database_url() {
        assert!(should_retain_for_verification(Some("postgres://localhost/db")));
    }

    fn verifiable_block(name: &str, engine: &str) -> VerifiableBlock {
        VerifiableBlock {
            name: name.to_string(),
            engine: engine.to_string(),
            queries: Vec::new(),
        }
    }

    /// A mixed-engine config (one postgres block, one mysql block) must keep
    /// the postgres block eligible for verification and route the mysql
    /// block to the skip list — never to the live-DB path. This is the pure,
    /// database-free half of the mixed-engine regression: it proves the
    /// filtering logic itself is correct, independent of a running database.
    #[test]
    fn partition_verifiable_blocks_splits_postgres_from_other_engines() {
        let blocks = vec![
            verifiable_block("pg_block", "postgresql"),
            verifiable_block("mysql_block", "mysql"),
        ];

        let (pg, skipped) = partition_verifiable_blocks(&blocks);

        assert_eq!(pg.len(), 1, "exactly one block must be eligible for verification");
        assert_eq!(pg[0].name, "pg_block");

        assert_eq!(skipped.len(), 1, "exactly one block must be skipped");
        assert_eq!(skipped[0].name, "mysql_block");
    }

    /// `postgres`, `pg`, and `cockroachdb` are aliases for the same
    /// PostgreSQL-wire-compatible engine and must all be treated as
    /// verifiable, matching `scythe-codegen`'s `normalize_engine` table.
    #[test]
    fn partition_verifiable_blocks_treats_postgres_aliases_as_verifiable() {
        let blocks = vec![
            verifiable_block("a", "postgres"),
            verifiable_block("b", "pg"),
            verifiable_block("c", "postgresql"),
            verifiable_block("d", "cockroachdb"),
        ];

        let (pg, skipped) = partition_verifiable_blocks(&blocks);

        assert_eq!(
            pg.len(),
            4,
            "all four postgres aliases must be verifiable, got: {:?}",
            pg
        );
        assert!(skipped.is_empty());
    }

    /// Non-PostgreSQL engines (mysql, sqlite, mssql, oracle, ...) must never
    /// reach live verification, since `verify_against_database` only speaks
    /// the PostgreSQL wire protocol.
    #[test]
    fn partition_verifiable_blocks_skips_non_postgres_engines() {
        let blocks = vec![
            verifiable_block("a", "mysql"),
            verifiable_block("b", "sqlite"),
            verifiable_block("c", "mssql"),
            verifiable_block("d", "oracle"),
        ];

        let (pg, skipped) = partition_verifiable_blocks(&blocks);

        assert!(pg.is_empty(), "no non-postgres engine may be verifiable, got: {:?}", pg);
        assert_eq!(skipped.len(), 4);
    }

    fn query_result(model_struct: Option<&str>, row_struct: Option<&str>, query_fn: Option<&str>) -> QueryResult {
        QueryResult {
            code: scythe_codegen::GeneratedCode {
                model_struct: model_struct.map(str::to_string),
                row_struct: row_struct.map(str::to_string),
                query_fn: query_fn.map(str::to_string),
                enum_def: None,
            },
            enums: Vec::new(),
        }
    }

    /// When `query_class_header()` is empty (e.g. `rust-sqlx`, which has no
    /// wrapping class), each query's model struct, row struct, and function
    /// must stay interleaved in per-query order rather than being grouped by
    /// kind — matching how `sqlx.rs` expects `impl` blocks to sit next to
    /// their row types.
    #[test]
    fn assemble_output_interleaves_per_query_when_class_header_is_empty() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        assert!(
            backend.query_class_header().is_empty(),
            "test assumes rust-sqlx has no query class header"
        );

        let results = vec![
            query_result(Some("MODEL_A"), Some("ROW_A"), Some("FN_A")),
            query_result(None, Some("ROW_B"), Some("FN_B")),
        ];

        let output = assemble_output(backend.as_ref(), &results);

        let pos = |needle: &str| {
            output
                .find(needle)
                .unwrap_or_else(|| panic!("missing '{needle}' in:\n{output}"))
        };
        let (model_a, row_a, fn_a, row_b, fn_b) =
            (pos("MODEL_A"), pos("ROW_A"), pos("FN_A"), pos("ROW_B"), pos("FN_B"));

        assert!(model_a < row_a, "MODEL_A must precede ROW_A within the first query");
        assert!(row_a < fn_a, "ROW_A must precede FN_A within the first query");
        assert!(
            fn_a < row_b,
            "the first query's FN_A must precede the second query's ROW_B (interleaved, not grouped)"
        );
        assert!(row_b < fn_b, "ROW_B must precede FN_B within the second query");
    }

    /// When `query_class_header()` is non-empty (e.g. `php-pdo`, which wraps
    /// query functions in `final class Queries { ... }`), all model/row
    /// structs across every query must come first, then the class header,
    /// then all query functions — never interleaved, since PHP methods must
    /// live inside the class body while types are declared outside it.
    #[test]
    fn assemble_output_groups_types_then_class_header_then_fns_when_non_empty() {
        let backend = get_backend("php-pdo", "postgresql").expect("php-pdo should support postgresql");
        let class_header = backend.query_class_header();
        assert!(
            !class_header.is_empty(),
            "test assumes php-pdo has a query class header"
        );

        let results = vec![
            query_result(Some("MODEL_A"), Some("ROW_A"), Some("FN_A")),
            query_result(None, Some("ROW_B"), Some("FN_B")),
        ];

        let output = assemble_output(backend.as_ref(), &results);

        let pos = |needle: &str| {
            output
                .find(needle)
                .unwrap_or_else(|| panic!("missing '{needle}' in:\n{output}"))
        };
        let (model_a, row_a, row_b, header, fn_a, fn_b) = (
            pos("MODEL_A"),
            pos("ROW_A"),
            pos("ROW_B"),
            pos(&class_header),
            pos("FN_A"),
            pos("FN_B"),
        );

        assert!(
            model_a < header && row_a < header && row_b < header,
            "all types must precede the class header"
        );
        assert!(
            header < fn_a && header < fn_b,
            "the class header must precede every query function"
        );
    }

    /// With no analyzed queries at all, there is nothing to write; the file
    /// must fall back to a placeholder comment rather than an empty string
    /// (an empty generated file reads as a build failure, not "no queries").
    #[test]
    fn assemble_output_falls_back_to_placeholder_when_results_are_empty() {
        // A hand-rolled backend (rather than a real one from `get_backend`)
        // is required here: every shipping backend overrides at least one of
        // file_header/file_footer/post_footer/query_class_header with
        // non-empty content, so none of them can produce a genuinely empty
        // `output_parts` to exercise this fallback. The manifest is cloned
        // from a real backend purely to satisfy the trait's `manifest()`
        // accessor, which this test path never calls.
        struct EmptyOutputBackend {
            manifest: scythe_backend::manifest::BackendManifest,
        }

        impl CodegenBackend for EmptyOutputBackend {
            fn name(&self) -> &str {
                "test-empty-output"
            }

            fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
                &self.manifest
            }

            fn generate_row_struct(
                &self,
                _query_name: &str,
                _columns: &[scythe_codegen::ResolvedColumn],
            ) -> Result<String, scythe_core::errors::ScytheError> {
                Ok(String::new())
            }

            fn generate_model_struct(
                &self,
                _table_name: &str,
                _columns: &[scythe_codegen::ResolvedColumn],
            ) -> Result<String, scythe_core::errors::ScytheError> {
                Ok(String::new())
            }

            fn generate_query_fn(
                &self,
                _analyzed: &AnalyzedQuery,
                _struct_name: &str,
                _columns: &[scythe_codegen::ResolvedColumn],
                _params: &[scythe_codegen::ResolvedParam],
            ) -> Result<String, scythe_core::errors::ScytheError> {
                Ok(String::new())
            }

            fn generate_enum_def(&self, _enum_info: &EnumInfo) -> Result<String, scythe_core::errors::ScytheError> {
                Ok(String::new())
            }

            fn generate_composite_def(
                &self,
                _composite: &scythe_core::analyzer::CompositeInfo,
            ) -> Result<String, scythe_core::errors::ScytheError> {
                Ok(String::new())
            }

            // file_header, file_footer, query_class_header, and post_footer
            // are intentionally left at their trait defaults (all empty),
            // which is exactly the condition this test needs.
        }

        let real = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let backend = EmptyOutputBackend {
            manifest: real.manifest().clone(),
        };

        let output = assemble_output(&backend, &[]);

        assert_eq!(output, "// No queries generated.\n");
    }
}
