//! Tests that verify generated Rust code is syntactically valid.
//! Uses `syn` to parse the output without needing sqlx/chrono as deps.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Returns the workspace root (two levels up from crate manifest dir).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn schema_dir(relative: &str) -> PathBuf {
    workspace_root().join("tests/schemas").join(relative)
}

/// A single piece of generated code (struct, function, or enum).
struct CodeFragment {
    query_name: String,
    kind: &'static str,
    code: String,
}

/// Result of generating code for a schema.
struct GenerationResult {
    /// All generated code combined (for content assertions).
    combined: String,
    /// Individual code fragments for per-item validation.
    fragments: Vec<CodeFragment>,
}

/// Helper: given a schema dir with scythe.toml, parse schemas and queries
/// through the library API and return individual code fragments.
fn generate_for_schema(relative_path: &str) -> GenerationResult {
    let schema_dir = schema_dir(relative_path);
    let config_path = schema_dir.join("scythe.toml");
    let config_str = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|_| panic!("missing config: {}", config_path.display()));

    let config: toml::Value = toml::from_str(&config_str).unwrap();
    let sql_blocks = config["sql"].as_array().unwrap();
    let sql_block = &sql_blocks[0];

    let schema_files: Vec<String> = sql_block["schema"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| schema_dir.join(s.as_str().unwrap()).display().to_string())
        .collect();

    let query_patterns: Vec<String> = sql_block["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| schema_dir.join(s.as_str().unwrap()).display().to_string())
        .collect();

    // Read schema files and build catalog
    let schema_contents: Vec<String> = schema_files
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();
    let catalog = scythe_core::catalog::Catalog::from_ddl(&schema_refs).unwrap();

    // Resolve query files via glob
    let mut query_file_paths = Vec::new();
    for pattern in &query_patterns {
        for entry in glob::glob(pattern).unwrap() {
            query_file_paths.push(entry.unwrap().display().to_string());
        }
    }
    query_file_paths.sort();

    let mut combined = String::new();
    combined.push_str(
        "// Auto-generated test output\n\
         #![allow(dead_code, unused_imports, clippy::all)]\n\n",
    );

    let mut fragments = Vec::new();
    let mut seen_enums = HashSet::new();
    let manifest = scythe_codegen::load_or_default_manifest().unwrap();

    for qf in &query_file_paths {
        let content = std::fs::read_to_string(qf).unwrap();
        let blocks = split_query_blocks(&content);
        for block in &blocks {
            let parsed = match scythe_core::parser::parse_query(block) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("parse error in {}: {}", qf, e);
                    continue;
                }
            };
            let analyzed = match scythe_core::analyzer::analyze(&catalog, &parsed) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("analyze error in {}: {}", qf, e);
                    continue;
                }
            };

            let query_name = analyzed.name.clone();

            // Collect enum definitions (deduplicated)
            for e in &analyzed.enums {
                if seen_enums.insert(e.sql_name.clone()) {
                    let def = scythe_codegen::generate_single_enum_def(e, &manifest);
                    combined.push_str(&def);
                    combined.push_str("\n\n");
                    fragments.push(CodeFragment {
                        query_name: format!("enum:{}", e.sql_name),
                        kind: "enum",
                        code: def,
                    });
                }
            }

            match scythe_codegen::generate(&analyzed) {
                Ok(code) => {
                    if let Some(ref s) = code.model_struct {
                        combined.push_str(s);
                        combined.push_str("\n\n");
                        fragments.push(CodeFragment {
                            query_name: query_name.clone(),
                            kind: "model_struct",
                            code: s.clone(),
                        });
                    }
                    if let Some(ref s) = code.row_struct {
                        combined.push_str(s);
                        combined.push_str("\n\n");
                        fragments.push(CodeFragment {
                            query_name: query_name.clone(),
                            kind: "row_struct",
                            code: s.clone(),
                        });
                    }
                    if let Some(ref s) = code.query_fn {
                        combined.push_str(s);
                        combined.push_str("\n\n");
                        fragments.push(CodeFragment {
                            query_name: query_name.clone(),
                            kind: "query_fn",
                            code: s.clone(),
                        });
                    }
                }
                Err(e) => {
                    eprintln!("codegen error in {}: {}", qf, e);
                }
            }
        }
    }

    GenerationResult {
        combined,
        fragments,
    }
}

