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
    /// Every query block that failed to parse, analyze, or generate code,
    /// each recorded as `"<file>: <stage> error: <message>"`.
    ///
    /// A block landing here used to be silently dropped: the old code
    /// `eprintln!`'d the error and `continue`'d, so it contributed to
    /// neither the numerator nor the denominator of `validate_fragments`'s
    /// percentage. That let up to every query in a schema fail codegen
    /// outright while `valid_pct` still read 100% on whatever handful of
    /// fragments survived -- see #161. Callers must assert this is empty.
    pipeline_failures: Vec<String>,
}

/// Helper: given a schema dir with scythe.toml, parse schemas and queries
/// through the library API and return individual code fragments.
fn generate_for_schema(relative_path: &str) -> GenerationResult {
    let schema_dir = schema_dir(relative_path);
    let config_path = schema_dir.join("scythe.toml");
    let config_str =
        std::fs::read_to_string(&config_path).unwrap_or_else(|_| panic!("missing config: {}", config_path.display()));

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

    let schema_contents: Vec<String> = schema_files
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    let schema_refs: Vec<&str> = schema_contents.iter().map(|s| s.as_str()).collect();
    let catalog = scythe_core::catalog::Catalog::from_ddl(&schema_refs).unwrap();

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
    let mut pipeline_failures = Vec::new();
    let mut seen_enums = HashSet::new();
    let manifest = scythe_codegen::load_or_default_manifest().unwrap();

    for qf in &query_file_paths {
        let content = std::fs::read_to_string(qf).unwrap();
        let blocks = split_query_blocks(&content);
        for block in &blocks {
            let parsed = match scythe_core::parser::parse_query(block) {
                Ok(p) => p,
                Err(e) => {
                    pipeline_failures.push(format!("{qf}: parse error: {e}"));
                    continue;
                }
            };
            let analyzed = match scythe_core::analyzer::analyze(&catalog, &parsed) {
                Ok(a) => a,
                Err(e) => {
                    pipeline_failures.push(format!("{qf}: analyze error: {e}"));
                    continue;
                }
            };

            let query_name = analyzed.name.clone();

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
                    // Ahead of the row struct that names them: this file is
                    // assembled in emission order and then type-checked as a
                    // unit, so a nested struct referenced before it is
                    // declared would fail here for the wrong reason.
                    for def in &code.nested_struct_defs {
                        combined.push_str(&def.code);
                        combined.push_str("\n\n");
                        fragments.push(CodeFragment {
                            query_name: query_name.clone(),
                            kind: "nested_struct",
                            code: def.code.clone(),
                        });
                    }
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
                    pipeline_failures.push(format!("{qf}: codegen error: {e}"));
                }
            }
        }
    }

    GenerationResult {
        combined,
        fragments,
        pipeline_failures,
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

/// Validate each code fragment individually with syn, returning the
/// fragments that failed to parse.
///
/// No percentage, no floor: a fragment that fails to parse as Rust is a
/// defect regardless of how many others succeeded, so the caller asserts
/// this is empty rather than tolerating a fraction of it. See #161 -- the
/// previous `valid_pct >= 90.0` accepted up to 10% syntactically broken
/// output, and the drops handled in `generate_for_schema` (now surfaced via
/// `pipeline_failures`) meant the *denominator* itself could shrink without
/// bound, so a backend regression that broke almost everything could still
/// read as "100% of the survivors were valid".
fn validate_fragments(fragments: &[CodeFragment]) -> Vec<String> {
    let mut invalid = Vec::new();
    let header = "#![allow(dead_code, unused_imports, clippy::all)]\n";

    for frag in fragments {
        let test_code = format!("{}{}", header, frag.code);
        if let Err(e) = syn::parse_file(&test_code) {
            invalid.push(format!(
                "[{}:{}] {}: {}",
                frag.query_name,
                frag.kind,
                e,
                frag.code.lines().next().unwrap_or("")
            ));
        }
    }

    invalid
}

/// `simple/basemind`'s fixed number of generated fragments (enums, structs
/// and query functions combined) for its current schema and queries. Pinned
/// exactly, not as a floor: `generate_for_schema` no longer silently drops a
/// query block that fails to parse/analyze/generate (see
/// `pipeline_failures`), so if this count ever changes it means the fixture
/// itself changed and this constant must be updated alongside it -- not that
/// some queries quietly stopped producing fragments.
const BASEMIND_EXPECTED_FRAGMENTS: usize = 129;

/// `medium/pagila`'s fixed fragment count. See `BASEMIND_EXPECTED_FRAGMENTS`.
const PAGILA_EXPECTED_FRAGMENTS: usize = 36;

#[test]
fn test_basemind_generates_valid_rust() {
    let result = generate_for_schema("simple/basemind");
    assert!(
        result.pipeline_failures.is_empty(),
        "every query block must parse, analyze, and generate code; failures:\n{}",
        result.pipeline_failures.join("\n")
    );
    assert_eq!(
        result.fragments.len(),
        BASEMIND_EXPECTED_FRAGMENTS,
        "fragment count drifted from the pinned expectation -- update \
         BASEMIND_EXPECTED_FRAGMENTS only if the basemind fixture itself changed"
    );

    let invalid = validate_fragments(&result.fragments);
    for msg in &invalid {
        eprintln!("INVALID: {}", msg);
    }
    assert!(
        invalid.is_empty(),
        "all {} generated fragments must be valid Rust, {} were not:\n{}",
        result.fragments.len(),
        invalid.len(),
        invalid.join("\n")
    );
}

#[test]
fn test_pagila_generates_valid_rust() {
    let result = generate_for_schema("medium/pagila");
    assert!(
        result.pipeline_failures.is_empty(),
        "every query block must parse, analyze, and generate code; failures:\n{}",
        result.pipeline_failures.join("\n")
    );
    assert_eq!(
        result.fragments.len(),
        PAGILA_EXPECTED_FRAGMENTS,
        "fragment count drifted from the pinned expectation -- update \
         PAGILA_EXPECTED_FRAGMENTS only if the pagila fixture itself changed"
    );

    let invalid = validate_fragments(&result.fragments);
    for msg in &invalid {
        eprintln!("INVALID: {}", msg);
    }
    assert!(
        invalid.is_empty(),
        "all {} generated fragments must be valid Rust, {} were not:\n{}",
        result.fragments.len(),
        invalid.len(),
        invalid.join("\n")
    );
}

#[test]
fn test_generated_code_contains_expected_structs() {
    let result = generate_for_schema("simple/basemind");

    // Each assertion checks for the query *function* itself, not just a
    // struct or the SQL-side query name that happens to share a substring
    // with it. The two used to also accept `contains("CreateUserAccount")`/
    // `contains("RetrieveUserAccountById")` -- the query name, which
    // `pub struct CreateUserAccountRow` alone satisfies with no `fn` ever
    // generated. See #161.
    assert!(
        result.combined.contains("fn create_user_account"),
        "should generate a function for the CreateUserAccount query"
    );
    assert!(
        result.combined.contains("fn delete_user_account"),
        "should generate function for DeleteUserAccount :exec query"
    );
    assert!(
        result.combined.contains("fn retrieve_user_account_by_id"),
        "should generate a function for the RetrieveUserAccountByID query"
    );
}

#[test]
fn test_pagila_generated_code_contains_enums() {
    let result = generate_for_schema("medium/pagila");

    // The exact declaration line `generate_single_enum_def` emits (see
    // `scythe_codegen::lib::generate_single_enum_def`), not just a substring
    // match. The old check also accepted `contains("mpaa_rating")`, which the
    // `#[sqlx(type_name = "mpaa_rating", ...)]` attribute alone satisfies
    // with no `pub enum MpaaRating` ever generated -- the same disjunct
    // problem as the query-name checks above. See #161.
    assert!(
        result.combined.contains("pub enum MpaaRating {"),
        "should generate a Rust enum for the mpaa_rating type"
    );
}
