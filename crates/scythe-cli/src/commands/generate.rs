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
    /// Optional path to a *partial* manifest merged over the backend's
    /// compiled-in manifest (see [`scythe_backend::manifest::ManifestOverlay`]).
    ///
    /// Declared explicitly rather than left to the `options` catch-all below:
    /// `#[serde(flatten)]` would otherwise swallow it and hand it to
    /// `apply_options`, where every backend that does not recognise the key
    /// ignores it — so a `manifest = "..."` line would silently do nothing.
    ///
    /// Keyed per target, not globally, because a backend name alone does not
    /// determine a manifest: `java-jdbc` covers nine engines and `rust-sqlx`
    /// five, and a single global mapping would hand a MySQL target the
    /// PostgreSQL type mappings (#82).
    #[serde(default)]
    manifest: Option<String>,
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
    /// Verbatim `manifest = "..."` value, still relative to the config file's
    /// directory. Resolution happens in `run_generate`, which is where
    /// `base_dir` lives. The legacy `[sql.gen.rust]` shape has no equivalent
    /// key, so it always resolves to `None`.
    manifest_override: Option<String>,
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
                    manifest_override: t.manifest.clone(),
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
                    manifest_override: None,
                    options,
                });
            }
            if let Some(ref py) = legacy.python {
                targets.push(ResolvedGenTarget {
                    backend: format!("python-{}", py.target),
                    output: default_output.clone(),
                    manifest_override: None,
                    options: std::collections::HashMap::new(),
                });
            }
            if let Some(ref ts) = legacy.typescript {
                targets.push(ResolvedGenTarget {
                    backend: format!("typescript-{}", ts.target),
                    output: default_output.clone(),
                    manifest_override: None,
                    options: std::collections::HashMap::new(),
                });
            }
            if let Some(ref go) = legacy.go {
                targets.push(ResolvedGenTarget {
                    backend: format!("go-{}", go.target),
                    output: default_output.clone(),
                    manifest_override: None,
                    options: std::collections::HashMap::new(),
                });
            }
            if targets.is_empty() {
                targets.push(ResolvedGenTarget {
                    backend: "rust-sqlx".to_string(),
                    output: default_output,
                    manifest_override: None,
                    options: std::collections::HashMap::new(),
                });
            }
            targets
        }
        None => {
            vec![ResolvedGenTarget {
                backend: "rust-sqlx".to_string(),
                output: default_output,
                manifest_override: None,
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

        // Computed once per `[[sql]]` block, not per target: every target
        // under one block shares the same schema, and `verify_provenance`
        // recomputes the identical value from the identical catalog when
        // checking these same artifacts later.
        let schema_fingerprint = catalog.fingerprint();

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

            // Merged before `apply_options`, not after: some backends derive
            // state from the manifest inside `apply_options` (python-psycopg3
            // computes its import set from the scalar mappings), so the
            // overlay has to already be in place or those derivations would
            // be based on the mappings the user just replaced.
            if let Some(ref manifest_path) = target.manifest_override {
                apply_manifest_override(backend.manifest_mut(), &target.backend, manifest_path, base_dir)?;
            }

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

            generate_for_backend(
                &sql_config.name,
                &*backend,
                &analyzed_queries,
                &output_dir,
                &overrides,
                &sql_config.engine,
                &schema_fingerprint,
            )?;
        }
    }

    eprintln!("Done.");
    Ok(())
}

/// Read the per-target manifest overlay at `manifest_path` and merge it into
/// `manifest` in place.
///
/// `manifest_path` comes from `manifest = "..."` on a `[[sql.gen]]` target and
/// is resolved against `base_dir` — the directory containing `scythe.toml` —
/// exactly like `output`, and for the same reason: as of 0.13.0 every path in
/// the config resolves relative to the config file, not the process's current
/// working directory, so `scythe generate --config /elsewhere/scythe.toml`
/// produces identical output from any directory. `Path::join` (not
/// `rebase_pattern`) is correct here because this is a literal path, not a
/// glob, and an already-absolute value passes through unchanged.
///
/// Every failure is fatal and names the backend and the *resolved absolute*
/// path. A missing file in particular is an error rather than a silent
/// fallback to the compiled-in manifest: falling back would reintroduce the
/// class of bug where the same config yields different code depending on
/// where it was run from.
fn apply_manifest_override(
    manifest: &mut scythe_backend::manifest::BackendManifest,
    backend_name: &str,
    manifest_path: &str,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let resolved = base_dir.join(manifest_path);
    // Reported rather than `resolved` so the message is unambiguous no matter
    // which directory the user ran from. `absolute` is purely lexical (it
    // neither resolves symlinks nor requires existence), so it still produces
    // a usable path for the not-found case below.
    let display_path = std::path::absolute(&resolved).unwrap_or_else(|_| resolved.clone());
    let display_path = display_path.display();

    let content = std::fs::read_to_string(&resolved).map_err(|e| {
        format!(
            "backend '{backend_name}': failed to read manifest override '{display_path}': {e}\n  \
             note: `manifest` in [[sql.gen]] resolves relative to the directory containing \
             scythe.toml,\n        not the current working directory"
        )
    })?;

    let overlay = scythe_backend::manifest::parse_overlay(&content).map_err(|e| {
        format!(
            "backend '{backend_name}': invalid manifest override '{display_path}': {e}\n  \
             note: the file is a *partial* manifest -- it may contain [types.scalars], \
             [types.containers],\n        [naming], and [imports.rules] only"
        )
    })?;

    manifest
        .apply_overlay(&overlay)
        .map_err(|e| format!("backend '{backend_name}': invalid manifest override '{display_path}': {e}"))?;

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

/// Line-comment token to embed the provenance header behind, derived from
/// `manifest().backend.language`.
///
/// `language` is a `String`, not a Rust enum, but exactly 10 distinct values
/// are used across every one of the 106 shipped manifests, so this one match
/// table is the single place backends' comment syntax is derived from —
/// no per-backend declaration, and no manifest schema change. Unrecognized
/// values fall back to `//`: every backend shipped today matches one of the
/// listed languages, and `//` is a safe default for any future one that
/// doesn't.
fn provenance_comment_prefix(language: &str) -> &'static str {
    match language {
        "python" | "ruby" | "elixir" => "#",
        _ => "//",
    }
}

