use std::borrow::Cow;
use std::path::Path;

use serde::Deserialize;

use ahash::AHashSet;

use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case,
};
use scythe_codegen::{
    CodegenBackend, RbsEnumInfo, RbsGenerationContext, RbsQueryInfo, TypeOverride,
    generate_single_enum_def_with_backend, generate_with_backend_and_overrides, get_backend,
};
use scythe_core::analyzer::{AnalyzedQuery, EnumInfo, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::{QueryCommand, parse_query_with_dialect};

use super::shared::{resolve_globs, split_query_file};

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

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
    let default_output = sql_config
        .output
        .clone()
        .unwrap_or_else(|| "generated".to_string());

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

// ---------------------------------------------------------------------------
// Generate command
// ---------------------------------------------------------------------------

pub fn run_generate(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Read and parse config
    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig = toml::from_str(&config_str)
        .map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    // 2. Process each SQL block
    for sql_config in &config.sql {
        eprintln!("[{}] Parsing schema...", sql_config.name);

        // 3. Resolve schema files via glob
        let schema_files = resolve_globs(&sql_config.schema)?;

        // 4. Read all schema SQL
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| {
                std::fs::read_to_string(p)
                    .map_err(|e| format!("failed to read schema file '{}': {}", p, e))
            })
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

        // 5. Build catalog with the configured dialect
        let dialect = SqlDialect::from_str(&sql_config.engine).unwrap_or(SqlDialect::PostgreSQL);
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &dialect)?;

        // 6. Resolve query files via glob
        let query_files = resolve_globs(&sql_config.queries)?;

        // 7. Split each query file into individual query blocks
        let mut all_query_blocks = Vec::new();
        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;
            let blocks = split_query_file(&content);
            all_query_blocks.extend(blocks);
        }

        eprintln!(
            "[{}] Analyzing {} queries...",
            sql_config.name,
            all_query_blocks.len()
        );

        // 8. Parse and analyze all queries once
        let mut analyzed_queries: Vec<AnalyzedQuery> = Vec::new();
        for block in &all_query_blocks {
            let parsed = parse_query_with_dialect(block, &dialect)?;
            let analyzed = analyze(&catalog, &parsed)?;
            analyzed_queries.push(analyzed);
        }

        // 9. Convert type overrides
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

        // 10. Generate code for each backend target
        let gen_targets = resolve_gen_targets(sql_config);

        for target in &gen_targets {
            let mut backend = get_backend(&target.backend, &sql_config.engine).map_err(|e| {
                format!(
                    "backend '{}' with engine '{}': {}",
                    target.backend, sql_config.engine, e
                )
            })?;

            if !target.options.is_empty() {
                backend.apply_options(&target.options).map_err(|e| {
                    format!("backend '{}' apply_options failed: {}", target.backend, e)
                })?;
            }

            generate_for_backend(
                &sql_config.name,
                &*backend,
                &analyzed_queries,
                &target.output,
                &overrides,
            )?;
        }
    }

    eprintln!("Done.");
    Ok(())
}