/// Split a SQL file into individual query blocks (same logic as commands/shared.rs).
fn split_query_blocks(content: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current_block: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim_start();
        let is_annotation = trimmed.starts_with("-- name:") || trimmed.starts_with("-- @name");

        if is_annotation {
            if let Some(block) = current_block.take() {
                blocks.push(block);
            }
            current_block = Some(String::from(line));
        } else if let Some(ref mut block) = current_block {
            block.push('\n');
            block.push_str(line);
        }
    }

    if let Some(block) = current_block {
        blocks.push(block);
    }

    blocks
}

/// Validate each code fragment individually with syn, returning (valid_count, invalid details).
fn validate_fragments(fragments: &[CodeFragment]) -> (usize, Vec<String>) {
    let mut valid = 0;
    let mut invalid = Vec::new();
    let header = "#![allow(dead_code, unused_imports, clippy::all)]\n";

    for frag in fragments {
        // Wrap the fragment in a file-level context for syn to parse
        let test_code = format!("{}{}", header, frag.code);
        match syn::parse_file(&test_code) {
            Ok(_) => valid += 1,
            Err(e) => {
                invalid.push(format!(
                    "[{}:{}] {}: {}",
                    frag.query_name,
                    frag.kind,
                    e,
                    frag.code.lines().next().unwrap_or("")
                ));
            }
        }
    }

    (valid, invalid)
}

#[test]
fn test_basemind_generates_valid_rust() {
    let result = generate_for_schema("simple/basemind");
    assert!(
        !result.fragments.is_empty(),
        "should generate code fragments"
    );

    let (valid, invalid) = validate_fragments(&result.fragments);
    let total = result.fragments.len();

    // Report invalid fragments for debugging
    for msg in &invalid {
        eprintln!("INVALID: {}", msg);
    }

    // At least 90% of fragments should be valid Rust
    let valid_pct = (valid as f64 / total as f64) * 100.0;
    assert!(
        valid_pct >= 90.0,
        "at least 90% of generated code should be valid Rust, got {:.1}% ({}/{} valid)",
        valid_pct,
        valid,
        total
    );

    // Should generate a substantial number of items
    assert!(valid > 10, "should have many valid items, got {}", valid);
}

#[test]
fn test_pagila_generates_valid_rust() {
    let result = generate_for_schema("medium/pagila");
    assert!(
        !result.fragments.is_empty(),
        "should generate code fragments"
    );

    let (valid, invalid) = validate_fragments(&result.fragments);
    let total = result.fragments.len();

    for msg in &invalid {
        eprintln!("INVALID: {}", msg);
    }

    let valid_pct = (valid as f64 / total as f64) * 100.0;
    assert!(
        valid_pct >= 90.0,
        "at least 90% of generated code should be valid Rust, got {:.1}% ({}/{} valid)",
        valid_pct,
        valid,
        total
    );

    assert!(valid > 10, "should have many valid items, got {}", valid);
}

#[test]
fn test_generated_code_contains_expected_structs() {
    let result = generate_for_schema("simple/basemind");

    // Basemind schema has user_account queries
    assert!(
        result.combined.contains("fn create_user_account")
            || result.combined.contains("CreateUserAccount"),
        "should generate code for CreateUserAccount query"
    );
    assert!(
        result.combined.contains("fn delete_user_account"),
        "should generate function for DeleteUserAccount :exec query"
    );
    assert!(
        result.combined.contains("fn retrieve_user_account_by_id")
            || result.combined.contains("RetrieveUserAccountById"),
        "should generate code for RetrieveUserAccountByID query"
    );
}

#[test]
fn test_pagila_generated_code_contains_enums() {
    let result = generate_for_schema("medium/pagila");

    // Pagila schema has mpaa_rating enum
    assert!(
        result.combined.contains("MpaaRating") || result.combined.contains("mpaa_rating"),
        "should generate enum for mpaa_rating type"
    );
}