/// Strip `\n` and `\r` from a provenance field value before it is embedded
/// in the header line.
///
/// Only [`provenance_header_line`]'s `engine` argument needs this: `version`
/// is a compile-time constant, `backend.name()` is a hardcoded per-backend
/// literal, and `schema` is always `sch1:` plus 16 lowercase hex characters
/// — none of those three can contain a line terminator. `engine` is
/// `sql_config.engine`, deserialized verbatim from the user's `scythe.toml`
/// with no validation upstream (`normalize_engine` only consults it to pick
/// a dialect; the raw string is what a caller passes through to here). A
/// value containing `\n` or a lone `\r` would terminate the comment early —
/// everything after it would land on its own physical line with no comment
/// prefix, becoming live, uncommented content in the generated file. That
/// breaks the exact guarantee this module is built on: the header always
/// reads as an ordinary comment, never as code. Sanitizing at the point of
/// embedding (rather than at config parse time) means the guarantee holds
/// regardless of how `engine` arrives — this call site today, or any future
/// one — not just for callers that happen to validate it first.
///
/// [`verify_provenance`] must sanitize `sql_config.engine` the same way
/// before comparing it against the parsed header's `engine` field: the
/// header always holds the sanitized value, so comparing it against a raw,
/// unsanitized value would permanently false-flag SC-PRV04 for any config
/// whose engine string needed sanitizing — the same class of bug as the
/// backend-alias mismatch fixed in SC-PRV03 above.
fn sanitize_provenance_field(value: &str) -> Cow<'_, str> {
    if value.contains(['\n', '\r']) {
        Cow::Owned(value.replace(['\n', '\r'], ""))
    } else {
        Cow::Borrowed(value)
    }
}

/// Build the provenance header line assembly prepends to every generated
/// file, right after [`CodegenBackend::file_preamble`] and before
/// [`CodegenBackend::file_header`]: the sentinel `scythe check` searches for
/// (see `PROVENANCE_SENTINEL` and `parse_provenance_header` below),
/// commented out using the target language's own line-comment syntax so it
/// reads as an ordinary comment to every downstream compiler, formatter, and
/// human.
///
/// `backend.name()` (not the raw `[[sql.gen]]` `backend = "..."` config
/// value) is what gets embedded — `get_backend` accepts several aliases per
/// backend (`"sqlx"`, `"rust"`, and `"rust-sqlx"` all construct the same
/// backend), and `name()` is the one canonical form every alias agrees on.
/// `verify_provenance`'s SC-PRV03 check compares against this same
/// `backend.name()`, not the config alias, for exactly this reason.
///
/// `engine` is sanitized via [`sanitize_provenance_field`] before embedding
/// — see that function's doc comment for why only `engine` needs it.
fn provenance_header_line(backend: &dyn CodegenBackend, engine: &str, schema: &str) -> String {
    let comment = provenance_comment_prefix(&backend.manifest().backend.language);
    let engine = sanitize_provenance_field(engine);
    format!(
        "{comment} scythe:provenance v={} backend={} engine={} schema={}\n",
        env!("CARGO_PKG_VERSION"),
        backend.name(),
        engine,
        schema
    )
}

/// Assemble a backend's complete generated file: preamble (text that must
/// be the literal first bytes, e.g. PHP's `<?php`), the provenance header
/// line, then the assembled body (file header, deduped enums, structs,
/// query functions, footer, post-footer).
///
/// Preamble and the provenance line are deliberately outside
/// [`assemble_body`] and prepended here instead of inside it: `assemble_body`
/// is exercised directly by tests that don't want a schema fingerprint or
/// scythe version threaded through every call, and keeping the ordering
/// invariant (preamble first, unconditionally, before anything else — even
/// the provenance comment) in exactly one place is what makes it possible to
/// state and test that invariant at all.
///
/// The blank line between the provenance line and the body is emitted only
/// when `file_preamble()` is non-empty. This is not cosmetic: for the 8
/// backends with a preamble override, the old (pre-provenance) `file_header()`
/// text already opened with its own blank line (PHP's `"<?php\n\ndeclare(...)"`,
/// Ruby's `"# frozen_string_literal: true\n\n# Auto-generated..."`), so the
/// unconditional separator reproduced those exact bytes once the provenance
/// line was accounted for. For the other 44 backends, `file_header()` never
/// had a leading blank line, so an unconditional separator silently inserted
/// one that was never there before — invisible in the assembled output, but
/// a real, provable byte-level regression (caught by generating real
/// integration_tests projects and diffing against the committed artifacts
/// with just the provenance line stripped back out; see
/// `assemble_output_no_blank_line_after_provenance_when_preamble_is_empty`
/// and `assemble_output_blank_line_after_provenance_when_preamble_is_non_empty`
/// below). Conditioning the separator on `preamble.is_empty()` reproduces the
/// old bytes in both cases:
/// - preamble non-empty (PHP): `"<?php\n"` + provenance + `"\n"` + `"declare..."`;
///   strip the provenance line → `"<?php\n\ndeclare..."`, the old header exactly.
/// - preamble empty (Go): provenance + `""` + `"// Code generated..."`;
///   strip the provenance line → `"// Code generated..."`, the old header exactly.
fn assemble_output(backend: &dyn CodegenBackend, results: &[QueryResult], engine: &str, schema: &str) -> String {
    let preamble = backend.file_preamble();
    let provenance = provenance_header_line(backend, engine, schema);
    let separator = if preamble.is_empty() { "" } else { "\n" };

    format!("{preamble}{provenance}{separator}{}", assemble_body(backend, results))
}

