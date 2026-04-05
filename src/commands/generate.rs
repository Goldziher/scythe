use std::borrow::Cow;
use std::path::Path;

use serde::Deserialize;

use ahash::AHashSet;

use crate::analyzer::{EnumInfo, analyze};
use crate::catalog::Catalog;
use crate::codegen::{generate, generate_single_enum_def, load_or_default_manifest};
use crate::parser::parse_query;

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ScytheConfig {
    #[allow(dead_code)]
    scythe: ScytheMeta,
    sql: Vec<SqlConfig>,
    #[serde(default)]
    pub lint: Option<crate::lint::types::LintConfig>,
}

#[derive(Debug, Deserialize)]
struct ScytheMeta {
    #[allow(dead_code)]
    version: String,
}

#[derive(Debug, Deserialize)]
struct SqlConfig {
    name: String,
    #[allow(dead_code)]
    engine: String,
    schema: Vec<String>,
    queries: Vec<String>,
    output: String,
    #[allow(dead_code)]
    #[serde(rename = "gen")]
    gen_config: Option<GenConfig>,
    #[allow(dead_code)]
    type_overrides: Option<Vec<TypeOverrideConfig>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GenConfig {
    rust: Option<RustGenConfig>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RustGenConfig {
    target: String,
    derive: Option<Vec<String>>,
    serde: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TypeOverrideConfig {
    column: Option<String>,
    db_type: Option<String>,
    #[serde(rename = "type")]
    neutral_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Glob resolution
// ---------------------------------------------------------------------------

fn resolve_globs(patterns: &[String]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut paths = Vec::new();
    for pattern in patterns {
        let matches: Vec<_> = glob::glob(pattern)?.collect::<Result<Vec<_>, _>>()?;
        if matches.is_empty() {
            eprintln!("warning: glob pattern '{}' matched no files", pattern);
        }
        for path in matches {
            paths.push(path.display().to_string());
        }
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Query file splitting
// ---------------------------------------------------------------------------

/// Splits a .sql file containing multiple queries separated by `-- name:` or
/// `-- @name` annotations. Returns one string per query block (annotation +
/// SQL). Content before the first annotation is discarded.
fn split_query_file(content: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current_block: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim_start();
        let is_annotation = trimmed.starts_with("-- name:") || trimmed.starts_with("-- @name");

        if is_annotation {
            // Flush previous block
            if let Some(block) = current_block.take() {
                blocks.push(block);
            }
            current_block = Some(String::from(line));
        } else if let Some(ref mut block) = current_block {
            block.push('\n');
            block.push_str(line);
        }
        // Lines before the first annotation are silently dropped.
    }

    // Flush the last block
    if let Some(block) = current_block {
        blocks.push(block);
    }

    blocks
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

        // 5. Build catalog
        let catalog = Catalog::from_ddl(&schema_refs)?;

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

        // 8. Parse and analyze each query, collect results
        struct QueryResult {
            code: crate::codegen::GeneratedCode,
            enums: Vec<EnumInfo>,
        }

        let mut results: Vec<QueryResult> = Vec::new();

        for block in &all_query_blocks {
            let parsed = parse_query(block)?;
            let analyzed = analyze(&catalog, &parsed)?;
            let enums = analyzed.enums.clone();
            let code = generate(&analyzed)?;
            results.push(QueryResult { code, enums });
        }

        // 9. Deduplicate enums across all queries and generate individual defs
        let manifest = load_or_default_manifest()?;
        let mut seen_enums = AHashSet::new();
        let mut unique_enum_defs: Vec<String> = Vec::new();
        for result in &results {
            for info in &result.enums {
                if seen_enums.insert(info.sql_name.clone()) {
                    unique_enum_defs.push(generate_single_enum_def(info, &manifest));
                }
            }
        }

        // 10. Build output: header → enums → per-query structs/functions
        let mut output_parts: Vec<String> = Vec::new();

        // File header
        output_parts.push("// Auto-generated by scythe. Do not edit.\n#![allow(dead_code, unused_imports, clippy::all)]".to_string());

        // Deduplicated enums
        for def in &unique_enum_defs {
            output_parts.push(def.clone());
        }

        // Per-query code (structs + functions, skip enum_def)
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

        // 11. Write output
        let output_dir = Path::new(&sql_config.output);
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("failed to create output dir '{}': {}", sql_config.output, e))?;

        let output_file = output_dir.join("queries.rs");
        let output_content = if output_parts.len() <= 1 {
            String::from("// No queries generated.\n")
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
            "[{}] Writing output to {}",
            sql_config.name,
            output_file.display()
        );
    }

    eprintln!("Done.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Check command (validate without generating)
// ---------------------------------------------------------------------------

pub fn run_check(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use crate::lint::{LintContext, LintEngine, QueryViolation, Severity, default_registry};

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

        let catalog = Catalog::from_ddl(&schema_refs)?;

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
            let parsed = parse_query(block)?;
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
