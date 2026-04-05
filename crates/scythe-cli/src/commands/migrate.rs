use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use regex::Regex;
use serde::Deserialize;

use scythe_core::errors::{ErrorCode, ScytheError};

// ---------------------------------------------------------------------------
// sqlc config model
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SqlcConfig {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    plugins: Vec<SqlcPlugin>,
    #[serde(default)]
    sql: Vec<SqlcSqlEntry>,
    // v1 format
    #[serde(default)]
    packages: Vec<SqlcPackage>,
}

#[derive(Debug, Deserialize)]
struct SqlcPlugin {
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    wasm: Option<SqlcWasm>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SqlcWasm {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SqlcSqlEntry {
    #[serde(default)]
    schema: Option<SqlcStringOrList>,
    #[serde(default)]
    queries: Option<SqlcStringOrList>,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    codegen: Vec<SqlcCodegen>,
    #[serde(default, rename = "gen")]
    gen_block: Option<SqlcGen>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum SqlcStringOrList {
    Single(String),
    List(Vec<String>),
}

impl SqlcStringOrList {
    fn to_vec(&self) -> Vec<String> {
        match self {
            SqlcStringOrList::Single(s) => vec![s.clone()],
            SqlcStringOrList::List(v) => v.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SqlcCodegen {
    #[serde(default)]
    plugin: Option<String>,
    #[serde(default)]
    out: Option<String>,
    #[serde(default)]
    options: Option<SqlcCodegenOptions>,
}

#[derive(Debug, Deserialize)]
struct SqlcCodegenOptions {
    #[serde(default, rename = "crate")]
    crate_name: Option<String>,
    #[serde(default)]
    derive: Option<SqlcDerive>,
    #[serde(default)]
    overrides: Option<Vec<SqlcOverride>>,
}

#[derive(Debug, Deserialize)]
struct SqlcDerive {
    #[serde(default)]
    row: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SqlcOverride {
    #[serde(default)]
    column: Option<String>,
    #[serde(default, rename = "type")]
    type_name: Option<String>,
}

/// v2 gen block (used when codegen is absent)
#[derive(Debug, Deserialize)]
struct SqlcGen {
    #[serde(default)]
    go: Option<SqlcGenTarget>,
    #[serde(default)]
    kotlin: Option<SqlcGenTarget>,
    #[serde(default)]
    python: Option<SqlcGenTarget>,
}

#[derive(Debug, Deserialize)]
struct SqlcGenTarget {
    #[serde(default)]
    out: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    package: Option<String>,
}

/// v1 format packages
#[derive(Debug, Deserialize)]
struct SqlcPackage {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    queries: Option<String>,
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    engine: Option<String>,
}

// ---------------------------------------------------------------------------
// scythe config model (for serialisation via toml)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct ScytheConfig {
    scythe: ScytheMeta,
    sql: Vec<ScytheSqlBlock>,
}

#[derive(Debug, serde::Serialize)]
struct ScytheMeta {
    version: String,
}

#[derive(Debug, serde::Serialize)]
struct ScytheSqlBlock {
    name: String,
    engine: String,
    schema: Vec<String>,
    queries: Vec<String>,
    output: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "gen")]
    gen_block: Option<BTreeMap<String, ScytheGenTarget>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    type_overrides: Vec<ScytheTypeOverride>,
}

#[derive(Debug, serde::Serialize)]
struct ScytheGenTarget {
    target: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    derive: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct ScytheTypeOverride {
    column: String,
    #[serde(rename = "type")]
    type_name: String,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Turn a path that might be a directory into a glob pattern for .sql files.
fn ensure_glob_pattern(p: &str) -> String {
    if p.contains('*') || p.ends_with(".sql") {
        return p.to_string();
    }
    let trimmed = p.trim_end_matches('/');
    format!("{trimmed}/*.sql")
}

fn internal(msg: impl Into<String>) -> ScytheError {
    ScytheError::new(ErrorCode::InternalError, msg)
}

// ---------------------------------------------------------------------------
// config conversion
// ---------------------------------------------------------------------------

fn convert_config(sqlc: &SqlcConfig, base_dir: &Path) -> Result<String, ScytheError> {
    let mut sql_blocks: Vec<ScytheSqlBlock> = Vec::new();

    let version = sqlc.version.as_deref().unwrap_or("2");

    if version == "1" || (!sqlc.packages.is_empty() && sqlc.sql.is_empty()) {
        // v1 format: packages
        for (idx, pkg) in sqlc.packages.iter().enumerate() {
            let name = pkg.name.clone().unwrap_or_else(|| {
                if sqlc.packages.len() == 1 {
                    "main".to_string()
                } else {
                    format!("sql_{idx}")
                }
            });
            let engine = pkg
                .engine
                .clone()
                .unwrap_or_else(|| "postgresql".to_string());
            let schema = pkg
                .schema
                .as_ref()
                .map(|s| vec![s.clone()])
                .unwrap_or_default();
            let queries: Vec<String> = pkg
                .queries
                .as_ref()
                .map(|s| vec![ensure_glob_pattern(s)])
                .unwrap_or_default();
            let output = pkg.path.clone().unwrap_or_else(|| "generated".to_string());

            sql_blocks.push(ScytheSqlBlock {
                name,
                engine,
                schema,
                queries,
                output,
                gen_block: None,
                type_overrides: Vec::new(),
            });
        }
    } else {
        // v2 format
        for (idx, entry) in sqlc.sql.iter().enumerate() {
            let engine = entry
                .engine
                .clone()
                .unwrap_or_else(|| "postgresql".to_string());

            let schema: Vec<String> = entry
                .schema
                .as_ref()
                .map(|v| v.to_vec())
                .unwrap_or_default();

            let queries: Vec<String> = entry
                .queries
                .as_ref()
                .map(|v| v.to_vec())
                .unwrap_or_default()
                .into_iter()
                .map(|p| ensure_glob_pattern(&p))
                .collect();

            let mut output = String::new();
            let mut gen_map: BTreeMap<String, ScytheGenTarget> = BTreeMap::new();
            let mut overrides: Vec<ScytheTypeOverride> = Vec::new();

            // Extract from codegen entries (v2 with plugins)
            for cg in &entry.codegen {
                if let Some(out) = &cg.out {
                    output = out.clone();
                }
                let lang = cg.plugin.clone().unwrap_or_else(|| "rust".to_string());
                let target = cg
                    .options
                    .as_ref()
                    .and_then(|o| o.crate_name.clone())
                    .unwrap_or_else(|| "tokio-postgres".to_string());
                let derive = cg
                    .options
                    .as_ref()
                    .and_then(|o| o.derive.as_ref())
                    .map(|d| d.row.clone())
                    .unwrap_or_default();

                gen_map.insert(lang, ScytheGenTarget { target, derive });

                if let Some(opts) = &cg.options
                    && let Some(ovs) = &opts.overrides
                {
                    for ov in ovs {
                        if let (Some(col), Some(ty)) = (&ov.column, &ov.type_name) {
                            overrides.push(ScytheTypeOverride {
                                column: col.clone(),
                                type_name: ty.clone(),
                            });
                        }
                    }
                }
            }

            // Fall back: older `gen:` block within v2
            if output.is_empty()
                && let Some(g) = &entry.gen_block
            {
                let targets: Vec<(&str, &Option<SqlcGenTarget>)> =
                    vec![("go", &g.go), ("kotlin", &g.kotlin), ("python", &g.python)];
                for (lang, target_opt) in targets {
                    if let Some(t) = target_opt
                        && let Some(out) = &t.out
                    {
                        if output.is_empty() {
                            output = out.clone();
                        }
                        gen_map.insert(
                            lang.to_string(),
                            ScytheGenTarget {
                                target: lang.to_string(),
                                derive: Vec::new(),
                            },
                        );
                    }
                }
            }

            let name = if sqlc.sql.len() == 1 {
                "main".to_string()
            } else {
                format!("sql_{idx}")
            };

            let gen_opt = if gen_map.is_empty() {
                None
            } else {
                Some(gen_map)
            };

            sql_blocks.push(ScytheSqlBlock {
                name,
                engine,
                schema,
                queries,
                output,
                gen_block: gen_opt,
                type_overrides: overrides,
            });
        }
    }

    let config = ScytheConfig {
        scythe: ScytheMeta {
            version: "1".to_string(),
        },
        sql: sql_blocks,
    };

    let toml_string =
        toml::to_string_pretty(&config).map_err(|e| internal(format!("toml serialize: {e}")))?;

    let dest = base_dir.join("scythe.toml");
    fs::write(&dest, &toml_string)
        .map_err(|e| internal(format!("write {}: {e}", dest.display())))?;

    Ok(dest.display().to_string())
}

// ---------------------------------------------------------------------------
// query file conversion
// ---------------------------------------------------------------------------

struct ConvertStats {
    files: usize,
    queries: usize,
    params_renamed: usize,
}

/// Convert all query files found under the given paths.
fn convert_query_files(
    query_paths: &[String],
    base_dir: &Path,
) -> Result<ConvertStats, ScytheError> {
    let mut stats = ConvertStats {
        files: 0,
        queries: 0,
        params_renamed: 0,
    };

    for qp in query_paths {
        let pattern = base_dir.join(qp);
        let pattern_str = pattern.display().to_string();

        // If it is a directory (no glob), add glob suffix
        let glob_pattern = if pattern_str.contains('*') {
            pattern_str.clone()
        } else if Path::new(&pattern_str).is_dir() {
            format!("{pattern_str}/*.sql")
        } else {
            // Might be a single file
            pattern_str.clone()
        };

        let entries =
            glob::glob(&glob_pattern).map_err(|e| internal(format!("glob {glob_pattern}: {e}")))?;

        for entry in entries {
            let path = entry.map_err(|e| internal(format!("glob entry: {e}")))?;
            if !path.is_file() {
                continue;
            }
            let (q, p) = convert_single_file(&path)?;
            stats.files += 1;
            stats.queries += q;
            stats.params_renamed += p;
        }
    }

    Ok(stats)
}

/// Convert a single .sql query file in-place (with .bak backup).
fn convert_single_file(path: &Path) -> Result<(usize, usize), ScytheError> {
    let content =
        fs::read_to_string(path).map_err(|e| internal(format!("read {}: {e}", path.display())))?;

    let (converted, query_count, param_count) = convert_query_content(&content)?;

    if converted != content {
        // Create backup
        let bak = path.with_extension("sql.bak");
        fs::write(&bak, &content)
            .map_err(|e| internal(format!("backup {}: {e}", bak.display())))?;
        // Write converted file
        fs::write(path, &converted)
            .map_err(|e| internal(format!("write {}: {e}", path.display())))?;
    }

    Ok((query_count, param_count))
}

/// Core conversion logic for the text content of a query file.
///
/// Returns (converted_text, query_count, param_rename_count).
fn convert_query_content(input: &str) -> Result<(String, usize, usize), ScytheError> {
    let annotation_re = Regex::new(
        r"(?m)^--\s*name:\s*(\w+)\s+:(one|many|exec|execrows|execresult|batchone|batchmany|batchexec|copyfrom)\s*$",
    )
    .map_err(|e| internal(format!("regex: {e}")))?;

    let sqlc_arg_re =
        Regex::new(r"sqlc\.arg\((\w+)\)").map_err(|e| internal(format!("regex: {e}")))?;

    let sqlc_narg_re =
        Regex::new(r"sqlc\.narg\((\w+)\)").map_err(|e| internal(format!("regex: {e}")))?;

    // Regex to find existing positional parameters like $1, $2, etc.
    let positional_re = Regex::new(r"\$(\d+)").map_err(|e| internal(format!("regex: {e}")))?;

    let mut output = String::with_capacity(input.len());
    let mut query_count: usize = 0;
    let mut param_rename_count: usize = 0;

    // Find all annotation positions
    let mut match_positions: Vec<(usize, usize, String, String)> = Vec::new();
    for caps in annotation_re.captures_iter(input) {
        let m = caps.get(0).unwrap();
        let name = caps[1].to_string();
        let return_type = caps[2].to_string();
        match_positions.push((m.start(), m.end(), name, return_type));
    }

    if match_positions.is_empty() {
        // No sqlc annotations found, return as-is.
        return Ok((input.to_string(), 0, 0));
    }

    // Text before first annotation
    if match_positions[0].0 > 0 {
        output.push_str(&input[..match_positions[0].0]);
    }

    for (i, (_, end, name, return_type)) in match_positions.iter().enumerate() {
        query_count += 1;

        // The SQL body runs from end of annotation to start of next annotation (or EOF)
        let body_end = if i + 1 < match_positions.len() {
            match_positions[i + 1].0
        } else {
            input.len()
        };
        let body = &input[*end..body_end];

        // Find highest existing positional parameter in the body
        let mut max_positional: usize = 0;
        for caps in positional_re.captures_iter(body) {
            if let Ok(n) = caps[1].parse::<usize>()
                && n > max_positional
            {
                max_positional = n;
            }
        }

        // Replace sqlc.arg(name) and sqlc.narg(name) with $N
        let mut next_param = max_positional + 1;
        let mut param_names: Vec<String> = Vec::new();
        let mut converted_body = body.to_string();

        // Process one match at a time (leftmost first) to assign sequential numbers.
        loop {
            let arg_match = sqlc_arg_re.find(&converted_body);
            let narg_match = sqlc_narg_re.find(&converted_body);

            let m = match (arg_match, narg_match) {
                (Some(a), Some(n)) => {
                    if a.start() <= n.start() {
                        a
                    } else {
                        n
                    }
                }
                (Some(a), None) => a,
                (None, Some(n)) => n,
                (None, None) => break,
            };

            // Determine which regex matched to extract the capture
            let matched_text = m.as_str();
            let pname = if let Some(caps) = sqlc_arg_re.captures(matched_text) {
                caps[1].to_string()
            } else if let Some(caps) = sqlc_narg_re.captures(matched_text) {
                caps[1].to_string()
            } else {
                break;
            };

            // Check if this param was already seen (reuse its number)
            let param_num = if let Some(pos) = param_names.iter().position(|n| n == &pname) {
                max_positional + 1 + pos
            } else {
                let num = next_param;
                param_names.push(pname);
                next_param += 1;
                num
            };

            param_rename_count += 1;

            let replacement = format!("${param_num}");
            converted_body = format!(
                "{}{}{}",
                &converted_body[..m.start()],
                replacement,
                &converted_body[m.end()..]
            );
        }

        // Build scythe annotations
        output.push_str(&format!("-- @name {name}\n"));
        output.push_str(&format!("-- @returns :{return_type}\n"));

        for pname in &param_names {
            output.push_str(&format!("-- @param {pname}\n"));
        }

        output.push_str(&converted_body);
    }

    Ok((output, query_count, param_rename_count))
}

// ---------------------------------------------------------------------------
// public entry point
// ---------------------------------------------------------------------------

pub fn run_migrate(sqlc_config_path: &Path) -> Result<(), ScytheError> {
    if !sqlc_config_path.exists() {
        return Err(internal(format!(
            "config file not found: {}",
            sqlc_config_path.display()
        )));
    }

    let raw = fs::read_to_string(sqlc_config_path)
        .map_err(|e| internal(format!("read {}: {e}", sqlc_config_path.display())))?;

    let sqlc: SqlcConfig = if sqlc_config_path
        .extension()
        .is_some_and(|ext| ext == "json")
    {
        serde_json::from_str(&raw).map_err(|e| internal(format!("parse json config: {e}")))?
    } else {
        serde_yaml::from_str(&raw).map_err(|e| internal(format!("parse yaml config: {e}")))?
    };

    let base_dir = sqlc_config_path.parent().unwrap_or_else(|| Path::new("."));

    // 1. Convert config file
    let config_dest = convert_config(&sqlc, base_dir)?;
    println!("Generated config: {config_dest}");

    // 2. Collect all query paths from sql entries
    let mut all_query_paths: Vec<String> = Vec::new();

    let version = sqlc.version.as_deref().unwrap_or("2");
    if version == "1" || (!sqlc.packages.is_empty() && sqlc.sql.is_empty()) {
        for pkg in &sqlc.packages {
            if let Some(q) = &pkg.queries {
                all_query_paths.push(ensure_glob_pattern(q));
            }
        }
    } else {
        for entry in &sqlc.sql {
            if let Some(qv) = &entry.queries {
                for p in qv.to_vec() {
                    all_query_paths.push(ensure_glob_pattern(&p));
                }
            }
        }
    }

    // 3. Convert query files
    let stats = convert_query_files(&all_query_paths, base_dir)?;

    println!(
        "Migration complete: {} file(s) converted, {} query/queries found, {} param(s) renamed",
        stats.files, stats.queries, stats.params_renamed
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_annotation_conversion() {
        let input = "-- name: GetProject :one\nSELECT id, name FROM projects WHERE id = $1;\n";
        let (out, qc, pc) = convert_query_content(input).unwrap();
        assert_eq!(qc, 1);
        assert_eq!(pc, 0);
        assert!(out.contains("-- @name GetProject"));
        assert!(out.contains("-- @returns :one"));
        assert!(out.contains("WHERE id = $1"));
    }

    #[test]
    fn test_sqlc_arg_conversion() {
        let input = "\
-- name: ListProjects :many
SELECT * FROM projects
ORDER BY created_at DESC
LIMIT sqlc.arg(page_limit)::int4 OFFSET sqlc.arg(page_offset)::int4;
";
        let (out, qc, pc) = convert_query_content(input).unwrap();
        assert_eq!(qc, 1);
        assert_eq!(pc, 2);
        assert!(out.contains("LIMIT $1::int4 OFFSET $2::int4"));
        assert!(out.contains("-- @param page_limit"));
        assert!(out.contains("-- @param page_offset"));
    }

    #[test]
    fn test_sqlc_arg_with_existing_positional() {
        let input = "\
-- name: GetFiltered :many
SELECT * FROM projects WHERE owner_id = $1
LIMIT sqlc.arg(page_limit)::int4;
";
        let (out, _qc, pc) = convert_query_content(input).unwrap();
        assert_eq!(pc, 1);
        assert!(out.contains("LIMIT $2::int4"), "got: {out}");
    }

    #[test]
    fn test_sqlc_narg_conversion() {
        let input = "\
-- name: Search :many
SELECT * FROM projects WHERE name = sqlc.narg(search_name);
";
        let (out, _qc, pc) = convert_query_content(input).unwrap();
        assert_eq!(pc, 1);
        assert!(out.contains("WHERE name = $1"));
        assert!(out.contains("-- @param search_name"));
    }

    #[test]
    fn test_multiple_queries() {
        let input = "\
-- name: GetOne :one
SELECT 1;
-- name: GetTwo :many
SELECT 2;
";
        let (out, qc, _pc) = convert_query_content(input).unwrap();
        assert_eq!(qc, 2);
        assert!(out.contains("-- @name GetOne"));
        assert!(out.contains("-- @name GetTwo"));
        assert!(out.contains("-- @returns :one"));
        assert!(out.contains("-- @returns :many"));
    }

    #[test]
    fn test_no_annotations_passthrough() {
        let input = "SELECT 1;\n";
        let (out, qc, pc) = convert_query_content(input).unwrap();
        assert_eq!(qc, 0);
        assert_eq!(pc, 0);
        assert_eq!(out, input);
    }

    #[test]
    fn test_repeated_arg_same_name() {
        let input = "\
-- name: Test :one
SELECT * FROM t WHERE a = sqlc.arg(x) AND b = sqlc.arg(x);
";
        let (out, _qc, pc) = convert_query_content(input).unwrap();
        assert_eq!(pc, 2);
        // Both should map to $1
        assert!(out.contains("a = $1 AND b = $1"), "got: {out}");
        // Only one @param line
        let param_count = out.matches("-- @param x").count();
        assert_eq!(param_count, 1, "expected one @param x, got: {out}");
    }

    #[test]
    fn test_exec_type() {
        let input = "-- name: DeleteProject :exec\nDELETE FROM projects WHERE id = $1;\n";
        let (out, qc, _) = convert_query_content(input).unwrap();
        assert_eq!(qc, 1);
        assert!(out.contains("-- @returns :exec"));
    }

    #[test]
    fn test_mixed_arg_and_narg() {
        let input = "\
-- name: Mixed :many
SELECT * FROM t
WHERE a = sqlc.arg(foo) AND b = sqlc.narg(bar) AND c = $1;
";
        let (out, _, pc) = convert_query_content(input).unwrap();
        assert_eq!(pc, 2);
        // $1 is existing, sqlc.arg(foo) -> $2, sqlc.narg(bar) -> $3
        assert!(out.contains("a = $2"), "got: {out}");
        assert!(out.contains("b = $3"), "got: {out}");
        assert!(out.contains("c = $1"), "got: {out}");
        assert!(out.contains("-- @param foo"));
        assert!(out.contains("-- @param bar"));
    }

    #[test]
    fn test_text_before_first_annotation() {
        let input = "\
-- Some header comment
-- another line

-- name: GetOne :one
SELECT 1;
";
        let (out, qc, _) = convert_query_content(input).unwrap();
        assert_eq!(qc, 1);
        assert!(out.starts_with("-- Some header comment"));
        assert!(out.contains("-- @name GetOne"));
    }
}