/// Assemble the full file body for a backend from its per-query results:
/// file header, deduped enum definitions, model/row structs and query
/// functions (ordered per `query_class_header`), file footer, and post
/// footer — joined into the final string, including the "no queries"
/// fallback. Pure and I/O-free so it can be unit tested directly and so
/// post-assembly steps (e.g. rustfmt) stay visibly separate in the caller.
///
/// Excludes [`CodegenBackend::file_preamble`] and the provenance header
/// line — both are [`assemble_output`]'s responsibility, not this
/// function's. See its doc comment for why.
fn assemble_body(backend: &dyn CodegenBackend, results: &[QueryResult]) -> String {
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
///
/// `engine` and `schema` are the raw `[[sql]]` engine alias (e.g.
/// `"mariadb"`, exactly as the user wrote it — not normalized) and the
/// current [`scythe_core::catalog::Catalog::fingerprint`], threaded through
/// to [`assemble_output`] for the provenance header line every generated
/// file now carries.
fn generate_for_backend(
    config_name: &str,
    backend: &dyn CodegenBackend,
    analyzed_queries: &[AnalyzedQuery],
    output_dir: &str,
    overrides: &[TypeOverride],
    engine: &str,
    schema: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut results: Vec<QueryResult> = Vec::new();
    for analyzed in analyzed_queries {
        let enums = analyzed.enums.clone();
        let code = generate_with_backend_and_overrides(analyzed, backend, overrides)?;
        results.push(QueryResult { code, enums });
    }

    let mut output_content = assemble_output(backend, &results, engine, schema);

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

        all_violations.extend(verify_provenance(sql_config, &catalog, base_dir)?);

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

/// Sentinel a generated file's provenance header line is built around, e.g.
/// `// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:0123456789abcdef`.
///
/// Deliberately comment-syntax-agnostic: the header sits behind whatever
/// comment token the target language uses (`//`, `#`, `--`, a block
/// comment, ...), but [`parse_provenance_header`] never looks at the
/// prefix. It finds this sentinel substring, then treats everything after
/// it on that line as whitespace-separated `key=value` pairs -- one
/// tokenizer instead of a per-language regex for stripping each comment
/// syntax.
const PROVENANCE_SENTINEL: &str = "scythe:provenance";

/// How many leading lines of a generated file are scanned for the
/// provenance sentinel. Assembly always emits the header at the very top of
/// the file, so a small fixed window keeps verification cheap on large
/// generated files and avoids ever matching the token if it coincidentally
/// appeared deep inside generated SQL string literals.
const PROVENANCE_SCAN_LINES: usize = 20;

/// Fields parsed from a generated file's provenance header line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProvenanceHeader {
    version: Option<String>,
    backend: Option<String>,
    engine: Option<String>,
    schema: Option<String>,
}

impl ProvenanceHeader {
    /// Names of fields the header is missing, in a stable order, for the
    /// SC-PRV06 "malformed header" message.
    fn missing_fields(&self) -> Vec<&'static str> {
        [
            (self.version.is_none(), "v"),
            (self.backend.is_none(), "backend"),
            (self.engine.is_none(), "engine"),
            (self.schema.is_none(), "schema"),
        ]
        .into_iter()
        .filter_map(|(missing, name)| missing.then_some(name))
        .collect()
    }

    fn is_complete(&self) -> bool {
        self.missing_fields().is_empty()
    }
}

/// Find and parse the provenance header line within `content`'s leading
/// [`PROVENANCE_SCAN_LINES`] lines.
///
/// Returns `None` if no line in the scanned window contains the sentinel at
/// all -- an artifact that predates provenance headers, or was never
/// scythe-managed to begin with. That is a distinct, less severe finding
/// (SC-PRV05, "no header") from a sentinel that is present but parses with
/// missing fields (SC-PRV06, "malformed header" -- `Some` with
/// [`ProvenanceHeader::is_complete`] false).
fn parse_provenance_header(content: &str) -> Option<ProvenanceHeader> {
    let line = content
        .lines()
        .take(PROVENANCE_SCAN_LINES)
        .find(|line| line.contains(PROVENANCE_SENTINEL))?;

    let sentinel_start = line.find(PROVENANCE_SENTINEL).expect("contains() just matched");
    let tail = line[sentinel_start + PROVENANCE_SENTINEL.len()..].trim();

    let mut header = ProvenanceHeader::default();
    for token in tail.split_ascii_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        match key {
            "v" => header.version = Some(value.to_string()),
            "backend" => header.backend = Some(value.to_string()),
            "engine" => header.engine = Some(value.to_string()),
            "schema" => header.schema = Some(value.to_string()),
            // Forward-compatible: the header format may grow fields later.
            // An older verifier ignoring keys it does not recognize (rather
            // than erroring) is what lets the header format and this
            // verifier evolve independently.
            _ => {}
        }
    }

    Some(header)
}