/// Generate output for a single backend target.
fn generate_for_backend(
    config_name: &str,
    backend: &dyn CodegenBackend,
    analyzed_queries: &[AnalyzedQuery],
    output_dir: &str,
    overrides: &[TypeOverride],
) -> Result<(), Box<dyn std::error::Error>> {
    struct QueryResult {
        code: scythe_codegen::GeneratedCode,
        enums: Vec<EnumInfo>,
    }

    let mut results: Vec<QueryResult> = Vec::new();
    for analyzed in analyzed_queries {
        let enums = analyzed.enums.clone();
        let code = generate_with_backend_and_overrides(analyzed, backend, overrides)?;
        results.push(QueryResult { code, enums });
    }

    // Deduplicate enums across all queries
    let mut seen_enums = AHashSet::new();
    let mut unique_enum_defs: Vec<String> = Vec::new();
    for result in &results {
        for info in &result.enums {
            if seen_enums.insert(info.sql_name.clone())
                && let Ok(def) = generate_single_enum_def_with_backend(info, backend)
            {
                unique_enum_defs.push(def);
            }
        }
    }

    // Build output: header → enums → per-query structs/functions
    let mut output_parts: Vec<String> = Vec::new();

    // File header from backend
    let header = backend.file_header();
    if !header.is_empty() {
        output_parts.push(header);
    }

    // Deduplicated enums
    for def in &unique_enum_defs {
        output_parts.push(def.clone());
    }

    // Per-query code (structs + functions, skip enum_def since we already deduplicated above)
    let class_header = backend.query_class_header();
    if class_header.is_empty() {
        // No class wrapper: interleave structs and functions as before
        for result in &results {
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
        // Class wrapper mode: emit all type definitions first, then class
        // header, then all query functions (so types stay outside the class).
        for result in &results {
            if let Some(ref s) = result.code.model_struct {
                output_parts.push(s.clone());
            }
            if let Some(ref s) = result.code.row_struct {
                output_parts.push(s.clone());
            }
        }
        output_parts.push(class_header);
        for result in &results {
            if let Some(ref s) = result.code.query_fn {
                output_parts.push(s.clone());
            }
        }
    }

    // File footer (e.g., closing brace for C# class wrapper)
    let footer = backend.file_footer();
    if !footer.is_empty() {
        output_parts.push(footer);
    }

    // Post-footer code (e.g., top-level C# extension methods)
    let post_footer = backend.post_footer();
    if !post_footer.is_empty() {
        output_parts.push(post_footer);
    }

    // Determine output filename from backend manifest
    let ext = &backend.manifest().backend.file_extension;
    let filename = format!("queries.{}", ext);

    // Write output
    let out_path = Path::new(output_dir);
    std::fs::create_dir_all(out_path)
        .map_err(|e| format!("failed to create output dir '{}': {}", output_dir, e))?;

    let output_file = out_path.join(&filename);
    let output_content = if output_parts.is_empty() {
        "// No queries generated.\n".to_string()
    } else {
        output_parts.join("\n\n") + "\n"
    };

    std::fs::write(&output_file, &output_content).map_err(|e| {
        format!(
            "failed to write output file '{}': {}",
            output_file.display(),
            e
        )
    })?;

    eprintln!(
        "[{}] Writing {} output to {}",
        config_name,
        backend.name(),
        output_file.display()
    );

    // Generate RBS type signature file for Ruby backends
    generate_rbs_if_supported(config_name, backend, analyzed_queries, overrides, out_path)?;

    Ok(())
}

/// Determine the struct name for a query, matching the logic in scythe_codegen.
fn determine_struct_name(
    analyzed: &AnalyzedQuery,
    naming: &scythe_backend::naming::NamingConfig,
) -> String {
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
    // Quick check: only Ruby backends produce RBS files. Test with an empty context.
    let empty_context = RbsGenerationContext {
        queries: vec![],
        enums: vec![],
    };
    if backend.generate_rbs_file(&empty_context).is_none() {
        return Ok(());
    }

    let manifest = backend.manifest();
    let naming = &manifest.naming;

    // Build RBS query info for each analyzed query
    let mut rbs_queries: Vec<RbsQueryInfo> = Vec::new();
    let mut seen_enums = AHashSet::new();
    let mut rbs_enums: Vec<RbsEnumInfo> = Vec::new();

    for analyzed in analyzed_queries {
        let source_table = analyzed.source_table.as_deref().unwrap_or("");
        let columns = scythe_codegen::resolve::resolve_columns(
            &analyzed.columns,
            manifest,
            overrides,
            source_table,
        )?;
        let params = scythe_codegen::resolve::resolve_params(
            &analyzed.params,
            manifest,
            overrides,
            source_table,
        )?;

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
            struct_name: if needs_struct {
                Some(struct_name)
            } else {
                None
            },
            columns,
            params,
            command,
        });

        // Collect enum info
        for enum_info in &analyzed.enums {
            if seen_enums.insert(enum_info.sql_name.clone()) {
                let type_name = enum_type_name(&enum_info.sql_name, naming);
                let values: Vec<String> = enum_info
                    .values
                    .iter()
                    .map(|v| enum_variant_name(v, naming))
                    .collect();
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

// ---------------------------------------------------------------------------
// Check command (validate without generating)
// ---------------------------------------------------------------------------

pub fn run_check(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use scythe_lint::{LintContext, LintEngine, QueryViolation, Severity, default_registry};

    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: ScytheConfig = toml::from_str(&config_str)
        .map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    // Build lint engine from config
    let mut registry = default_registry();
    if let Some(ref lint_config) = config.lint {
        registry.apply_config(lint_config);
    }
    let engine = LintEngine::new(registry);

    let mut all_violations: Vec<QueryViolation> = Vec::new();

    for sql_config in &config.sql {
        eprintln!("[{}] Parsing schema...", sql_config.name);

        let schema_files = resolve_globs(&sql_config.schema)?;
        let schema_contents: Vec<String> = schema_files
            .iter()
            .map(|p| {
                std::fs::read_to_string(p)
                    .map_err(|e| format!("failed to read schema file '{}': {}", p, e))
            })
            .collect::<Result<_, _>>()?;
        let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();

        let dialect = SqlDialect::from_str(&sql_config.engine).unwrap_or(SqlDialect::PostgreSQL);
        let catalog = Catalog::from_ddl_with_dialect(&schema_refs, &dialect)?;

        let query_files = resolve_globs(&sql_config.queries)?;
        let mut all_query_blocks = Vec::new();
        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;
            let blocks = split_query_file(&content);
            all_query_blocks.extend(blocks);
        }

        eprintln!(
            "[{}] Checking {} queries...",
            sql_config.name,
            all_query_blocks.len()
        );

        let mut query_names: Vec<String> = Vec::new();

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
        }

        // Check catalog-level rules
        let cat_violations = engine.check_catalog(&catalog);
        for (v, sev) in cat_violations {
            all_violations.push(QueryViolation {
                query_name: String::new(),
                rule_id: v.rule_id.clone(),
                severity: sev,
                message: v.message,
            });
        }

        // Duplicate query name detection (SC-C03)
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

    // Print diagnostics
    let mut error_count = 0usize;
    let mut warning_count = 0usize;
    for qv in &all_violations {
        match qv.severity {
            Severity::Error => {
                error_count += 1;
                eprintln!(
                    "error: [{}] {} (query: {})",
                    qv.rule_id, qv.message, qv.query_name
                );
            }
            Severity::Warn => {
                warning_count += 1;
                eprintln!(
                    "warning: [{}] {} (query: {})",
                    qv.rule_id, qv.message, qv.query_name
                );
            }
            Severity::Off => {}
        }
    }

    if error_count > 0 {
        return Err(format!(
            "lint: {} error(s), {} warning(s)",
            error_count, warning_count
        )
        .into());
    }
    if warning_count > 0 {
        eprintln!("lint: {} warning(s)", warning_count);
    }

    eprintln!("Check passed.");
    Ok(())
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
}