/// Compare each of `sql_config`'s resolved generation targets' on-disk
/// artifact against `catalog` and the current scythe build, reporting
/// provenance drift as `SC-PRV01`-`SC-PRV06` violations.
///
/// # Scope
///
/// This answers "was this file generated from the *current schema*?" --
/// nothing more. It does not answer "was this file generated from the
/// current *queries*?": editing a query file without touching the schema
/// and without regenerating produces no schema-fingerprint mismatch, because
/// [`scythe_core::catalog::Catalog::fingerprint`] covers only tables,
/// columns, enums, and composites -- never query SQL. Where a full
/// toolchain is available, `scythe generate` followed by `git status`
/// already answers the stronger question by actually regenerating and
/// diffing; provenance verification exists for the CI/review path where
/// running the full generator on every check is too expensive or the
/// toolchain (database drivers, per-language compilers) is unavailable.
///
/// # Missing artifacts
///
/// A target whose output file does not exist is skipped without a finding.
/// The `.gitignore` a scythe project ships by default excludes
/// `**/generated/`, so an absent artifact in a fresh checkout is the normal
/// case, not drift.
///
/// # Version mismatches are warnings, never errors
///
/// SC-PRV02 (embedded scythe version differs from the running version) is
/// hardcoded to [`Severity::Warn`] with no path to escalate it: erroring
/// here would fail every consumer's CI on the day scythe itself cuts a
/// release, before they have had a chance to regenerate.
///
/// # Errors
///
/// Returns an error only for a genuinely broken target configuration (an
/// unknown backend/engine combination, mirroring `run_generate`'s own
/// behavior for the same lookup) or an I/O failure other than the artifact
/// simply not existing -- both are config/environment problems, not
/// provenance findings.
fn verify_provenance(
    sql_config: &SqlConfig,
    catalog: &Catalog,
    base_dir: &Path,
) -> Result<Vec<scythe_lint::QueryViolation>, Box<dyn std::error::Error>> {
    use scythe_lint::{QueryViolation, Severity};

    let current_schema = catalog.fingerprint();
    let current_version = env!("CARGO_PKG_VERSION");

    let mut violations = Vec::new();

    for target in resolve_gen_targets(sql_config) {
        let backend = get_backend(&target.backend, &sql_config.engine).map_err(|e| {
            format!(
                "backend '{}' with engine '{}': {}",
                target.backend, sql_config.engine, e
            )
        })?;

        let artifact_path = base_dir.join(&target.output).join(output_filename(&*backend));

        let content = match std::fs::read_to_string(&artifact_path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("failed to read '{}': {}", artifact_path.display(), e).into()),
        };

        let target_label = format!("{}:{}", sql_config.name, target.backend);
        let path_display = artifact_path.display().to_string();

        let Some(header) = parse_provenance_header(&content) else {
            violations.push(QueryViolation {
                query_name: target_label,
                rule_id: Cow::Borrowed("SC-PRV05"),
                severity: Severity::Warn,
                message: format!(
                    "{path_display}: no provenance header found (predates provenance tracking, or is not scythe-managed)"
                ),
            });
            continue;
        };

        if !header.is_complete() {
            violations.push(QueryViolation {
                query_name: target_label,
                rule_id: Cow::Borrowed("SC-PRV06"),
                severity: Severity::Warn,
                message: format!(
                    "{path_display}: provenance header is missing field(s): {}",
                    header.missing_fields().join(", ")
                ),
            });
            continue;
        }

        // Safe: `is_complete()` above just confirmed every field is `Some`.
        let header_schema = header.schema.as_deref().unwrap();
        let header_version = header.version.as_deref().unwrap();
        let header_backend = header.backend.as_deref().unwrap();
        let header_engine = header.engine.as_deref().unwrap();

        if header_schema != current_schema {
            violations.push(QueryViolation {
                query_name: target_label.clone(),
                rule_id: Cow::Borrowed("SC-PRV01"),
                severity: Severity::Error,
                message: format!(
                    "{path_display}: schema drift -- generated against schema {header_schema}, \
                     current schema is {current_schema} (run `scythe generate` to refresh)"
                ),
            });
        }

        // Compared against `backend.name()` (the canonical form assembly
        // embeds), not `target.backend` (whatever alias the user wrote in
        // `scythe.toml`, e.g. `"sqlx"` or `"rb"`). `get_backend` accepts
        // several aliases per backend and every one of them constructs a
        // backend whose `name()` returns the same canonical string -- so
        // comparing against the raw config alias would flag every config
        // using an alias as permanent, unfixable drift.
        if header_backend != backend.name() {
            violations.push(QueryViolation {
                query_name: target_label.clone(),
                rule_id: Cow::Borrowed("SC-PRV03"),
                severity: Severity::Error,
                message: format!(
                    "{path_display}: generated by backend '{header_backend}', but this target now \
                     configures backend '{}' (run `scythe generate` to refresh)",
                    backend.name()
                ),
            });
        }

        // Compared against the sanitized form of `sql_config.engine`
        // (`sanitize_provenance_field`, the same function
        // `provenance_header_line` runs `engine` through before embedding
        // it), not the raw config value: the header can only ever hold the
        // sanitized string, so comparing it against a raw value would
        // permanently false-flag every config whose engine string needed
        // sanitizing -- the same class of bug as the backend-alias mismatch
        // fixed above for SC-PRV03.
        let sanitized_engine = sanitize_provenance_field(&sql_config.engine);
        if header_engine != sanitized_engine.as_ref() {
            violations.push(QueryViolation {
                query_name: target_label.clone(),
                rule_id: Cow::Borrowed("SC-PRV04"),
                severity: Severity::Error,
                message: format!(
                    "{path_display}: generated for engine '{header_engine}', but this target now \
                     configures engine '{sanitized_engine}' (run `scythe generate` to refresh)"
                ),
            });
        }

        if header_version != current_version {
            violations.push(QueryViolation {
                query_name: target_label,
                rule_id: Cow::Borrowed("SC-PRV02"),
                severity: Severity::Warn,
                message: format!(
                    "{path_display}: generated by scythe {header_version}, this is scythe {current_version} \
                     (consider running `scythe generate`)"
                ),
            });
        }
    }

    Ok(violations)
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
    fn assemble_body_interleaves_per_query_when_class_header_is_empty() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        assert!(
            backend.query_class_header().is_empty(),
            "test assumes rust-sqlx has no query class header"
        );

        let results = vec![
            query_result(Some("MODEL_A"), Some("ROW_A"), Some("FN_A")),
            query_result(None, Some("ROW_B"), Some("FN_B")),
        ];

        let output = assemble_body(backend.as_ref(), &results);

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
    fn assemble_body_groups_types_then_class_header_then_fns_when_non_empty() {
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

        let output = assemble_body(backend.as_ref(), &results);

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
    fn assemble_body_falls_back_to_placeholder_when_results_are_empty() {
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

            fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
                &mut self.manifest
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

        let output = assemble_body(&backend, &[]);

        assert_eq!(output, "// No queries generated.\n");
    }

    // -----------------------------------------------------------------------
    // Provenance header production (`provenance_comment_prefix`,
    // `provenance_header_line`, `assemble_output`)
    // -----------------------------------------------------------------------

    #[test]
    fn provenance_comment_prefix_covers_all_ten_manifest_languages() {
        // The exact 10 values used across every shipped manifest (see
        // `grep -h '^language' crates/scythe-codegen/manifests/*.toml | sort -u`).
        let hash_comment = ["python", "ruby", "elixir"];
        let slash_comment = ["rust", "typescript", "go", "java", "kotlin", "csharp", "php"];

        for language in hash_comment {
            assert_eq!(provenance_comment_prefix(language), "#", "language: {language}");
        }
        for language in slash_comment {
            assert_eq!(provenance_comment_prefix(language), "//", "language: {language}");
        }
    }

    #[test]
    fn provenance_comment_prefix_defaults_to_slash_slash_for_unknown_language() {
        assert_eq!(provenance_comment_prefix("cobol"), "//");
    }

    #[test]
    fn provenance_header_line_contains_sentinel_and_all_four_fields() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = provenance_header_line(backend.as_ref(), "postgresql", "sch1:0123456789abcdef");

        assert!(line.starts_with("// scythe:provenance "), "got: {line:?}");
        assert!(line.contains(&format!("v={}", env!("CARGO_PKG_VERSION"))));
        assert!(line.contains("backend=rust-sqlx"));
        assert!(line.contains("engine=postgresql"));
        assert!(line.contains("schema=sch1:0123456789abcdef"));
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn provenance_header_line_uses_hash_comment_for_ruby() {
        let backend = get_backend("ruby-pg", "postgresql").expect("ruby-pg should support postgresql");
        let line = provenance_header_line(backend.as_ref(), "postgresql", "sch1:aaaa");
        assert!(line.starts_with("# scythe:provenance "), "got: {line:?}");
    }

    #[test]
    fn sanitize_provenance_field_strips_newline_and_carriage_return() {
        assert_eq!(
            sanitize_provenance_field("postgresql\nfn evil() {}"),
            "postgresqlfn evil() {}"
        );
        assert_eq!(
            sanitize_provenance_field("postgresql\r\nfn evil() {}"),
            "postgresqlfn evil() {}"
        );
        assert_eq!(
            sanitize_provenance_field("postgresql\rfn evil() {}"),
            "postgresqlfn evil() {}"
        );
        assert_eq!(sanitize_provenance_field("clean"), "clean");
    }

    /// Documents why `verify_provenance`'s sanitized-vs-sanitized SC-PRV04
    /// comparison cannot be exercised end to end with a value that actually
    /// differs pre/post sanitization: `get_backend` rejects any `engine`
    /// whose `normalize_engine()` output is not an exact-string match for
    /// one of the backend's `supported_engines()`, and every recognized
    /// alias is a clean literal containing neither `\n` nor `\r`. Any
    /// engine value containing one fails `get_backend` before
    /// `verify_provenance` ever reaches the comparison -- so today the fix
    /// there closes a latent bug, not a currently reachable one. It is
    /// still correct: `provenance_header_line`/`assemble_output` (tested
    /// below) accept an arbitrary `&str` with no such gate, so the
    /// comparison must not depend on `get_backend` policing its input for
    /// it.
    #[test]
    fn sanitize_provenance_field_is_a_no_op_for_every_alias_get_backend_accepts() {
        for alias in [
            "postgresql",
            "postgres",
            "pg",
            "cockroachdb",
            "crdb",
            "mysql",
            "mariadb",
            "sqlite",
            "sqlite3",
            "duckdb",
            "mssql",
            "sqlserver",
            "tsql",
            "oracle",
            "snowflake",
            "redshift",
        ] {
            assert_eq!(sanitize_provenance_field(alias).as_ref(), alias);
        }
    }

    /// Regression test for the header-injection defect: an `engine` value
    /// containing a newline must not survive into the embedded header, or
    /// it would terminate the comment early and turn everything after it
    /// into live, uncommented content in the generated file. Asserts on the
    /// *line count* of the produced header, not a substring -- a substring
    /// check (e.g. `!line.contains("evil")`) would still pass on the
    /// broken, unsanitized version, since the injected text is still
    /// present, just on its own line.
    #[test]
    fn provenance_header_line_sanitizes_newline_in_engine() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let malicious_engine = "postgresql\nfn evil() {}";

        let line = provenance_header_line(backend.as_ref(), malicious_engine, "sch1:ffff");

        assert_eq!(
            line.lines().count(),
            1,
            "a sanitized header must be exactly one line, got: {line:?}"
        );
        assert_eq!(
            line,
            format!(
                "// scythe:provenance v={} backend=rust-sqlx engine=postgresqlfn evil() {{}} schema=sch1:ffff\n",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    /// End-to-end: a malicious `engine` value must not inject an
    /// uncommented line anywhere in the fully assembled output, not just in
    /// the header line built in isolation.
    #[test]
    fn assemble_output_sanitizes_newline_in_engine_end_to_end() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let malicious_engine = "postgresql\nfn evil() {}";

        let output = assemble_output(backend.as_ref(), &[], malicious_engine, "sch1:ffff");

        let provenance_lines: Vec<&str> = output.lines().filter(|l| l.contains("scythe:provenance")).collect();
        assert_eq!(
            provenance_lines.len(),
            1,
            "expected exactly one provenance line, got: {provenance_lines:?}"
        );
        assert!(
            provenance_lines[0].starts_with("// scythe:provenance "),
            "the provenance line itself must still be a comment: {:?}",
            provenance_lines[0]
        );
        assert!(
            !output.lines().any(|line| line.trim_start().starts_with("fn evil()")),
            "injected content must never appear as its own uncommented line:\n{output}"
        );
    }

    /// `parse_provenance_header` must read back exactly the sanitized value
    /// that was embedded -- the round trip `verify_provenance` depends on.
    #[test]
    fn assemble_output_round_trips_sanitized_engine_through_parse_provenance_header() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        // No embedded space, unlike the injection-defect example elsewhere
        // in this file: this test isolates round-trip fidelity of the `\n`
        // strip itself. A space-containing value is a distinct, separately
        // documented limitation -- see
        // `parse_provenance_header_truncates_engine_values_containing_spaces`.
        let malicious_engine = "postgresql\ndroptable";

        let output = assemble_output(backend.as_ref(), &[], malicious_engine, "sch1:ffff");
        let header = parse_provenance_header(&output).expect("assembled output must carry a parseable header");

        let expected = sanitize_provenance_field(malicious_engine);
        assert_eq!(header.engine.as_deref(), Some(expected.as_ref()));
    }

    /// **Known limitation, not fixed here, reported per the coordinator's
    /// own request to check round-trip fidelity.** `sanitize_provenance_field`
    /// only strips `\n`/`\r` (the characters that would break the "header
    /// is always a comment" guarantee); it does not touch spaces. But
    /// `parse_provenance_header`'s tail is tokenized with
    /// `split_ascii_whitespace()`, so an `engine` value that still contains
    /// an internal space -- e.g. the coordinator's own injection example,
    /// `"postgresql\nfn evil() {}"`, which sanitizes to
    /// `"postgresqlfn evil() {}"` -- does NOT round-trip: everything from
    /// the first space onward has no `=`, fails `split_once('=')`, and is
    /// silently dropped. `header.engine` ends up truncated to
    /// `"postgresqlfn"`, not the full sanitized value.
    ///
    /// This is not a comment-injection risk (the line stays one line,
    /// stays a comment) -- it is a *correctness* gap: a truncated
    /// `header.engine` could, in principle, spuriously match or fail to
    /// match `sql_config.engine` in `verify_provenance`'s SC-PRV04 check
    /// depending on what the truncated prefix happens to collide with.
    /// Pinned here as documented, current behavior rather than silently
    /// leaving it unasserted.
    #[test]
    fn parse_provenance_header_truncates_engine_values_containing_spaces() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let malicious_engine = "postgresql\nfn evil() {}";

        let output = assemble_output(backend.as_ref(), &[], malicious_engine, "sch1:ffff");
        let header = parse_provenance_header(&output).expect("assembled output must carry a parseable header");

        let fully_sanitized = sanitize_provenance_field(malicious_engine);
        assert_eq!(fully_sanitized.as_ref(), "postgresqlfn evil() {}");

        // What actually comes back is truncated at the first space -- not
        // the full sanitized value asserted above.
        assert_eq!(
            header.engine.as_deref(),
            Some("postgresqlfn"),
            "if this fails, either the truncation was fixed (update this test) \
             or the tokenizer changed in some other way -- either way, re-verify \
             SC-PRV04 behavior for engine values containing spaces"
        );
    }

    /// `assemble_output` must emit the *canonical* backend name
    /// (`backend.name()`), not whatever alias resolved to it -- otherwise a
    /// config using `backend = "sqlx"` would produce a header
    /// `verify_provenance` can never match against `backend.name()`
    /// (`"rust-sqlx"`), permanently false-flagging SC-PRV03.
    #[test]
    fn provenance_header_line_uses_canonical_name_not_config_alias() {
        let alias_backend = get_backend("sqlx", "postgresql").expect("the 'sqlx' alias should resolve");
        assert_eq!(
            alias_backend.name(),
            "rust-sqlx",
            "canonical name backends must agree on"
        );

        let line = provenance_header_line(alias_backend.as_ref(), "postgresql", "sch1:aaaa");
        assert!(line.contains("backend=rust-sqlx"));
        assert!(!line.contains("backend=sqlx"));
    }

    /// The full producer -> consumer round trip: assemble a file with
    /// `assemble_output`, then feed it straight back through
    /// `parse_provenance_header`. This is the test that would fail if the
    /// two sides silently drifted from each other (e.g. one side changing
    /// key names, or the sentinel token) even though each passes its own
    /// isolated tests.
    #[test]
    fn assemble_output_round_trips_through_parse_provenance_header() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let results = vec![query_result(Some("MODEL_A"), Some("ROW_A"), Some("FN_A"))];

        let output = assemble_output(backend.as_ref(), &results, "postgresql", "sch1:fedcba9876543210");

        let header = parse_provenance_header(&output).expect("assembled output must carry a parseable header");
        assert_eq!(header.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(header.backend.as_deref(), Some("rust-sqlx"));
        assert_eq!(header.engine.as_deref(), Some("postgresql"));
        assert_eq!(header.schema.as_deref(), Some("sch1:fedcba9876543210"));
    }

    /// `<?php` must be the literal first five bytes of the assembled file --
    /// not merely present somewhere in it. A provenance comment (or
    /// anything else) landing above it would silently degrade the file to
    /// HTML output in a PHP interpreter.
    #[test]
    fn assemble_output_keeps_php_open_tag_as_the_first_bytes() {
        for backend_name in ["php-pdo", "php-amphp"] {
            let backend = get_backend(backend_name, "postgresql").unwrap_or_else(|e| panic!("{backend_name}: {e}"));
            let output = assemble_output(backend.as_ref(), &[], "postgresql", "sch1:aaaa");
            assert!(
                output.starts_with("<?php\n"),
                "{backend_name}: expected output to start with '<?php\\n', got:\n{}",
                &output[..output.len().min(120)]
            );
            // The provenance line must still be present, just after the tag.
            assert!(output.contains("scythe:provenance"));
        }
    }

    /// `# frozen_string_literal: true` must be the literal first line of the
    /// assembled file -- Ruby only recognizes the magic comment on line 1
    /// (or line 2 after a shebang); anything preceding it makes Ruby
    /// silently ignore the pragma rather than error, which is exactly the
    /// failure `file_preamble` exists to prevent.
    #[test]
    fn assemble_output_keeps_frozen_string_literal_as_the_first_line() {
        for backend_name in [
            "ruby-pg",
            "ruby-mysql2",
            "ruby-sqlite3",
            "ruby-tiny-tds",
            "ruby-oci8",
            "ruby-trilogy",
        ] {
            let backend = get_backend(backend_name, "postgresql")
                .or_else(|_| get_backend(backend_name, "mysql"))
                .or_else(|_| get_backend(backend_name, "sqlite"))
                .or_else(|_| get_backend(backend_name, "mssql"))
                .or_else(|_| get_backend(backend_name, "oracle"))
                .unwrap_or_else(|e| panic!("{backend_name}: {e}"));
            let output = assemble_output(backend.as_ref(), &[], "postgresql", "sch1:bbbb");
            let first_line = output.lines().next().unwrap_or_default();
            assert_eq!(
                first_line,
                "# frozen_string_literal: true",
                "{backend_name}: expected the magic comment on line 1, got:\n{}",
                &output[..output.len().min(120)]
            );
            assert!(output.contains("scythe:provenance"));
        }
    }

    /// A backend with no preamble (the overwhelming majority) must start
    /// directly with the provenance line -- no leading blank line, no
    /// leading anything else.
    #[test]
    fn assemble_output_starts_with_provenance_line_when_backend_has_no_preamble() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        assert!(
            backend.file_preamble().is_empty(),
            "test assumes rust-sqlx has no preamble"
        );

        let output = assemble_output(backend.as_ref(), &[], "postgresql", "sch1:cccc");
        assert!(
            output.starts_with("// scythe:provenance "),
            "got:\n{}",
            &output[..output.len().min(120)]
        );
    }

    /// Pins the blank-line regression found by generating real
    /// `integration_tests` projects and diffing the result (with just the
    /// provenance line stripped back out) against the committed artifacts:
    /// for a backend with no `file_preamble()` override, the line
    /// immediately after the provenance line must be `file_header()`'s
    /// first line, never an empty line. An unconditional separator
    /// (`format!("{prelude}\n{body}")` regardless of whether `preamble` was
    /// empty) would insert a blank line here that the old, pre-provenance
    /// output never had -- invisible in the assembled output itself, but
    /// provable by stripping the provenance line and diffing against the
    /// old bytes.
    #[test]
    fn assemble_output_no_blank_line_after_provenance_when_preamble_is_empty() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        assert!(
            backend.file_preamble().is_empty(),
            "test assumes rust-sqlx has no preamble"
        );

        let header = backend.file_header();
        let expected_first_body_line = header
            .lines()
            .next()
            .expect("test assumes rust-sqlx's file_header is non-empty");

        let output = assemble_output(backend.as_ref(), &[], "postgresql", "sch1:dddd");
        let mut lines = output.lines();
        let provenance_line = lines.next().expect("output must have a first line");
        assert!(
            provenance_line.starts_with("// scythe:provenance "),
            "got: {provenance_line:?}"
        );

        let next_line = lines.next().expect("output must have a second line");
        assert_ne!(
            next_line, "",
            "no blank line may follow the provenance line when preamble is empty"
        );
        assert_eq!(
            next_line, expected_first_body_line,
            "the line after the provenance line must be file_header()'s first line, not a blank separator"
        );
    }

    /// The mirror of the test above: for a backend WITH a `file_preamble()`
    /// override (PHP here), the line after the provenance line must be
    /// empty -- the old `file_header()` text for these backends already
    /// opened with its own blank line (`"<?php\n\ndeclare(...)"`), so the
    /// separator must still be emitted for stripped output to reconstruct
    /// the old bytes. Paired with the test above so neither branch of
    /// `assemble_output`'s conditional separator can drift without one of
    /// the two failing.
    #[test]
    fn assemble_output_blank_line_after_provenance_when_preamble_is_non_empty() {
        let backend = get_backend("php-pdo", "postgresql").expect("php-pdo should support postgresql");
        assert!(
            !backend.file_preamble().is_empty(),
            "test assumes php-pdo has a non-empty preamble"
        );

        let output = assemble_output(backend.as_ref(), &[], "postgresql", "sch1:eeee");
        let mut lines = output.lines();
        let preamble_line = lines.next().expect("output must have a first line");
        assert_eq!(preamble_line, "<?php");

        let provenance_line = lines.next().expect("output must have a second line");
        assert!(
            provenance_line.starts_with("// scythe:provenance "),
            "got: {provenance_line:?}"
        );

        let blank_line = lines.next().expect("output must have a third line");
        assert_eq!(
            blank_line, "",
            "a blank line must follow the provenance line when preamble is non-empty"
        );
    }

    // -----------------------------------------------------------------------
    // Provenance header parsing (`parse_provenance_header`)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_provenance_header_reads_slash_slash_comment() {
        let content = "// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:0123456789abcdef\npub fn x() {}\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.version.as_deref(), Some("0.13.0"));
        assert_eq!(header.backend.as_deref(), Some("rust-sqlx"));
        assert_eq!(header.engine.as_deref(), Some("postgresql"));
        assert_eq!(header.schema.as_deref(), Some("sch1:0123456789abcdef"));
    }

    #[test]
    fn parse_provenance_header_reads_hash_comment() {
        let content =
            "# scythe:provenance v=0.13.0 backend=python-psycopg3 engine=postgresql schema=sch1:aaaa\nimport os\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.backend.as_deref(), Some("python-psycopg3"));
    }

    #[test]
    fn parse_provenance_header_reads_double_dash_comment() {
        let content = "-- scythe:provenance v=0.13.0 backend=elixir-postgrex engine=postgresql schema=sch1:bbbb\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.backend.as_deref(), Some("elixir-postgrex"));
    }

    /// A block-comment closer with a space before it is just another
    /// unrecognized (no `=`) token and is ignored -- proving the tokenizer
    /// truly never looks at comment syntax, opening or closing.
    #[test]
    fn parse_provenance_header_ignores_block_comment_closer() {
        let content = "/* scythe:provenance v=0.13.0 backend=csharp-npgsql engine=postgresql schema=sch1:cccc */\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.schema.as_deref(), Some("sch1:cccc"));
        assert_eq!(header.backend.as_deref(), Some("csharp-npgsql"));
    }

    #[test]
    fn parse_provenance_header_returns_none_when_sentinel_absent() {
        let content = "// Auto-generated by scythe. Do not edit.\npub fn x() {}\n";
        assert!(parse_provenance_header(content).is_none());
    }

    #[test]
    fn parse_provenance_header_only_scans_leading_window() {
        let mut content = String::new();
        for _ in 0..(PROVENANCE_SCAN_LINES + 5) {
            content.push_str("// filler line, not the header\n");
        }
        content.push_str("// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:dddd\n");

        assert!(
            parse_provenance_header(&content).is_none(),
            "a sentinel outside the scan window must not be picked up"
        );
    }

    #[test]
    fn parse_provenance_header_ignores_unknown_keys_forward_compatibly() {
        let content =
            "// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:eeee future_field=xyz\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert!(header.is_complete());
    }

    #[test]
    fn provenance_header_missing_fields_lists_them() {
        let content = "// scythe:provenance v=0.13.0 backend=rust-sqlx\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert!(!header.is_complete());
        assert_eq!(header.missing_fields(), vec!["engine", "schema"]);
    }

    // -----------------------------------------------------------------------
    // End-to-end provenance verification (`verify_provenance`)
    // -----------------------------------------------------------------------

    fn provenance_test_catalog() -> Catalog {
        Catalog::from_ddl(&["CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);"]).unwrap()
    }

    /// A `SqlConfig` whose single implicit generation target (no
    /// `[[sql.gen]]` block, matching `resolve_gen_targets`'s documented
    /// fallback) is `rust-sqlx`, writing to `output_dir`.
    fn provenance_test_sql_config(output_dir: &std::path::Path) -> SqlConfig {
        SqlConfig {
            name: "main".to_string(),
            engine: "postgresql".to_string(),
            schema: Vec::new(),
            queries: Vec::new(),
            output: Some(output_dir.to_string_lossy().into_owned()),
            gen_config: None,
            type_overrides: None,
        }
    }

    fn write_artifact(dir: &std::path::Path, header_tail: &str) {
        std::fs::write(
            dir.join("queries.rs"),
            format!("// scythe:provenance {header_tail}\npub fn generated() {{}}\n"),
        )
        .unwrap();
    }

    #[test]
    fn verify_provenance_skips_missing_artifact_silently() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let sql_config = provenance_test_sql_config(dir.path());

        // No file written at all -- `.gitignore`'d `generated/` output is
        // the normal state of a fresh checkout, not drift.
        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert!(violations.is_empty());
    }

    #[test]
    fn verify_provenance_matching_header_produces_no_violations() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={}",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert!(violations.is_empty(), "expected no violations, got {violations:?}");
    }

    #[test]
    fn verify_provenance_detects_schema_drift_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let sql_config = provenance_test_sql_config(dir.path());

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema=sch1:0000000000000000",
                env!("CARGO_PKG_VERSION")
            ),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV01");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Error);
    }

    #[test]
    fn verify_provenance_detects_version_mismatch_as_warn_never_error() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        write_artifact(
            dir.path(),
            &format!("v=0.0.1-does-not-exist backend=rust-sqlx engine=postgresql schema={schema}"),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV02");
        assert_eq!(
            violations[0].severity,
            scythe_lint::Severity::Warn,
            "a scythe version mismatch must never be reported as Error"
        );
    }

    #[test]
    fn verify_provenance_detects_backend_mismatch_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-tokio-postgres engine=postgresql schema={}",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV03");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Error);
    }

    /// A config target written with a backend *alias* (`"sqlx"` instead of
    /// the canonical `"rust-sqlx"`) must not be reported as SC-PRV03 drift
    /// against a header that (correctly) embeds the canonical name --
    /// `get_backend("sqlx", ...)` and `get_backend("rust-sqlx", ...)`
    /// construct the same backend, and `assemble_output` always writes the
    /// canonical `backend.name()`, never the alias the user happened to
    /// type in `scythe.toml`.
    #[test]
    fn verify_provenance_does_not_false_flag_backend_alias() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();

        let sql_config = SqlConfig {
            name: "main".to_string(),
            engine: "postgresql".to_string(),
            schema: Vec::new(),
            queries: Vec::new(),
            output: None,
            gen_config: Some(GenTargets::Array(vec![GenTarget {
                backend: "sqlx".to_string(),
                output: dir.path().to_string_lossy().into_owned(),
                manifest: None,
                options: std::collections::HashMap::new(),
            }])),
            type_overrides: None,
        };

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={}",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert!(
            violations.is_empty(),
            "backend alias must not cause false-positive drift, got {violations:?}"
        );
    }

    #[test]
    fn verify_provenance_detects_engine_mismatch_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=mysql schema={}",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV04");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Error);
    }

    #[test]
    fn verify_provenance_no_header_found_is_warn_sc_prv05() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let sql_config = provenance_test_sql_config(dir.path());

        std::fs::write(
            dir.path().join("queries.rs"),
            "// Auto-generated by scythe. Do not edit.\n",
        )
        .unwrap();

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV05");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Warn);
    }

    #[test]
    fn verify_provenance_malformed_header_is_warn_sc_prv06() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let sql_config = provenance_test_sql_config(dir.path());

        // Missing `engine` and `schema`.
        write_artifact(
            dir.path(),
            &format!("v={} backend=rust-sqlx", env!("CARGO_PKG_VERSION")),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV06");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Warn);
    }

    #[test]
    fn verify_provenance_reports_multiple_mismatches_independently() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let sql_config = provenance_test_sql_config(dir.path());

        // Schema, backend, and engine all wrong at once -- version matches
        // so SC-PRV02 must not fire.
        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-tokio-postgres engine=mysql schema=sch1:1111111111111111",
                env!("CARGO_PKG_VERSION")
            ),
        );

        let violations = verify_provenance(&sql_config, &catalog, std::path::Path::new(".")).unwrap();
        let mut ids: Vec<&str> = violations.iter().map(|v| v.rule_id.as_ref()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["SC-PRV01", "SC-PRV03", "SC-PRV04"]);
    }
}
