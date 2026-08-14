use std::borrow::Cow;
use std::path::Path;

use serde::Deserialize;

use ahash::{AHashMap, AHashSet};

use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case};
use scythe_codegen::validation::{ValidationOutcome, validate_generated_code};
use scythe_codegen::{
    CodegenBackend, RbsEnumInfo, RbsGenerationContext, RbsQueryInfo, TypeOverride, degrade_unsupported_nested_structs,
    generate_single_enum_def_with_backend, generate_with_backend_and_overrides, get_backend,
};
use scythe_core::analyzer::{AnalyzedQuery, EnumInfo, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::{QueryCommand, parse_query_with_dialect};
use scythe_lint::{QueryViolation, RuleRegistry, Severity};

use super::shared::{
    config_dir, has_unannotated_sql, redact_url_password, resolve_contained_output, resolve_globs, split_query_file,
};

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
#[serde(try_from = "RawSqlConfig")]
struct SqlConfig {
    name: String,
    engine: String,
    schema: Vec<String>,
    queries: Vec<String>,
    /// Legacy: output directory, used as the default when no `[[sql.gen]]`
    /// array targets are configured (i.e. `gen` is absent, or is the legacy
    /// `[sql.gen.<lang>]` table form). Ignored for `[[sql.gen]]` array
    /// entries -- each of those needs its own `output` key. See
    /// `website/src/content/docs/guide/configuration.md`, where this is
    /// documented as legacy.
    output: Option<String>,
    /// Generation targets via [[sql.gen]] or [sql.gen.rust]
    gen_config: Option<GenTargets>,
    type_overrides: Option<Vec<TypeOverrideConfig>>,
}

/// Deserialization target for a raw `[[sql]]` table, before [`GenTargets`]
/// validation. `gen` is captured as an untyped [`toml::Value`] rather than
/// deserialized directly into [`GenTargets`] so [`SqlConfig`]'s `TryFrom`
/// impl can hand [`parse_gen_targets`] this block's `output` alongside it --
/// needed to name the #116 mistake (`[[sql]].output` set alongside an
/// array-form `[[sql.gen]]`, where it is silently ignored) directly instead
/// of reporting only a generic missing-field error.
///
/// `#[serde(deny_unknown_fields)]` is safe here specifically because this
/// struct's field list is already the complete, documented `[[sql]]` schema
/// (see the Fields table for `[[sql]]` in `configuration.md`): `name`,
/// `engine`, `schema`, `queries`, `output`, `gen`, `type_overrides`. It is
/// deliberately *not* applied to `ScytheConfig` (the enclosing top-level
/// struct) or to the separate, narrower `SqlConfig` copies in `audit.rs`,
/// `lint_cmd.rs`, and `fmt.rs`: those are intentionally partial projections
/// of the same `scythe.toml` (missing e.g. `output`/`gen`/`type_overrides`,
/// or -- at the top level -- `[inspect]`/`[audit]`), so denying unknown
/// fields there would reject configs that are valid for other commands
/// reading the same file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSqlConfig {
    name: String,
    engine: String,
    schema: Vec<String>,
    queries: Vec<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default, rename = "gen")]
    gen_config: Option<toml::Value>,
    #[serde(default)]
    type_overrides: Option<Vec<TypeOverrideConfig>>,
}

impl TryFrom<RawSqlConfig> for SqlConfig {
    type Error = String;

    fn try_from(raw: RawSqlConfig) -> Result<Self, Self::Error> {
        let gen_config = raw
            .gen_config
            .map(|value| parse_gen_targets(value, raw.output.as_deref(), &raw.name))
            .transpose()?;

        Ok(SqlConfig {
            name: raw.name,
            engine: raw.engine,
            schema: raw.schema,
            queries: raw.queries,
            output: raw.output,
            gen_config,
            type_overrides: raw.type_overrides,
        })
    }
}

/// Supports both legacy `[sql.gen.rust]` and new `[[sql.gen]]` array formats.
///
/// Built by [`parse_gen_targets`] rather than derived `Deserialize`. TOML's
/// `[[sql.gen]]` (array) vs `[sql.gen.<lang>]` (table) syntax already
/// disambiguates the two shapes unambiguously, so dispatching on that shape
/// directly preserves each variant's real deserialization error instead of
/// discarding it. The `#[serde(untagged)]` enum this replaces buffered both
/// variants' content and, on failure, reported only "data did not match any
/// variant of untagged enum GenTargets" -- naming an internal type and
/// pointing at the `[[sql.gen]]` section header rather than, e.g., the
/// missing `output` key that actually caused the failure (#116).
#[derive(Debug)]
enum GenTargets {
    /// New format: `[[sql.gen]]` array of targets
    Array(Vec<GenTarget>),
    /// Legacy format: `[sql.gen.rust]` with a nested language key
    Legacy(LegacyGenConfig),
}

/// Parse the `gen` key of a `[[sql]]` block into [`GenTargets`].
///
/// `top_level_output` is this block's `[[sql]].output`, threaded through so
/// a missing `output` on an array entry can name the specific mistake behind
/// it (see [`RawSqlConfig`] and [`describe_gen_target_error`]) instead of a
/// generic "missing field" error.
///
/// `block_name` is this block's `[[sql]].name`, and it carries the only
/// reliable location information in the message. Errors raised here reach
/// serde as `Error::custom`, which carries no span; the first frame that
/// backfills one is the top-level `sql` key, so the rendered `TOML parse error
/// at line N` points at the *first* `[[sql]]` block whichever block actually
/// failed. Naming the block in the text is what disambiguates a multi-block
/// config -- see `error_in_second_sql_block_names_that_block`.
fn parse_gen_targets(
    value: toml::Value,
    top_level_output: Option<&str>,
    block_name: &str,
) -> Result<GenTargets, String> {
    match value {
        toml::Value::Array(items) => {
            let mut targets = Vec::with_capacity(items.len());
            for (idx, item) in items.into_iter().enumerate() {
                let target: GenTarget = item
                    .clone()
                    .try_into()
                    .map_err(|e| describe_gen_target_error(idx, &item, &e, top_level_output, block_name))?;
                targets.push(target);
            }
            Ok(GenTargets::Array(targets))
        }
        toml::Value::Table(_) => value
            .try_into::<LegacyGenConfig>()
            .map(GenTargets::Legacy)
            .map_err(|e| format!("[[sql]] block \"{block_name}\": invalid `[sql.gen.<lang>]` table: {e}")),
        other => Err(format!(
            "`gen` must be an array of `[[sql.gen]]` tables (new format, e.g. `[[sql.gen]]`) or a \
             `[sql.gen.<lang>]` table (legacy format, e.g. `[sql.gen.rust]`), found {}",
            other.type_str()
        )),
    }
}

/// Turn a single `[[sql.gen]]` entry's deserialization failure into an
/// actionable message: which entry (by position and, when present, its
/// `backend`), and -- for the #116 mistake specifically -- a pointer at the
/// `[[sql]].output` the user likely meant to apply to every target.
fn describe_gen_target_error(
    idx: usize,
    item: &toml::Value,
    err: &toml::de::Error,
    top_level_output: Option<&str>,
    block_name: &str,
) -> String {
    let position = match item.get("backend").and_then(toml::Value::as_str) {
        Some(backend) => format!(
            "[[sql]] block \"{}\", [[sql.gen]] entry #{} (backend = \"{}\")",
            block_name,
            idx + 1,
            backend
        ),
        None => format!("[[sql]] block \"{}\", [[sql.gen]] entry #{}", block_name, idx + 1),
    };

    if err.to_string().contains("missing field `output`") {
        return match top_level_output {
            Some(top) => format!(
                "{position} is missing `output`. This block sets `[[sql]].output = \"{top}\"`, but that \
                 field is only used as a default for the legacy `[sql.gen.<lang>]` table form -- it is \
                 ignored for `[[sql.gen]]` array entries. Add `output = \"...\"` to this entry directly."
            ),
            None => format!("{position} is missing required field `output` (e.g. `output = \"src/generated\"`)."),
        };
    }

    format!("{position}: {err}")
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
    /// `apply_options`, where no backend declares `manifest` as a known key --
    /// so a `manifest = "..."` line would be rejected outright by every
    /// backend (#103) instead of being applied as the overlay it names.
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
    kotlin: Option<LegacyLangGenConfig>,
    /// Any other `[sql.gen.<lang>]` table -- a language this legacy format
    /// has no mapping for. Captured via flatten (rather than left to serde's
    /// default unknown-field handling, which would silently drop it) so
    /// `resolve_gen_targets` can name it in a hard error instead of quietly
    /// generating nothing for that language. See issue #97.
    #[serde(flatten)]
    unsupported: std::collections::HashMap<String, toml::Value>,
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
///
/// Returns `Err` (naming the offending config value) instead of silently
/// falling back to a default backend when a legacy `target` cannot be
/// resolved to a real backend -- see issue #97, where an unrecognized
/// `[sql.gen.rust] target` silently produced `rust-sqlx` and an unrecognized
/// `[sql.gen.<lang>]` table (e.g. `kotlin`, which this legacy format had no
/// field for at all) was silently dropped, generating the wrong language
/// with no error.
fn resolve_gen_targets(sql_config: &SqlConfig) -> Result<Vec<ResolvedGenTarget>, String> {
    let default_output = sql_config.output.clone().unwrap_or_else(|| "generated".to_string());

    match &sql_config.gen_config {
        Some(GenTargets::Array(targets)) => Ok(targets
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
            .collect()),
        Some(GenTargets::Legacy(legacy)) => {
            if !legacy.unsupported.is_empty() {
                let mut names: Vec<&str> = legacy.unsupported.keys().map(String::as_str).collect();
                names.sort_unstable();
                return Err(format!(
                    "[sql.gen] has no backend for language(s): {} (supported: rust, python, typescript, go, \
                     kotlin -- use the `[[sql.gen]]` array form with an explicit `backend` for anything else)",
                    names.join(", ")
                ));
            }

            let mut targets = Vec::new();
            if let Some(ref rust) = legacy.rust {
                let backend = match rust.target.as_str() {
                    "sqlx" => "rust-sqlx",
                    "tokio-postgres" => "rust-tokio-postgres",
                    "tiberius" => "rust-tiberius",
                    "sibyl" => "rust-sibyl",
                    other => {
                        return Err(format!(
                            "[sql.gen.rust] target '{other}' is not a supported rust backend (expected one \
                             of: sqlx, tokio-postgres, tiberius, sibyl)"
                        ));
                    }
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
            if let Some(ref kotlin) = legacy.kotlin {
                targets.push(ResolvedGenTarget {
                    backend: format!("kotlin-{}", kotlin.target),
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
            Ok(targets)
        }
        None => Ok(vec![ResolvedGenTarget {
            backend: "rust-sqlx".to_string(),
            output: default_output,
            manifest_override: None,
            options: std::collections::HashMap::new(),
        }]),
    }
}

/// Reject a `column = "table.col"` override whose target names no column present in any of
/// `analyzed_queries` -- almost always a typo'd table or column name.
///
/// Without this, such an entry is indistinguishable from a correct one that simply never
/// happens to fire: [`scythe_codegen::resolve::resolve_columns`] silently falls back to the
/// column's inferred type either way (#189). Covers every query's flat `columns` plus, for a
/// `:grouped` query, both halves of `group_by`'s parent/child split -- the same column sets
/// [`generate_with_backend_and_overrides`] and [`generate_rbs_if_supported`] resolve.
fn check_type_overrides_resolve(
    config_name: &str,
    analyzed_queries: &[AnalyzedQuery],
    overrides: &[TypeOverride],
) -> Result<(), Box<dyn std::error::Error>> {
    let known = scythe_codegen::overrides::column_references(analyzed_queries.iter().flat_map(|query| {
        let group_columns = query
            .group_by
            .iter()
            .flat_map(|group| group.parent_columns.iter().chain(group.child_columns.iter()));
        query.columns.iter().chain(group_columns)
    }));
    let unmatched = scythe_codegen::overrides::unmatched_column_overrides(overrides, &known);
    if unmatched.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = unmatched.iter().filter_map(|o| o.column.as_deref()).collect();
    Err(format!(
        "[{config_name}] type_overrides column reference(s) match no column in any resolved query -- check for \
         a typo in the table or column name: {}",
        names.join(", ")
    )
    .into())
}

pub fn run_generate(
    config_path: &str,
    allow_output_escape: bool,
    validate_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut written: Vec<String> = Vec::new();
    let mut validation_failed = false;

    let result = run_generate_inner(
        config_path,
        allow_output_escape,
        validate_output,
        &mut written,
        &mut validation_failed,
    );

    if result.is_err() && !written.is_empty() {
        // Generation is not transactional -- a failure on a later `[[sql]]`
        // block (or a later target within one) leaves every output already
        // written by this run on disk, some of it now stale relative to the
        // config that failed to finish. Silently leaving that out of the
        // error would make a partial, inconsistent output tree look no
        // different from a clean failure with nothing written. See #212.
        eprintln!(
            "warning: generation failed partway through -- {} output file(s) were already written \
             this run and may be stale relative to the rest of the config:",
            written.len()
        );
        for path in &written {
            eprintln!("  {path}");
        }
    }

    if result.is_ok() {
        eprintln!("Done.");
    }

    // Exit 2, not a plain `Err` (which `main` turns into exit 1) -- matching
    // the #212 contract `check`/`lint`/`fmt --check` already follow: exit 1
    // stays reserved for an operational failure (bad config, unreadable
    // file), exit 2 means "a finding is present". A `--validate-output`
    // failure is a finding about the generated code itself, not an
    // operational failure, so it gets `check`'s exit code, not `main`'s
    // generic one. Checked only when generation itself succeeded: an
    // operational failure already exits 1 below, and whatever validation
    // this run got through before that failure is moot.
    if result.is_ok() && validate_output && validation_failed {
        eprintln!("generate: one or more backends failed generated-code validation (see above)");
        std::process::exit(2);
    }

    result
}

fn run_generate_inner(
    config_path: &str,
    allow_output_escape: bool,
    validate_output: bool,
    written: &mut Vec<String>,
    validation_failed: &mut bool,
) -> Result<(), Box<dyn std::error::Error>> {
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
            if has_unannotated_sql(&content) {
                return Err(format!(
                    "[{}] query file '{}' has SQL content with no `-- name:` / `-- @name` annotation above it; \
                     that content is silently dropped, not generated. Add an annotation above every statement \
                     (e.g. `-- name: MyQuery :one`), or move non-query SQL out of the `queries` glob",
                    sql_config.name, query_file
                )
                .into());
            }
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

        // Computed once per `[[sql]]` block, not per target, exactly like
        // `schema_fingerprint` above and for the same reason: every target
        // under one block shares the same analyzed query set, and
        // `verify_provenance` recomputes the identical value from the same
        // inputs when checking these same artifacts later (#94).
        let queries_fingerprint = AnalyzedQuery::fingerprint_set(&analyzed_queries);

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

        // A `column = "table.col"` override that names no relation/column pair present in
        // any query this block resolves is a config mistake -- almost always a typo'd table
        // or column name -- that would otherwise stay completely silent: it simply never
        // gets a chance to fire (#189). Checked once per block, before any target is
        // generated -- a hard error rather than a warning, the same severity this file
        // already gives a config mistake that can never be correct (e.g. two targets
        // colliding on one output path, an unannotated SQL block): a warning would compete
        // with the other `eprintln!` progress lines and is easy to miss in CI.
        check_type_overrides_resolve(&sql_config.name, &analyzed_queries, &overrides)?;

        let gen_targets = resolve_gen_targets(sql_config)?;

        // Resolve every target's backend and output directory before writing
        // any of them. This does two things a single combined loop cannot:
        // (1) it lets a containment violation or a filename collision on the
        // *last* target abort before the *first* target's files are written,
        // and (2) it makes the collision check below possible at all, since
        // detecting "two targets write the same file" requires knowing every
        // target's resolved (output_dir, filename) up front.
        let mut resolved: Vec<(&ResolvedGenTarget, Box<dyn CodegenBackend>, String)> = Vec::new();
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

            // ~keep `output` is a path, not a glob pattern, so it is resolved via
            // plain `Path::join` (not `rebase_pattern`/`glob::Pattern::escape`)
            // — an output directory literally named `a[1]` must not be
            // mangled. `PathBuf::push` (which `join` uses internally) leaves
            // an already-absolute `target.output` unchanged: pushing an
            // absolute path replaces the buffer instead of appending to it.
            //
            // Rejected by default when the resolved directory escapes
            // `base_dir` (`../` traversal, or an absolute path) -- `scythe
            // generate` running in CI against a PR-modified `scythe.toml` is
            // otherwise an arbitrary directory-create-and-write primitive.
            // See #207.
            let output_dir = resolve_contained_output(base_dir, &target.output, &target.backend, allow_output_escape)?
                .to_string_lossy()
                .into_owned();

            resolved.push((target, backend, output_dir));
        }

        // Two targets whose (output directory, filename) coincide would
        // otherwise silently clobber each other -- the second one to write
        // wins and the first target's generated code simply vanishes with
        // no error. Detected up front (before either is written) rather
        // than left for `scythe check`'s SC-PRV03 to notice after the fact.
        // See #207.
        let mut seen_paths: AHashMap<String, &str> = AHashMap::new();
        for (target, backend, output_dir) in &resolved {
            let filename = output_filename(backend.as_ref());
            let full_path = Path::new(output_dir).join(&filename).to_string_lossy().into_owned();
            if let Some(existing_backend) = seen_paths.insert(full_path.clone(), target.backend.as_str()) {
                return Err(format!(
                    "[{}] targets '{}' and '{}' both write '{}'; give one of them a distinct `output` \
                     directory so neither silently overwrites the other's generated code",
                    sql_config.name, existing_backend, target.backend, full_path
                )
                .into());
            }
        }

        for (target, backend, output_dir) in &resolved {
            let options_fingerprint = resolve_options_fingerprint(target, base_dir)?;
            let target_validation_failed = generate_for_backend(
                &sql_config.name,
                backend.as_ref(),
                &analyzed_queries,
                output_dir,
                &overrides,
                ProvenanceFields {
                    engine: &sql_config.engine,
                    schema: &schema_fingerprint,
                    queries: &queries_fingerprint,
                    options: &options_fingerprint,
                },
                validate_output,
            )?;
            if target_validation_failed {
                *validation_failed = true;
            }
            written.push(
                Path::new(output_dir)
                    .join(output_filename(backend.as_ref()))
                    .display()
                    .to_string(),
            );
        }
    }

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
             [types.containers],\n        [types.docblock_containers], [naming], and \
             [imports.rules] only"
        )
    })?;

    manifest
        .apply_overlay(&overlay)
        .map_err(|e| format!("backend '{backend_name}': invalid manifest override '{display_path}': {e}"))?;

    Ok(())
}

/// Compute the `opt1:<16 hex>` fingerprint [`provenance_header_line`] embeds
/// as `target`'s `options=` field: a deterministic fingerprint over its
/// resolved `[[sql.gen]]` options and, when one is configured, its manifest
/// overlay's file *contents* (GH #155). Before this field existed,
/// changing an option (a `derive` list, `serde = true`, `row_type`, a
/// naming-case override) or swapping in a different `manifest = "..."`
/// overlay left the header byte-identical, so `scythe check` reported the
/// stale artifact as fresh.
///
/// Deliberately independent of [`apply_manifest_override`] mutating a real
/// backend, even though both read the same file: this is called both when
/// *writing* the header (`run_generate_inner`, alongside a full
/// `apply_manifest_override` + `apply_options` on a constructed backend) and
/// when *verifying* it (`verify_provenance`, which never constructs a
/// fully-configured backend at all -- it only needs `backend.name()` for the
/// finding's label). Reading the overlay file fresh here, rather than
/// threading `apply_manifest_override`'s already-read content through to the
/// verifier, is what keeps the write and verify paths from being able to
/// silently disagree about what an overlay's content was after some future,
/// partial refactor of either one.
///
/// # Stability
///
/// Delegates the actual hashing to
/// [`scythe_codegen::provenance::options_fingerprint`], which documents the
/// order-, machine-, and run-independence guarantees in full. The two things
/// this function itself is responsible for keeping stable:
///
/// - The overlay is hashed by *content* (the bytes `std::fs::read_to_string`
///   returns), never by `resolved` or `display_path` -- both are absolute
///   paths that depend on where the project happens to be checked out, and
///   embedding either would make the fingerprint differ between two
///   otherwise-identical checkouts of the same project.
/// - `target.options` is a `HashMap<String, String>`, whose iteration order
///   here is left entirely to `options_fingerprint` to normalize (it sorts
///   by key) -- this function passes the pairs through in whatever order
///   `.iter()` happens to yield them.
fn resolve_options_fingerprint(target: &ResolvedGenTarget, base_dir: &Path) -> Result<String, String> {
    let manifest_overlay_content = match &target.manifest_override {
        Some(manifest_path) => {
            let resolved = base_dir.join(manifest_path);
            let display_path = std::path::absolute(&resolved).unwrap_or_else(|_| resolved.clone());
            let content = std::fs::read_to_string(&resolved).map_err(|e| {
                format!(
                    "backend '{}': failed to read manifest override '{}' while fingerprinting options: {e}",
                    target.backend,
                    display_path.display()
                )
            })?;
            Some(content)
        }
        None => None,
    };

    let options: Vec<(&str, &str)> = target
        .options
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    Ok(scythe_codegen::provenance::options_fingerprint(
        options,
        manifest_overlay_content.as_deref(),
    ))
}

/// Validate that every `[[sql.gen]]` target in `sql_config` would actually
/// construct -- resolving gen targets, building each backend, applying its
/// manifest override and options -- without writing anything, and report
/// each one that would not as an `SC-PRV09` error finding.
///
/// Mirrors exactly the fallible steps `run_generate_inner` performs before
/// it writes a single byte, so `scythe check` reports precisely the configs
/// `scythe generate` refuses (issue #209, item 1) -- previously the only
/// signal was `verify_provenance`'s SC-PRV07, and only once a stale
/// artifact happened to already exist on disk to check provenance against;
/// a config with no artifact yet generated produced no finding at all, and
/// `[main] All queries valid.` printed regardless.
///
/// Returns a plain `Vec`, like [`verify_provenance`], rather than a
/// `Result` propagated via `?`: this function's finding must move the exit
/// code (it is `Error`, not `Warn`, unlike SC-PRV07), but it must not be
/// able to abort `run_check` before `emit_findings` runs, which would
/// discard every other finding already collected for this block and every
/// block after it -- and, for `check --output r.json`, leave no report
/// file written at all. See
/// `run_check_still_emits_lint_findings_when_a_gen_target_cannot_be_constructed`,
/// which pins exactly that: a real SC-S03 lint finding must survive an
/// unconstructable `[[sql.gen]]` target in the same config, not disappear
/// with it.
fn validate_gen_targets_constructible(sql_config: &SqlConfig, base_dir: &Path) -> Vec<QueryViolation> {
    let mut violations = Vec::new();

    let gen_targets = match resolve_gen_targets(sql_config) {
        Ok(targets) => targets,
        Err(e) => {
            violations.push(QueryViolation {
                query_name: sql_config.name.clone(),
                rule_id: Cow::Borrowed("SC-PRV09"),
                severity: Severity::Error,
                message: format!(
                    "failed to resolve [sql.gen] targets: {e} (run `scythe generate` for the full diagnosis)"
                ),
            });
            return violations;
        }
    };

    for target in &gen_targets {
        let target_label = format!("{}:{}", sql_config.name, target.backend);

        let mut backend = match get_backend(&target.backend, &sql_config.engine) {
            Ok(backend) => backend,
            Err(e) => {
                violations.push(QueryViolation {
                    query_name: target_label,
                    rule_id: Cow::Borrowed("SC-PRV09"),
                    severity: Severity::Error,
                    message: format!(
                        "backend '{}' with engine '{}' could not be constructed: {} (run `scythe generate` \
                         for the full diagnosis)",
                        target.backend, sql_config.engine, e
                    ),
                });
                continue;
            }
        };

        if let Some(ref manifest_path) = target.manifest_override
            && let Err(e) = apply_manifest_override(backend.manifest_mut(), &target.backend, manifest_path, base_dir)
        {
            violations.push(QueryViolation {
                query_name: target_label.clone(),
                rule_id: Cow::Borrowed("SC-PRV09"),
                severity: Severity::Error,
                message: format!(
                    "manifest override could not be applied: {e} (run `scythe generate` for the full diagnosis)"
                ),
            });
            continue;
        }

        if !target.options.is_empty()
            && let Err(e) = backend.apply_options(&target.options)
        {
            violations.push(QueryViolation {
                query_name: target_label.clone(),
                rule_id: Cow::Borrowed("SC-PRV09"),
                severity: Severity::Error,
                message: format!(
                    "backend options could not be applied: {e} (run `scythe generate` for the full diagnosis)"
                ),
            });
        }

        // `scythe generate` refuses (by default, i.e. without
        // `--allow-output-escape`) to write outside the project root -- see
        // `resolve_contained_output` and #207. `check` has no equivalent
        // flag of its own, so it validates under that same strict default:
        // an `output` that would be rejected without `--allow-output-escape`
        // is reported here too. Before this, `check` never called
        // `resolve_contained_output` at all, so a config with a
        // path-traversing `output` (e.g. `output = "../../ESCAPED"`) passed
        // `scythe check` cleanly and was only ever refused once `scythe
        // generate` actually ran against it -- a check that is green where
        // generate is red is worse than no check.
        if let Err(e) = resolve_contained_output(base_dir, &target.output, &target.backend, false) {
            violations.push(QueryViolation {
                query_name: target_label,
                rule_id: Cow::Borrowed("SC-PRV09"),
                severity: Severity::Error,
                message: format!("{e} (run `scythe generate` for the full diagnosis)"),
            });
        }
    }

    violations
}

/// A single generated query's code alongside the enum definitions it
/// references, kept together so [`assemble_output`] can dedupe enums and
/// interleave (or separate) query code without re-deriving either from
/// `analyzed_queries`.
struct QueryResult {
    code: scythe_codegen::GeneratedCode,
    enums: Vec<EnumInfo>,
    /// Enums this query reaches *through a nested-aggregate field*, which
    /// need the nested-capable definition (extra serde derives and variant
    /// renames) rather than the plain driver-only one.
    ///
    /// Carried per query rather than passed alongside `results` because the
    /// decision needs `AnalyzedQuery`, which [`assemble_body`] does not have;
    /// it unions these across queries instead. The nested-capable form is a
    /// superset of the plain one, so one query needing it decides for the
    /// whole file safely.
    nested_enum_names: Vec<String>,
}

/// Build the provenance header line assembly prepends to every generated
/// file, pinning the embedded `v=` field to *this binary's* version.
///
/// The construction itself lives in
/// [`scythe_codegen::provenance::header_line_with_options`] so that
/// `scythe-codegen`'s `tool_validation` harness can assemble the same bytes
/// and hand them to `php -l` / `ruby -c` / `gofmt`; see that module's doc
/// comment. This wrapper exists only to supply the version, which is not
/// `scythe-codegen`'s to decide: the number that belongs in the header is the
/// version of the `scythe` binary that wrote the file, because that is
/// exactly what [`verify_artifact`]'s SC-PRV02 check compares against.
///
/// `options` is the `opt1:<16 hex>` fingerprint from
/// [`resolve_options_fingerprint`] (GH #155) -- the `options=`
/// counterpart to `schema`'s `sch1:` and `queries`'s `q1:` fields.
fn provenance_header_line(
    backend: &dyn CodegenBackend,
    engine: &str,
    schema: &str,
    queries: &str,
    options: &str,
) -> String {
    scythe_codegen::provenance::header_line_with_options(
        backend,
        env!("CARGO_PKG_VERSION"),
        engine,
        schema,
        queries,
        options,
    )
}

/// Reject two queries destined for one output file whose row/model struct or
/// enum declaration resolves to the same generated identifier with a
/// *different* rendered body.
///
/// `scythe-codegen`'s `resolve::check_type_name_collisions` only compares
/// names *within one query's own* enum list, plus that query's own row/model
/// type -- it has no view of any other query written into the same file.
/// Two different queries can independently derive the same `struct_case`
/// name here: `@name get_user` and `@name GetUser` both PascalCase to
/// `GetUserRow`, and nothing caught that before `assemble_body` rendered
/// both declarations into one file (board #200). The same gap exists across
/// two different enums whose SQL names case-convert to the same identifier.
///
/// A name collision with an *identical* rendered body is not an error: it is
/// the same "one declaration, reused" case `assemble_body` already dedupes
/// for a shared table model or a composite reachable from two queries --
/// keyed there on rendered text, and here on declared name plus body, so the
/// two agree on what counts as the same declaration rather than a
/// redeclaration.
///
/// Deliberately independent of `assemble_body`'s own text-exact dedup sets:
/// this runs first, over the full `(name, body)` picture, so it reports the
/// SQL-level source of a collision (`query 'X'`, `enum 'Y'`) rather than
/// leaving two textually different declarations to reach the same generated
/// file and fail whatever compiler reads it next.
fn check_file_level_type_name_collisions(
    analyzed_queries: &[AnalyzedQuery],
    results: &[QueryResult],
    backend: &dyn CodegenBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    let naming = &backend.manifest().naming;
    let mut declared: AHashMap<String, (String, String)> = AHashMap::new();

    for (analyzed, result) in analyzed_queries.iter().zip(results) {
        if let Some(ref table_name) = analyzed.source_table {
            if let Some(ref body) = result.code.model_struct {
                let name = scythe_codegen::model_struct_name(table_name, naming);
                record_type_declaration(&mut declared, name, format!("table '{table_name}'"), body)?;
            }
        } else if let Some(ref body) = result.code.row_struct {
            let name = row_struct_name(&analyzed.name, naming);
            record_type_declaration(&mut declared, name, format!("query '{}'", analyzed.name), body)?;
        }

        for info in &result.enums {
            let nested = result.nested_enum_names.iter().any(|n| n == &info.sql_name);
            let body = generate_single_enum_def_with_backend(info, backend, nested)
                .map_err(|e| format!("rendering enum '{}' for collision check: {e}", info.sql_name))?;
            let name = enum_type_name(&info.sql_name, naming);
            record_type_declaration(&mut declared, name, format!("enum '{}'", info.sql_name), &body)?;
        }
    }

    Ok(())
}

/// Record one generated declaration under `name`. A second declaration under
/// the same `name` is accepted silently when its body is byte-identical to
/// the first (the intended dedup), and rejected otherwise.
fn record_type_declaration(
    declared: &mut AHashMap<String, (String, String)>,
    name: String,
    source: String,
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some((prev_source, prev_body)) = declared.get(&name) {
        if prev_body != body {
            return Err(format!(
                "{prev_source} and {source} both generate a declaration named '{name}' with different bodies -- \
                 rename one of them (@name, or the qualifying schema/table) so the generated names differ"
            )
            .into());
        }
        return Ok(());
    }
    declared.insert(name, (source, body.to_string()));
    Ok(())
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
/// state and test that invariant at all. That one place is
/// [`scythe_codegen::provenance::assemble_file`], which also documents why
/// the blank separator after the provenance line is conditional on the
/// preamble being non-empty.
fn assemble_output(
    backend: &dyn CodegenBackend,
    results: &[QueryResult],
    engine: &str,
    schema: &str,
    queries: &str,
    options: &str,
) -> String {
    scythe_codegen::provenance::assemble_file(
        &backend.file_preamble(),
        &provenance_header_line(backend, engine, schema, queries, options),
        &assemble_body(backend, results),
    )
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
    // Unioned across queries: an enum is emitted once for the whole file, so
    // any query reaching it through a nested aggregate decides the form for
    // all of them. That form is a superset of the plain one, so widening is
    // safe -- see `QueryResult::nested_enum_names`.
    let nested_enum_names: AHashSet<&str> = results
        .iter()
        .flat_map(|result| result.nested_enum_names.iter().map(String::as_str))
        .collect();

    let mut seen_enums = AHashSet::new();
    let mut unique_enum_defs: Vec<String> = Vec::new();
    for result in results {
        for info in &result.enums {
            if seen_enums.insert(info.sql_name.clone())
                && let Ok(def) = generate_single_enum_def_with_backend(
                    info,
                    backend,
                    nested_enum_names.contains(info.sql_name.as_str()),
                )
            {
                unique_enum_defs.push(def);
            }
        }
    }

    let mut seen_nested = AHashSet::new();
    let mut unique_nested_defs: Vec<String> = Vec::new();
    for result in results {
        for def in &result.code.nested_struct_defs {
            if seen_nested.insert(def.name.clone()) {
                unique_nested_defs.push(def.code.clone());
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

    // Before the row structs: Python evaluates a class body's annotations
    // eagerly, so a row dataclass annotated `list[GetUserPostsRowPosts]`
    // needs that name already bound.
    for def in &unique_nested_defs {
        output_parts.push(def.clone());
    }

    // `model_struct` carries a table's `SELECT *` model and/or the composite
    // type declarations reachable from this query, rendered as one text blob
    // per query (see `generate_with_backend_and_overrides` in
    // `scythe-codegen`). Two queries that select the same composite column,
    // or that both `SELECT *` from the same table with the same projection,
    // render byte-identical text -- `generate_composite_def` /
    // `generate_model_struct` are pure functions of the composite/table
    // shape, not of which query asked for them. Unlike `enum_def` and
    // `nested_struct_defs` above, there is no per-item name to key a dedup
    // on here, only the assembled text; deduping on that text is exact
    // (never merges two declarations that happen to differ) and is what
    // stops a language whose compiler rejects a redeclared type -- Java's
    // `record ... is already defined`, Kotlin's `redeclaration:`, plus Rust,
    // Go, C#, and PHP, which reject a duplicate struct/type/record/class the
    // same way -- from ever seeing this file (board #166).
    let mut seen_model_structs: AHashSet<&str> = AHashSet::new();
    let class_header = backend.query_class_header();
    if class_header.is_empty() {
        for result in results {
            if let Some(ref s) = result.code.model_struct
                && seen_model_structs.insert(s.as_str())
            {
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
            if let Some(ref s) = result.code.model_struct
                && seen_model_structs.insert(s.as_str())
            {
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
/// the `.java` capitalization rule lives in exactly one place: header
/// verification (`artifact_paths`) has to locate the same file this names,
/// and a second copy of the rule would diverge silently.
fn output_filename(backend: &dyn CodegenBackend) -> String {
    let ext = backend.output_extension();
    if ext == "java" {
        format!("Queries.{}", ext)
    } else {
        format!("queries.{}", ext)
    }
}

/// Filename of the RBS type-signature file Ruby backends emit alongside
/// `queries.rb`. Fixed, not derived from the manifest: RBS is Ruby's own
/// signature format, so the extension is a property of RBS rather than of
/// the backend.
const RBS_FILENAME: &str = "queries.rbs";

/// Whether `backend` emits an RBS signature file alongside its main output.
///
/// Probes [`CodegenBackend::generate_rbs_file`] with an empty context rather
/// than testing `manifest().backend.language == "ruby"`: the trait method's
/// `None` default *is* the definition of "does not emit RBS", so asking it
/// directly cannot disagree with what generation actually does. Shared by
/// [`generate_rbs_if_supported`] and [`artifact_paths`] so writing and
/// verifying can never end up with different ideas about which targets have
/// a second artifact.
fn backend_emits_rbs(backend: &dyn CodegenBackend) -> bool {
    let empty_context = RbsGenerationContext {
        queries: vec![],
        enums: vec![],
    };
    backend.generate_rbs_file(&empty_context).is_some()
}

/// The provenance-header fields that are derived once per `[[sql.gen]]`
/// target and then travel together down every generation path.
///
/// They are grouped rather than passed individually because they are only
/// ever read as a set — by [`provenance_header_line`], and by nothing else —
/// and because passing them separately pushed both
/// [`generate_for_backend`] and [`generate_rbs_if_supported`] past clippy's
/// argument-count threshold when `queries` was added.
#[derive(Clone, Copy)]
struct ProvenanceFields<'a> {
    /// The raw `[[sql]]` engine alias (e.g. `"mariadb"`), exactly as the
    /// user wrote it — deliberately not normalized, so the header records
    /// the configured engine rather than the one inference resolved it to.
    engine: &'a str,
    /// The current [`scythe_core::catalog::Catalog::fingerprint`].
    schema: &'a str,
    /// The sibling
    /// [`scythe_core::analyzer::AnalyzedQuery::fingerprint_set`] over the
    /// analyzed query set — see that method's doc comment for what
    /// participates.
    queries: &'a str,
    /// The `opt1:<16 hex>` fingerprint from [`resolve_options_fingerprint`]
    /// (GH #155) over this *target's* resolved `[[sql.gen]]` options
    /// and manifest overlay contents -- unlike `engine`/`schema`/`queries`,
    /// which are shared by every target under one `[[sql]]` block, this one
    /// is per-target (two targets in the same block can configure different
    /// options), which is why it is computed alongside the other three
    /// inside the per-target loop rather than once per block.
    options: &'a str,
}

/// Generate output for a single backend target.
///
/// `provenance` is threaded through to [`assemble_output`] for the header
/// line every generated file now carries. Returns whether
/// `--validate-output` (`validate_output`) found a real problem in this
/// target's generated code -- `false` whenever `validate_output` is `false`,
/// since nothing was checked to fail.
fn generate_for_backend(
    config_name: &str,
    backend: &dyn CodegenBackend,
    analyzed_queries: &[AnalyzedQuery],
    output_dir: &str,
    overrides: &[TypeOverride],
    provenance: ProvenanceFields<'_>,
    validate_output: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let ProvenanceFields {
        engine,
        schema,
        queries,
        options,
    } = provenance;
    // ~keep Nested struct names are deduplicated by (name, shape) only *within*
    // one analyze() call, so two queries in the same file can each derive
    // the same name -- `to_snake_case` collapses `GetUserPosts` and
    // `GETUserPosts` onto one snake_case stem, and `@name` is free-form, so
    // this is reachable rather than theoretical. Emitting both definitions
    // is E0428 in Rust and a redeclaration everywhere else. Same name plus
    // same shape is one definition; same name plus a different shape is
    // unresolvable and must fail loudly rather than silently give one query
    // the other's type.
    let mut nested_shapes: AHashMap<&str, &scythe_core::analyzer::NestedStructInfo> = AHashMap::new();
    for analyzed in analyzed_queries {
        for nested in &analyzed.nested_structs {
            if let Some(existing) = nested_shapes.insert(nested.name.as_str(), nested)
                && existing.fields != nested.fields
            {
                return Err(format!(
                    "two queries in this output file derive the nested struct name '{}' from different row \
                     shapes; rename one of the queries or its output column (@name / column alias) so the \
                     generated struct names differ",
                    nested.name
                )
                .into());
            }
        }
    }

    let mut results: Vec<QueryResult> = Vec::new();
    for analyzed in analyzed_queries {
        let code = generate_with_backend_and_overrides(analyzed, backend, overrides)?;

        // `analyzed.enums` covers enums reachable from the top-level
        // columns/params *and* from nested-aggregate fields. Emitting one
        // that is only reachable through a nested struct this backend
        // degraded away would add an unused definition that did not exist
        // before nested-aggregate inference, breaking the byte-identity
        // guarantee the degradation pass exists to provide.
        let mut nested_enum_names: Vec<String> = Vec::new();
        let enums: Vec<EnumInfo> = analyzed
            .enums
            .iter()
            .filter(|info| {
                let from_nested = scythe_codegen::nested_type_is_emitted(
                    analyzed,
                    &code.nested_struct_defs,
                    "enum::",
                    &info.sql_name,
                );
                if from_nested {
                    nested_enum_names.push(info.sql_name.clone());
                    return true;
                }
                analyzed
                    .columns
                    .iter()
                    .any(|col| col.neutral_type.strip_prefix("enum::") == Some(info.sql_name.as_str()))
                    || analyzed
                        .params
                        .iter()
                        .any(|param| param.neutral_type.strip_prefix("enum::") == Some(info.sql_name.as_str()))
            })
            .cloned()
            .collect();

        results.push(QueryResult {
            code,
            enums,
            nested_enum_names,
        });
    }

    check_file_level_type_name_collisions(analyzed_queries, &results, backend)?;

    let mut output_content = assemble_output(backend, &results, engine, schema, queries, options);

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

    let validation_failed = if validate_output {
        report_generated_code_validation(config_name, &output_file, backend.name(), &output_content)
    } else {
        false
    };

    generate_rbs_if_supported(config_name, backend, analyzed_queries, overrides, out_path, provenance)?;

    Ok(validation_failed)
}

/// Report the outcome of validating one target's generated output against
/// its real compiler/linter (`scythe_codegen::validation::validate_generated_code`),
/// and say whether it counts as a failure that should move `generate`'s exit
/// code.
///
/// Surfaces the same three states [`ValidationOutcome`] distinguishes,
/// rather than collapsing any pair of them:
///
/// - `VALIDATED` -- a real tool ran to completion and found nothing wrong.
/// - `SKIPPED` -- nothing was actually checked, either because this
///   backend's language has no tool-based validator at all
///   ([`ValidationOutcome::Unsupported`]), or because every tool this
///   validator would have used is not installed. The latter is
///   [`ValidationOutcome::Passed`] with an empty `tools_run` -- that
///   variant's own default policy calls it a pass, but reporting a run that
///   inspected nothing as "validated" would recreate the exact unfalsifiable
///   gate `--validate-output` exists to close, so it is reported as a skip
///   here instead.
/// - `FAILED` -- a real tool ran and found a problem, or the harness itself
///   could not drive an installed tool.
///
/// Only `FAILED` returns `true`; `SKIPPED` never moves `generate`'s exit
/// code, matching [`ValidationOutcome::is_failure`]'s own non-strict policy.
fn report_generated_code_validation(config_name: &str, output_file: &Path, backend_name: &str, code: &str) -> bool {
    let display_path = output_file.display();

    match validate_generated_code(code, backend_name) {
        ValidationOutcome::Unsupported => {
            eprintln!(
                "[{config_name}] {display_path}: generated-code validation SKIPPED (no tool-based validator for \
                 backend '{backend_name}')"
            );
            false
        }
        ValidationOutcome::Passed {
            tools_run,
            tools_missing,
        } if tools_run.is_empty() => {
            eprintln!(
                "[{config_name}] {display_path}: generated-code validation SKIPPED ({} not installed; nothing \
                 was verified)",
                tools_missing.join(", ")
            );
            false
        }
        ValidationOutcome::Passed {
            tools_run,
            tools_missing,
        } => {
            if tools_missing.is_empty() {
                eprintln!(
                    "[{config_name}] {display_path}: generated-code validation VALIDATED ({})",
                    tools_run.join(", ")
                );
            } else {
                eprintln!(
                    "[{config_name}] {display_path}: generated-code validation VALIDATED ({}); SKIPPED ({} not \
                     installed)",
                    tools_run.join(", "),
                    tools_missing.join(", ")
                );
            }
            false
        }
        ValidationOutcome::Failed {
            tools_run,
            tools_missing,
            errors,
        } => {
            eprintln!(
                "[{config_name}] {display_path}: generated-code validation FAILED ({})",
                tools_run.join(", ")
            );
            for error in &errors {
                eprintln!("  {error}");
            }
            if !tools_missing.is_empty() {
                eprintln!(
                    "[{config_name}] {display_path}: generated-code validation SKIPPED ({} not installed)",
                    tools_missing.join(", ")
                );
            }
            true
        }
    }
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
///
/// `provenance` is threaded through for the same reason
/// [`generate_for_backend`] takes it: `queries.rbs` gets a provenance
/// header too. It is a tracked artifact in all six `integration_tests/ruby-*`
/// projects, and its content is schema-derived — a column changing type or
/// nullability rewrites the RBS signatures. Emitting it without a header
/// would have left the one generated file per Ruby target that `scythe
/// check` could never report as drifted, which is worse than not verifying
/// it at all: RBS is what `steep` type-checks the caller's code against, so
/// a stale `.rbs` reports type errors against a schema that no longer exists.
fn generate_rbs_if_supported(
    config_name: &str,
    backend: &dyn CodegenBackend,
    analyzed_queries: &[AnalyzedQuery],
    overrides: &[TypeOverride],
    out_path: &Path,
    provenance: ProvenanceFields<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !backend_emits_rbs(backend) {
        return Ok(());
    }
    let ProvenanceFields {
        engine,
        schema,
        queries,
        options,
    } = provenance;

    let manifest = backend.manifest();
    let naming = &manifest.naming;

    let mut rbs_queries: Vec<RbsQueryInfo> = Vec::new();
    let mut seen_enums = AHashSet::new();
    let mut rbs_enums: Vec<RbsEnumInfo> = Vec::new();

    for analyzed in analyzed_queries {
        let source_table = analyzed.source_table.as_deref().unwrap_or("");
        let func = fn_name(&analyzed.name, naming);

        if analyzed.command == QueryCommand::Grouped {
            // A `:grouped` query's RBS needs the same parent/child column split as its `.rb`
            // counterpart (`scythe_codegen::generate_with_backend_and_overrides`'s own
            // `QueryCommand::Grouped` branch), not the flat `analyzed.columns` the branch
            // below resolves -- resolving the flat list here is GH #203: the emitted `.rbs`
            // described one class with neither the `children` field nor the child class the
            // `.rb` `Data.define`s actually declare. `group_by` is guaranteed `Some` by the
            // time we get here in practice (the `.rb` path runs first for every query in
            // `generate_for_backend` and returns the identical "missing @group_by" error if
            // it isn't, aborting the whole `generate` command before this RBS path runs), but
            // this is handled as a real error rather than `unwrap`/`expect` regardless.
            let group_by = analyzed.group_by.as_ref().ok_or_else(|| {
                format!(
                    "query '{}' is :grouped but is missing @group_by annotation",
                    analyzed.name
                )
            })?;

            let parent_struct_name = row_struct_name(&analyzed.name, naming);

            // Same degradation pass as the non-grouped branch below, run separately against
            // each half of the split (mirroring `generate_with_backend_and_overrides`) since a
            // :grouped query's parent/child columns are a distinct copy of `analyzed.columns`.
            let (degraded_parent, degraded_child) = if analyzed.nested_structs.is_empty() {
                (None, None)
            } else {
                let (dp, _) =
                    degrade_unsupported_nested_structs(&group_by.parent_columns, &analyzed.nested_structs, backend)?;
                let (dc, _) =
                    degrade_unsupported_nested_structs(&group_by.child_columns, &analyzed.nested_structs, backend)?;
                (Some(dp), Some(dc))
            };
            let parent_cols = scythe_codegen::resolve::resolve_columns(
                degraded_parent.as_deref().unwrap_or(&group_by.parent_columns),
                manifest,
                overrides,
            )?;
            let child_cols = scythe_codegen::resolve::resolve_columns(
                degraded_child.as_deref().unwrap_or(&group_by.child_columns),
                manifest,
                overrides,
            )?;
            let params = scythe_codegen::resolve::resolve_params(&analyzed.params, manifest, overrides, source_table)?;

            let needs_struct = !analyzed.columns.is_empty();

            rbs_queries.push(RbsQueryInfo {
                func_name: func,
                struct_name: if needs_struct { Some(parent_struct_name) } else { None },
                columns: parent_cols,
                child_columns: child_cols,
                params,
                // Kept as `Grouped` (not rewritten to `Many`) so `ruby_rbs.rs` can tell a
                // grouped query apart from a flat `:many` one and emits both the parent and
                // the child class instead of one flat class (#203).
                command: QueryCommand::Grouped,
            });
        } else {
            // This path resolves columns independently of the `.rb` file (rather than reusing
            // its output) and would otherwise reference a nested-struct type name the backend
            // never defines anywhere, for any backend that hasn't opted in. Skipped when
            // nested_structs is empty (the common case) for the same zero-copy reason as the
            // grouped branch above.
            let degraded_columns = if analyzed.nested_structs.is_empty() {
                None
            } else {
                let (cols, _defs) =
                    degrade_unsupported_nested_structs(&analyzed.columns, &analyzed.nested_structs, backend)?;
                Some(cols)
            };
            let columns = scythe_codegen::resolve::resolve_columns(
                degraded_columns.as_deref().unwrap_or(&analyzed.columns),
                manifest,
                overrides,
            )?;
            let params = scythe_codegen::resolve::resolve_params(&analyzed.params, manifest, overrides, source_table)?;

            let struct_name = determine_struct_name(analyzed, naming);
            let needs_struct =
                matches!(analyzed.command, QueryCommand::One | QueryCommand::Many) && !analyzed.columns.is_empty();

            rbs_queries.push(RbsQueryInfo {
                func_name: func,
                struct_name: if needs_struct { Some(struct_name) } else { None },
                columns,
                // Only a `:grouped` query has child columns; the branch above
                // is the one that fills this.
                child_columns: Vec::new(),
                params,
                command: analyzed.command.clone(),
            });
        }

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
        // No preamble: RBS has no construct that must be the first bytes of
        // the file (`generate_rbs_content` opens with its own `#` comment),
        // so the header goes straight on line 1 with no separator — the same
        // shape `assemble_output` produces for the 44 backends without a
        // preamble. The comment token comes from the backend's manifest
        // language (`ruby` -> `#`), which is also RBS's comment syntax.
        let rbs_content = scythe_codegen::provenance::assemble_file(
            "",
            &provenance_header_line(backend, engine, schema, queries, options),
            &rbs_content,
        );

        let rbs_file = out_path.join(RBS_FILENAME);
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
    /// `--exit-zero` flag: always exit 0 even with error-severity findings.
    pub exit_zero: bool,
}

pub fn run_check(opts: RunCheckOpts) -> Result<(), Box<dyn std::error::Error>> {
    use scythe_lint::reporters::{Finding, Format};
    use scythe_lint::{LintContext, LintEngine, default_registry, emit_findings, provenance_registry};

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

    // The `SC-PRV*` rules are kept out of `default_registry` (they implement
    // neither `check_query` nor `check_catalog`, so `scythe lint` and
    // `scythe audit --list-rules` would advertise rules they can never
    // emit). They still need the user's `[lint]` table applied to them, and
    // it must be the *same* table, so that one `[lint.rules]` /
    // `[lint.categories]` block governs SQL rules and provenance rules
    // alike.
    let mut provenance_rules = provenance_registry();
    if let Some(ref lint_config) = config.lint {
        provenance_rules.apply_config(lint_config);
    }
    let provenance_severities = ProvenanceSeverities::from_registry(&provenance_rules);

    let mut all_violations: Vec<QueryViolation> = Vec::new();
    // Queries are grouped per `[[sql]]` block so verification connects once and
    // labels findings with the block's engine. `engine` is carried alongside
    // the queries so `verify_against_database` can skip non-PostgreSQL blocks
    // instead of running every query through the PostgreSQL wire protocol.
    let mut verifiable: Vec<VerifiableBlock> = Vec::new();

    // Maps a query's name back to the `.sql` file it came from, so findings
    // can be attributed to the real source file instead of `scythe.toml`
    // for every finding regardless of which query tripped it (#209, item 3).
    // Catalog-wide and config-level findings (empty `query_name`) have no
    // single file to attribute to and keep using `config_path`.
    let mut query_file_by_name: AHashMap<String, String> = AHashMap::new();

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

        // Marks where this block's violations start in `all_violations`, so
        // the "All queries valid." message below can be conditioned on
        // whether *this* block actually is valid, instead of being printed
        // unconditionally after findings (including this block's own) have
        // already been collected. See #209, item 2. Captured before the
        // gen-target validation just below so an SC-PRV09 finding also
        // counts toward that decision.
        let block_violations_start = all_violations.len();

        // Validated eagerly, exactly as `scythe generate` constructs every
        // target before writing anything: `check` exists to validate what
        // `generate` would do without doing it, so a config `generate`
        // refuses (an unresolvable `[sql.gen]`, or a backend/engine pair
        // that will not construct) is now reported as an `SC-PRV09` error
        // finding -- not silently degraded to a droppable SC-PRV07
        // *warning* that only fires once an artifact already happens to
        // exist on disk to check provenance against. See #209, item 1.
        all_violations.extend(validate_gen_targets_constructible(sql_config, base_dir));

        let query_files = resolve_globs(&sql_config.queries, base_dir, &format!("[{}] queries", sql_config.name))?;
        let mut all_query_blocks: Vec<(String, String)> = Vec::new();
        for query_file in &query_files {
            let content = std::fs::read_to_string(query_file)
                .map_err(|e| format!("failed to read query file '{}': {}", query_file, e))?;
            if has_unannotated_sql(&content) {
                return Err(format!(
                    "[{}] query file '{}' has SQL content with no `-- name:` / `-- @name` annotation above it; \
                     that content is silently dropped, not checked. Add an annotation above every statement \
                     (e.g. `-- name: MyQuery :one`), or move non-query SQL out of the `queries` glob",
                    sql_config.name, query_file
                )
                .into());
            }
            let blocks = split_query_file(&content);
            // A file with real content (a license header, a query commented
            // out by hand, a mistyped annotation like `--name:`) that still
            // yields zero blocks is, from here, indistinguishable from one
            // whose annotations were simply never recognised -- exactly what
            // `check` exists to catch. Reported instead of falling through
            // to the unconditional "All queries valid." below (#186). A
            // genuinely empty (or whitespace-only) file is left alone: there
            // is nothing there that could have been misrecognised.
            if blocks.is_empty() && !content.trim().is_empty() {
                all_violations.push(QueryViolation {
                    query_name: String::new(),
                    rule_id: Cow::Borrowed("SC-PRV10"),
                    severity: Severity::Error,
                    message: format!(
                        "[{}] query file '{}' has content but produced zero `-- name:` / `-- @name` query \
                         blocks; nothing in this file was checked. If every statement here is intentionally \
                         commented out, or this file has no queries, remove it from the `queries` glob",
                        sql_config.name, query_file
                    ),
                });
            }
            for block in blocks {
                all_query_blocks.push((query_file.clone(), block));
            }
        }

        eprintln!("[{}] Checking {} queries...", sql_config.name, all_query_blocks.len());

        let mut query_names: Vec<String> = Vec::new();
        let mut analyzed_queries: Vec<scythe_core::analyzer::AnalyzedQuery> = Vec::new();

        for (query_file, block) in &all_query_blocks {
            // A parse or analyze failure used to propagate via `?`, aborting
            // `run_check` before `emit_findings` and discarding every
            // finding already collected -- for `check --output r.json` that
            // meant no report file was written at all. Both are now
            // findings instead, exactly like `verify_provenance` already
            // treats an unconstructible backend: unable to fully validate
            // this one query is not the same as nothing being wrong with
            // the rest of the file. See #209, item 4.
            let parsed = match parse_query_with_dialect(block, &dialect) {
                Ok(p) => p,
                Err(e) => {
                    all_violations.push(QueryViolation {
                        query_name: String::new(),
                        rule_id: Cow::Borrowed("SC-PARSE01"),
                        severity: Severity::Error,
                        message: format!("{query_file}: failed to parse query: {e}"),
                    });
                    continue;
                }
            };
            let analyzed = match analyze(&catalog, &parsed) {
                Ok(a) => a,
                Err(e) => {
                    all_violations.push(QueryViolation {
                        query_name: parsed.annotations.name.clone(),
                        rule_id: Cow::Borrowed("SC-PARSE02"),
                        severity: Severity::Error,
                        message: format!(
                            "{query_file}: failed to analyze query '{}': {e}",
                            parsed.annotations.name
                        ),
                    });
                    continue;
                }
            };

            query_names.push(analyzed.name.clone());
            query_file_by_name.insert(analyzed.name.clone(), query_file.clone());

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

            // Unconditionally retained, unlike before #94: `verify_provenance`
            // now needs the full analyzed query set for every `scythe check`
            // run, not just ones with `--database-url`, to compute the
            // current `queries=` fingerprint for SC-PRV08. The
            // `--database-url`-gated cost this used to avoid was `queries`
            // being consumed a second time by `VerifiableBlock` below (for
            // live-database verification) -- that part of the retention is
            // still conditional; only the always-needed part (holding
            // `analyzed` long enough to fingerprint it) is not.
            analyzed_queries.push(analyzed);
        }

        // ~keep Computed from every analyzed query in this block, exactly as
        // `run_generate` computes the same value -- see
        // `AnalyzedQuery::fingerprint_set` for what participates. Read
        // before the possible move into `VerifiableBlock` just below.
        let queries_fingerprint = AnalyzedQuery::fingerprint_set(&analyzed_queries);

        if should_retain_for_verification(opts.database_url.as_deref()) {
            verifiable.push(VerifiableBlock {
                name: sql_config.name.clone(),
                engine: sql_config.engine.clone(),
                queries: analyzed_queries,
                schema: scythe_inspect::describe_catalog(&catalog)?,
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

        // Deliberately not `?`-propagating: `verify_provenance` returns a
        // plain `Vec` precisely so it cannot abort the run before
        // `emit_findings` below and take every other block's findings with
        // it. See its doc comment.
        all_violations.extend(verify_provenance(
            sql_config,
            &catalog,
            &queries_fingerprint,
            base_dir,
            provenance_severities,
        ));

        // Printed only when this block produced no error-severity finding
        // -- previously printed unconditionally, so `[main] All queries
        // valid.` appeared even immediately above an SC-S03 error for the
        // very same block. See #209, item 2.
        let block_has_error = all_violations[block_violations_start..]
            .iter()
            .any(|v| matches!(v.severity, Severity::Error));
        if block_has_error {
            let block_errors = all_violations[block_violations_start..]
                .iter()
                .filter(|v| matches!(v.severity, Severity::Error))
                .count();
            eprintln!("[{}] {} error(s) found.", sql_config.name, block_errors);
        } else {
            eprintln!("[{}] All queries valid.", sql_config.name);
        }
    }

    let mut findings: Vec<Finding> = all_violations
        .iter()
        .filter(|qv| !matches!(qv.severity, Severity::Off))
        .map(|qv| Finding {
            // Attributed to the `.sql` file the query actually came from
            // when one is known, instead of always naming `scythe.toml` --
            // SARIF/JSON consumers (e.g. GitHub code scanning) otherwise
            // annotate the config file for a violation that lives in the
            // query file, and the human reporter groups findings under the
            // wrong heading. Catalog-wide findings (empty `query_name`, e.g.
            // SC-C01) and ones whose query never made it far enough to be
            // named (a parse failure) have no single file to attribute to
            // and keep `config_path`. See #209, item 3.
            file: query_file_by_name
                .get(&qv.query_name)
                .cloned()
                .unwrap_or_else(|| config_path.to_string()),
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
        // Drift rules live in their own registry: `scythe lint` and
        // `scythe audit` cannot observe a live database, so listing SC-DRF*
        // among their rules would advertise rules those commands can never
        // report. The same `[lint]` config is applied, so
        // `rules."SC-DRF02" = "error"` tunes drift like any other rule.
        // Built here rather than alongside the lint registry so that a run
        // without `--database-url` does no drift work at all.
        let mut drift_registry = scythe_lint::drift_registry();
        if let Some(ref lint_config) = config.lint {
            drift_registry.apply_config(lint_config);
        }
        let drift_severities = scythe_inspect::DriftSeverities::from_registry(&drift_registry);

        findings.extend(verify_against_database(url, &verifiable, &drift_severities)?);
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

    // `emit_findings` and the flush above have already run, so the report is
    // on disk (or stdout) before any exit happens here — see
    // `run_check_still_emits_lint_findings_when_a_gen_target_cannot_be_constructed`
    // for the regression this ordering guards against.
    if let Some(code) = check_exit_code(error_count, opts.exit_zero) {
        std::process::exit(code);
    }

    Ok(())
}

/// Decides `check`'s process exit code for error-severity findings, split out
/// from the `std::process::exit` call site so the decision itself stays unit
/// testable: `std::process::exit` tears down the test binary, so nothing
/// after it can be observed in-process. Mirrors `audit`/`inspect`'s
/// severity-to-exit-code convention (see `run_audit` / `run_inspect`):
/// `Some(2)` when error-severity findings are present and `--exit-zero` was
/// not passed, `None` otherwise (the caller falls through to `Ok(())`, exit
/// 0).
fn check_exit_code(error_count: usize, exit_zero: bool) -> Option<i32> {
    if error_count > 0 && !exit_zero { Some(2) } else { None }
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
    /// The block's DDL schema, reduced for schema-drift comparison.
    ///
    /// Captured here because the `Catalog` it comes from is dropped at the end
    /// of the loop iteration that built it, long before the connection exists.
    schema: scythe_inspect::SchemaDescription,
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
    drift_severities: &scythe_inspect::DriftSeverities,
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
            // `error_chain`, not `{e}`: `tokio_postgres::Error` displays as a
            // bare "db error", which makes a wrong password, a missing database
            // and a missing role indistinguishable. The server's `FATAL: ...`
            // text is one level down the source chain.
            .map_err(|e| {
                format!(
                    "failed to connect to '{}': {}",
                    redact_url_password(url),
                    scythe_inspect::error::error_chain(&e)
                )
            })?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("scythe check: database connection error: {e}");
            }
        });

        let mut findings = Vec::new();
        for block in &pg_blocks {
            eprintln!(
                "[{}] Verifying {} queries against the database...",
                block.name,
                block.queries.len()
            );
            findings.extend(scythe_inspect::verify_queries(&client, &block.name, &block.queries).await);
        }

        // Drift is checked separately from query verification because it
        // answers a question preparing a statement cannot: whether the
        // database still has the tables, columns, types and — above all — the
        // nullability the committed DDL claims.
        eprintln!("Checking the schema for drift against the database...");
        let schemas: Vec<(&str, &scythe_inspect::SchemaDescription)> =
            pg_blocks.iter().map(|b| (b.name.as_str(), &b.schema)).collect();
        findings.extend(scythe_inspect::drift_findings(&client, &schemas, drift_severities).await?);

        Ok(findings)
    })
}

/// Sentinel a generated file's provenance header line is built around, e.g.
/// `// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:0123456789abcdef
/// queries=q1:fedcba9876543210`.
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
///
/// `queries` and `options` are deliberately excluded from
/// [`missing_fields`](Self::missing_fields) and therefore from
/// [`is_complete`](Self::is_complete) -- those two stay the *original*
/// four-field completeness contract from #68. A header written before a
/// given field existed (0.14.0 for `queries`, and every version before GH #155
/// for `options`) has no such field at all, and that must read as "drift for
/// this field is not covered for this artifact", not as SC-PRV06
/// malformed-header -- otherwise every artifact already committed before
/// the #155 fix shipped would flag SC-PRV06 the next time `scythe check` ran, with
/// no code change of its own to explain it. See [`verify_artifact`]'s
/// SC-PRV08 and SC-PRV11 checks for where `queries` and `options` actually
/// get used, and why each is compared only when present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProvenanceHeader {
    version: Option<String>,
    backend: Option<String>,
    engine: Option<String>,
    schema: Option<String>,
    /// `q1:<16 hex>` fingerprint of the analyzed query set this file was
    /// generated from (#94). `None` for both "no such field in the header"
    /// (an artifact from scythe 0.14.0 or earlier) and "no header at all" --
    /// both are handled the same way by [`verify_artifact`]'s SC-PRV08
    /// check: skipped, not reported as drift.
    queries: Option<String>,
    /// `opt1:<16 hex>` fingerprint of the `[[sql.gen]]` target's resolved
    /// options and manifest overlay contents (GH #155). `None` for
    /// both "no such field" (an artifact generated before this field
    /// existed) and "no header at all" -- both skipped by
    /// [`verify_artifact`]'s SC-PRV11 check the same way `queries` is
    /// skipped by SC-PRV08, and for the same backward-compatibility reason.
    options: Option<String>,
}

impl ProvenanceHeader {
    /// Names of fields the header is missing, in a stable order, for the
    /// SC-PRV06 "malformed header" message.
    ///
    /// Neither `queries` nor `options` ever appears here -- see the struct
    /// doc comment.
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
            "queries" => header.queries = Some(value.to_string()),
            "options" => header.options = Some(value.to_string()),
            // Forward-compatible: the header format may grow fields later.
            // An older verifier ignoring keys it does not recognize (rather
            // than erroring) is what lets the header format and this
            // verifier evolve independently.
            _ => {}
        }
    }

    Some(header)
}

/// Effective severity of each `SC-PRV*` rule, resolved once from the
/// [`scythe_lint::provenance_registry`] that `run_check` builds and applies
/// `[lint]` from `scythe.toml` to.
///
/// Provenance findings are produced outside the `LintRule::check_*` path --
/// there is no `LintContext` for "a file on disk" -- so they cannot pick up
/// their severity the way a SQL rule does, by being iterated out of
/// [`scythe_lint::RuleRegistry::active_rules`]. That is also why they are
/// kept out of `default_registry`: `scythe lint` and `scythe audit
/// --list-rules` would otherwise advertise nine rules neither can emit.
/// Resolving them here through
/// [`scythe_lint::RuleRegistry::effective_severity`] -- the same call every
/// SQL rule's severity goes through, against a registry the same `[lint]`
/// table was applied to -- is what keeps them configurable anyway:
/// `[lint.rules]` and `[lint.categories]` reach them exactly like any other
/// rule. Hardcoding these severities at the comparison sites -- as this
/// struct's consumer used to -- left schema drift as the one finding in
/// scythe with no configured way to downgrade or disable it, and therefore
/// no opt-out from failing CI on it.
///
/// Snapshotted into a `Copy` struct rather than holding a `&RuleRegistry` so
/// the registry does not have to outlive the whole check run just to answer
/// nine fixed questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProvenanceSeverities {
    schema_drift: Severity,
    version_drift: Severity,
    backend_drift: Severity,
    engine_drift: Severity,
    missing_header: Severity,
    malformed_header: Severity,
    unverifiable: Severity,
    query_drift: Severity,
    options_drift: Severity,
}

impl ProvenanceSeverities {
    /// Resolve every provenance rule's severity against `registry`.
    ///
    /// [`scythe_lint::RuleRegistry::effective_severity`] is total -- it falls
    /// back to the rule's own `default_severity()` when neither a per-rule
    /// nor a per-category override applies -- so this cannot fail and needs
    /// no defaults of its own. That matters: a second copy of the defaults
    /// here could disagree with `rules::provenance` without anything
    /// noticing.
    fn from_registry(registry: &RuleRegistry) -> Self {
        use scythe_lint::rules::provenance as rules;

        Self {
            schema_drift: registry.effective_severity(&rules::SchemaDrift),
            version_drift: registry.effective_severity(&rules::ScytheVersionDrift),
            backend_drift: registry.effective_severity(&rules::BackendDrift),
            engine_drift: registry.effective_severity(&rules::EngineDrift),
            missing_header: registry.effective_severity(&rules::MissingProvenanceHeader),
            malformed_header: registry.effective_severity(&rules::MalformedProvenanceHeader),
            unverifiable: registry.effective_severity(&rules::UnverifiableProvenance),
            query_drift: registry.effective_severity(&rules::QueryDrift),
            options_drift: registry.effective_severity(&rules::OptionsDrift),
        }
    }
}

/// What a single on-disk artifact's provenance header is checked against.
///
/// Bundled rather than passed as loose arguments because every field is
/// fixed for a whole `[[sql]]` block (or, for `backend_name`, for a whole
/// target) while [`verify_artifact`] is called once per artifact file -- a
/// Ruby target has two.
struct ProvenanceExpectation<'a> {
    /// `"<block name>:<configured backend>"`, used as the finding's
    /// `query_name`. Deliberately the *configured* backend string, so a
    /// finding points back at the `scythe.toml` line the reader has to edit.
    target_label: &'a str,
    /// Canonical `backend.name()`, never the configured alias -- see the
    /// SC-PRV03 comparison below.
    backend_name: &'a str,
    /// Sanitized `[[sql]]` engine -- see the SC-PRV04 comparison below.
    engine: &'a str,
    schema: &'a str,
    /// Current `q1:<16 hex>` fingerprint of this block's analyzed query set
    /// (see [`scythe_core::analyzer::AnalyzedQuery::fingerprint_set`]),
    /// compared against the header's `queries` field for SC-PRV08 -- see
    /// that comparison in [`verify_artifact`] for why it only fires when the
    /// header actually has one.
    queries: &'a str,
    /// Current `opt1:<16 hex>` fingerprint of this *target's* resolved
    /// `[[sql.gen]]` options and manifest overlay contents (see
    /// [`resolve_options_fingerprint`]), compared against the header's
    /// `options` field for SC-PRV11 -- see that comparison in
    /// [`verify_artifact`] for why it only fires when the header actually
    /// has one. Per-target, unlike `engine`/`schema`/`queries`: two targets
    /// under one `[[sql]]` block can configure different options.
    options: &'a str,
    version: &'a str,
    severities: ProvenanceSeverities,
}

/// Compare each of `sql_config`'s resolved generation targets' on-disk
/// artifacts against `catalog`, the current analyzed query set, the current
/// resolved `[[sql.gen]]` options, and the current scythe build, reporting
/// provenance drift as `SC-PRV01`-`SC-PRV08` and `SC-PRV11` violations.
///
/// # Scope
///
/// This answers "was this file generated from the *current schema*?", as of
/// #94 "was this file generated from the *current queries*?", and as of GH #155
/// / GH #155 "was this file generated with the *current options* (and
/// manifest overlay)?" -- nothing more. Each of the latter two questions
/// (SC-PRV08, SC-PRV11) is answered only when the artifact's header actually
/// carries the corresponding field: a header written before that field
/// existed predates it, and that reads as "not covered for this artifact",
/// not as drift -- see [`verify_artifact`]'s SC-PRV08 and SC-PRV11 checks.
/// Neither question is a substitute for actually regenerating: where
/// a full toolchain is available, `scythe generate` followed by `git status`
/// answers both by actually regenerating and diffing byte-for-byte;
/// provenance verification exists for the CI/review path where running the
/// full generator on every check is too expensive or the toolchain (database
/// drivers, per-language compilers) is unavailable.
///
/// # This function cannot fail
///
/// It returns a plain `Vec`, not a `Result`, and that is a deliberate
/// structural guarantee rather than a stylistic choice. `run_check`
/// accumulates findings across every `[[sql]]` block and only emits them --
/// as SARIF, JSON, or human text -- after the last block. Any `?` in here
/// therefore unwinds straight out of `run_check` past `emit_findings`, so a
/// single unconstructable backend or unreadable file would hand a CI
/// consumer an *empty* report plus exit 1 and silently discard every lint
/// finding from every block checked before it. Provenance verification is
/// the newest and least essential thing `check` does; it must never be able
/// to take the rest of the run down with it.
///
/// The two failures that used to propagate are now findings instead:
///
/// - A target whose backend/engine pair does not construct (a typo'd or
///   removed backend name, or an engine that backend does not support --
///   note that a config with no `[[sql.gen]]` block at all synthesizes a
///   `rust-sqlx` target, which does not support every engine `check` accepts).
///   A target that cannot be constructed is a target that cannot be verified,
///   so it is reported as SC-PRV07 and skipped. `scythe generate` still
///   reports the same misconfiguration as a hard error, which is the right
///   place for it.
/// - Any read failure other than "the file does not exist": a non-UTF-8
///   artifact, a directory sitting where the artifact should be, a
///   permissions problem. Same reasoning -- unverifiable, not drifted.
///
/// # Missing artifacts
///
/// A target whose output file does not exist is skipped without any finding
/// at all, not even SC-PRV07. The `.gitignore` a scythe project ships by
/// default excludes `**/generated/`, so an absent artifact in a fresh
/// checkout is the normal case, not a problem worth reporting.
///
/// # Severities
///
/// Every finding's severity comes from `severities`, which `run_check`
/// resolved from [`scythe_lint::provenance_registry`] after applying the
/// same `[lint]` table it applies to the SQL rules' registry. Nothing here
/// is hardcoded. See
/// [`ProvenanceSeverities`] for why that matters, and
/// [`scythe_lint::rules::provenance::ScytheVersionDrift`] for why SC-PRV02
/// nevertheless *defaults* to `Warn`. A rule configured `off` still produces
/// an entry here; `run_check` drops `Severity::Off` findings before
/// reporting, exactly as it does for SQL rules.
fn verify_provenance(
    sql_config: &SqlConfig,
    catalog: &Catalog,
    current_queries: &str,
    base_dir: &Path,
    severities: ProvenanceSeverities,
) -> Vec<QueryViolation> {
    let current_schema = catalog.fingerprint();
    // Sanitized once, for the same reason `provenance_header_line` sanitizes
    // before embedding: the header can only ever hold the sanitized string,
    // so comparing it against a raw config value would permanently
    // false-flag SC-PRV04 for any config whose engine string needed
    // sanitizing.
    let sanitized_engine = scythe_codegen::provenance::sanitize_field(&sql_config.engine);

    let mut violations = Vec::new();

    let targets = match resolve_gen_targets(sql_config) {
        Ok(targets) => targets,
        Err(e) => {
            violations.push(QueryViolation {
                query_name: sql_config.name.clone(),
                rule_id: Cow::Borrowed("SC-PRV07"),
                severity: severities.unverifiable,
                message: format!(
                    "cannot verify provenance: failed to resolve [sql.gen] targets: {e} (run `scythe generate` \
                     for the full diagnosis)"
                ),
            });
            return violations;
        }
    };

    for target in targets {
        let target_label = format!("{}:{}", sql_config.name, target.backend);

        let backend = match get_backend(&target.backend, &sql_config.engine) {
            Ok(backend) => backend,
            Err(e) => {
                violations.push(QueryViolation {
                    query_name: target_label,
                    rule_id: Cow::Borrowed("SC-PRV07"),
                    severity: severities.unverifiable,
                    message: format!(
                        "cannot verify provenance: backend '{}' with engine '{}' could not be \
                         constructed: {} (run `scythe generate` for the full diagnosis)",
                        target.backend, sql_config.engine, e
                    ),
                });
                continue;
            }
        };

        let options_fingerprint = match resolve_options_fingerprint(&target, base_dir) {
            Ok(fingerprint) => fingerprint,
            Err(e) => {
                violations.push(QueryViolation {
                    query_name: target_label,
                    rule_id: Cow::Borrowed("SC-PRV07"),
                    severity: severities.unverifiable,
                    message: format!("cannot verify provenance: {e} (run `scythe generate` for the full diagnosis)"),
                });
                continue;
            }
        };

        let expected = ProvenanceExpectation {
            target_label: &target_label,
            backend_name: backend.name(),
            engine: sanitized_engine.as_ref(),
            schema: &current_schema,
            queries: current_queries,
            options: &options_fingerprint,
            version: env!("CARGO_PKG_VERSION"),
            severities,
        };

        let output_dir = base_dir.join(&target.output);
        for artifact_path in artifact_paths(&*backend, &output_dir) {
            violations.extend(verify_artifact(&artifact_path, &expected));
        }
    }

    violations
}

/// Every file `generate_for_backend` writes for one target, in the order it
/// writes them.
///
/// Derived from the same two predicates generation uses --
/// [`output_filename`] and [`backend_emits_rbs`] -- so verification can
/// never end up looking at a different set of files than generation
/// produces. Ruby targets contribute a second entry: `queries.rbs` is a
/// tracked artifact whose signatures change with the schema, so leaving it
/// out of verification left it able to go stale with no drift signal at all.
fn artifact_paths(backend: &dyn CodegenBackend, output_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut paths = vec![output_dir.join(output_filename(backend))];
    if backend_emits_rbs(backend) {
        paths.push(output_dir.join(RBS_FILENAME));
    }
    paths
}

/// Check one generated file's provenance header against `expected`.
///
/// Returns no findings at all when the file does not exist -- see
/// [`verify_provenance`]'s "Missing artifacts" section.
fn verify_artifact(artifact_path: &Path, expected: &ProvenanceExpectation<'_>) -> Vec<QueryViolation> {
    let mut violations = Vec::new();
    let path_display = artifact_path.display().to_string();

    let content = match std::fs::read_to_string(artifact_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return violations,
        Err(e) => {
            violations.push(QueryViolation {
                query_name: expected.target_label.to_string(),
                rule_id: Cow::Borrowed("SC-PRV07"),
                severity: expected.severities.unverifiable,
                message: format!("{path_display}: cannot verify provenance: {e}"),
            });
            return violations;
        }
    };

    let Some(header) = parse_provenance_header(&content) else {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV05"),
            severity: expected.severities.missing_header,
            message: format!(
                "{path_display}: no provenance header found (predates provenance tracking, or is not scythe-managed)"
            ),
        });
        return violations;
    };

    if !header.is_complete() {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV06"),
            severity: expected.severities.malformed_header,
            message: format!(
                "{path_display}: provenance header is missing field(s): {}",
                header.missing_fields().join(", ")
            ),
        });
        return violations;
    }

    // ~keep Safe: `is_complete()` above just confirmed every field is `Some`.
    let header_schema = header.schema.as_deref().unwrap();
    let header_version = header.version.as_deref().unwrap();
    let header_backend = header.backend.as_deref().unwrap();
    let header_engine = header.engine.as_deref().unwrap();

    if header_schema != expected.schema {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV01"),
            severity: expected.severities.schema_drift,
            message: format!(
                "{path_display}: schema drift -- generated against schema {header_schema}, \
                 current schema is {} (run `scythe generate` to refresh)",
                expected.schema
            ),
        });
    }

    // SC-PRV08 (#94) -- a distinct finding from SC-PRV01 above: this compares
    // the *query* fingerprint, not the schema fingerprint, so editing a
    // `.sql` query file without touching the schema is no longer invisible
    // to `scythe check`.
    //
    // ~keep Deliberately gated on `header.queries` being present, unlike every
    // other field compared here: those four are covered by
    // `header.is_complete()` returning early above, but `queries` is not
    // part of that four-field completeness contract (see
    // [`ProvenanceHeader`]'s doc comment) precisely so that a header written
    // by scythe 0.14.0 or earlier -- which has no `queries=` field at all --
    // is treated as "query drift is not covered for this artifact" rather
    // than as either SC-PRV06 malformed-header or SC-PRV08 drift. Only a
    // header that *has* a `queries=` field and disagrees with the current
    // fingerprint fires this rule.
    if let Some(header_queries) = header.queries.as_deref()
        && header_queries != expected.queries
    {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV08"),
            severity: expected.severities.query_drift,
            message: format!(
                "{path_display}: query drift -- generated against queries {header_queries}, \
                 current queries are {} (run `scythe generate` to refresh)",
                expected.queries
            ),
        });
    }

    // SC-PRV11 (GH #155) -- a distinct finding from SC-PRV01/03/04
    // above: this compares the *options* fingerprint (derive list, serde
    // flag, `row_type`, naming case, manifest overlay contents, ...), not
    // schema/backend/engine, so changing an option no longer leaves the
    // header byte-identical to a target whose config never changed.
    //
    // ~keep Gated on `header.options` being present, exactly like the SC-PRV08
    // gate on `header.queries` immediately above and for the identical
    // reason: `options` is not part of the four-field completeness contract
    // (see [`ProvenanceHeader`]'s doc comment), so a header written before
    // this field existed reads as "options drift is not covered for this
    // artifact" rather than as SC-PRV06 malformed-header or SC-PRV11 drift.
    if let Some(header_options) = header.options.as_deref()
        && header_options != expected.options
    {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV11"),
            severity: expected.severities.options_drift,
            message: format!(
                "{path_display}: options drift -- generated with options {header_options}, current \
                 [[sql.gen]] options are {} (run `scythe generate` to refresh)",
                expected.options
            ),
        });
    }

    // ~keep Compared against `backend.name()` (the canonical form assembly
    // embeds), not the configured `backend = "..."` alias. `get_backend`
    // accepts several aliases per backend (e.g. `"sqlx"` or `"rb"`) and
    // every one of them constructs a backend whose `name()` returns the same
    // canonical string -- so comparing against the raw config alias would
    // flag every config using an alias as permanent, unfixable drift.
    if header_backend != expected.backend_name {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV03"),
            severity: expected.severities.backend_drift,
            message: format!(
                "{path_display}: generated by backend '{header_backend}', but this target now \
                 configures backend '{}' (run `scythe generate` to refresh)",
                expected.backend_name
            ),
        });
    }

    // `expected.engine` is already the *sanitized* configured engine (see
    // `verify_provenance`), matching what the header can only ever hold.
    if header_engine != expected.engine {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV04"),
            severity: expected.severities.engine_drift,
            message: format!(
                "{path_display}: generated for engine '{header_engine}', but this target now \
                 configures engine '{}' (run `scythe generate` to refresh)",
                expected.engine
            ),
        });
    }

    if header_version != expected.version {
        violations.push(QueryViolation {
            query_name: expected.target_label.to_string(),
            rule_id: Cow::Borrowed("SC-PRV02"),
            severity: expected.severities.version_drift,
            message: format!(
                "{path_display}: generated by scythe {header_version}, this is scythe {} \
                 (consider running `scythe generate`)",
                expected.version
            ),
        });
    }

    violations
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

    /// Must fail before the fix: pre-#189 `generate` had no such check at all, so a
    /// `column` override naming a table or column that exists nowhere in the schema
    /// (a typo) silently generated code with the override never applied, exit 0.
    #[test]
    fn check_type_overrides_resolve_rejects_column_matching_nothing() {
        use scythe_core::analyzer::AnalyzedColumn;

        let query = AnalyzedQuery::build(|q| {
            q.name = "GetUser".to_string();
            q.columns = vec![AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                source_relation: Some("users".to_string()),
                ..Default::default()
            }];
        });
        let overrides = vec![TypeOverride {
            column: Some("users.emial".to_string()), // deliberate typo
            db_type: None,
            neutral_type: Some("string".to_string()),
        }];

        let err = check_type_overrides_resolve("main", &[query], &overrides)
            .expect_err("a column override naming no real column must be rejected");
        let message = err.to_string();
        assert!(message.contains("users.emial"), "{message}");
        assert!(message.contains("main"), "{message}");
    }

    /// A column override that does name a real, resolved column must not be flagged --
    /// the companion case to the rejection above, pinning that the check only fires on a
    /// genuine miss.
    #[test]
    fn check_type_overrides_resolve_passes_when_column_matches() {
        use scythe_core::analyzer::AnalyzedColumn;

        let query = AnalyzedQuery::build(|q| {
            q.name = "GetUser".to_string();
            q.columns = vec![AnalyzedColumn {
                name: "email".to_string(),
                neutral_type: "string".to_string(),
                source_relation: Some("users".to_string()),
                ..Default::default()
            }];
        });
        let overrides = vec![TypeOverride {
            column: Some("users.email".to_string()),
            db_type: None,
            neutral_type: Some("json".to_string()),
        }];

        check_type_overrides_resolve("main", &[query], &overrides).expect("users.email is a real column");
    }

    /// A `:grouped` query's child columns are never in `AnalyzedQuery::columns` -- they
    /// live in `group_by.child_columns` -- so a `column` override targeting one must be
    /// checked against that split too, or every such override would be reported as a false
    /// positive.
    #[test]
    fn check_type_overrides_resolve_matches_grouped_child_column() {
        use scythe_core::analyzer::{AnalyzedColumn, GroupByConfig};

        let query = AnalyzedQuery::build(|q| {
            q.name = "GetUserWithOrders".to_string();
            q.group_by = Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: Vec::new(),
                child_columns: vec![AnalyzedColumn {
                    name: "total".to_string(),
                    neutral_type: "int32".to_string(),
                    source_relation: Some("orders".to_string()),
                    ..Default::default()
                }],
            });
        });
        let overrides = vec![TypeOverride {
            column: Some("orders.total".to_string()),
            db_type: None,
            neutral_type: Some("string".to_string()),
        }];

        check_type_overrides_resolve("main", &[query], &overrides).expect("orders.total is a real child column");
    }

    /// A `db_type`-only override has no single column to miss, so it must never be
    /// reported by the `column`-targeted check even when it matches no schema column by
    /// name (it isn't naming one).
    #[test]
    fn check_type_overrides_resolve_ignores_db_type_only_overrides() {
        let query = AnalyzedQuery::build(|q| {
            q.name = "GetUser".to_string();
        });
        let overrides = vec![TypeOverride {
            column: None,
            db_type: Some("ltree".to_string()),
            neutral_type: Some("string".to_string()),
        }];

        check_type_overrides_resolve("main", &[query], &overrides).expect("db_type-only overrides are never flagged");
    }

    /// Minimal `[[sql]]` preamble shared by the config-parsing tests below --
    /// only `sql` varies between them.
    fn sql_block(sql: &str) -> String {
        format!(
            "[scythe]\nversion = \"1\"\n\n[[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
             schema = [\"sql/schema/*.sql\"]\nqueries = [\"sql/queries/*.sql\"]\n{sql}\n"
        )
    }

    /// #116: omitting `output` from a `[[sql.gen]]` entry must report the
    /// real cause -- the missing `output` field -- not serde's opaque
    /// "data did not match any variant of untagged enum GenTargets".
    #[test]
    fn missing_output_on_array_gen_target_names_output_in_the_error() {
        let toml = sql_block("[[sql.gen]]\nbackend = \"rust-sqlx\"\n");

        let err = toml::from_str::<ScytheConfig>(&toml).expect_err("missing `output` must fail to parse");
        let message = err.to_string();

        assert!(
            message.contains("output"),
            "error must name the missing `output` field, got: {message}"
        );
        assert!(
            !message.contains("GenTargets") && !message.contains("untagged enum"),
            "error must not leak the internal `GenTargets` type name, got: {message}"
        );
    }

    /// A multi-block config is the case where the rendered line number lies.
    /// Errors from `TryFrom` reach serde as `Error::custom`, which carries no
    /// span, and the first frame that backfills one is the top-level `sql` key
    /// -- so `TOML parse error at line N` points at the *first* `[[sql]]` block
    /// no matter which block failed. The block name in the message text is the
    /// only thing that tells the user where to look, so pin it.
    #[test]
    fn error_in_second_sql_block_names_that_block() {
        let toml = "\
[scythe]
version = \"1\"

[[sql]]
name = \"first\"
engine = \"postgresql\"
schema = [\"sql/schema.sql\"]
queries = [\"sql/q.sql\"]

[[sql.gen]]
backend = \"python-psycopg3\"
output = \"out1\"

[[sql]]
name = \"second\"
engine = \"postgresql\"
schema = [\"sql/schema.sql\"]
queries = [\"sql/q.sql\"]

[[sql.gen]]
backend = \"rust-sqlx\"
";

        let err = toml::from_str::<ScytheConfig>(toml).expect_err("missing `output` must fail to parse");
        let message = err.to_string();

        assert!(
            message.contains("second"),
            "error must name the failing block, since the line number points at the first one, got: {message}"
        );
        assert!(
            !message.contains("\"first\""),
            "error must not name the block that parsed cleanly, got: {message}"
        );
    }

    /// #116's actual root cause: a user sets `[[sql]].output` expecting it to
    /// apply to every `[[sql.gen]]` array target, forgetting that field is
    /// only honoured by the legacy `[sql.gen.<lang>]` table form. The error
    /// must name that mistake directly instead of a bare "missing field".
    #[test]
    fn output_on_sql_block_plus_array_gen_missing_output_is_diagnosed_specifically() {
        let toml = sql_block("output = \"src/generated\"\n\n[[sql.gen]]\nbackend = \"rust-sqlx\"\n");

        let err = toml::from_str::<ScytheConfig>(&toml).expect_err("missing `output` must fail to parse");
        let message = err.to_string();

        assert!(
            message.contains("[[sql]].output") && message.contains("src/generated"),
            "error must point at the top-level `[[sql]].output` value that does not apply here, got: {message}"
        );
        assert!(
            message.contains("legacy") && message.contains("[sql.gen.<lang>]"),
            "error must explain that `output` is only used by the legacy table form, got: {message}"
        );
    }

    /// Valid `[[sql.gen]]` array-form configs -- with and without a sibling
    /// `[[sql]].output` -- must keep parsing.
    #[test]
    fn valid_array_gen_config_parses() {
        let toml = sql_block("[[sql.gen]]\nbackend = \"rust-sqlx\"\noutput = \"src/generated\"\n");
        let config: ScytheConfig = toml::from_str(&toml).expect("valid array-form config must parse");
        match &config.sql[0].gen_config {
            Some(GenTargets::Array(targets)) => {
                assert_eq!(targets.len(), 1);
                assert_eq!(targets[0].backend, "rust-sqlx");
                assert_eq!(targets[0].output, "src/generated");
            }
            other => panic!("expected GenTargets::Array, got {other:?}"),
        }
    }

    /// Valid legacy `[sql.gen.<lang>]` table-form configs must keep parsing,
    /// including when paired with `[[sql]].output` -- the one combination
    /// where that field is actually used.
    #[test]
    fn valid_legacy_gen_config_parses() {
        let toml = sql_block("output = \"src/generated\"\n\n[sql.gen.rust]\ntarget = \"sqlx\"\n");
        let config: ScytheConfig = toml::from_str(&toml).expect("valid legacy config must parse");
        assert_eq!(config.sql[0].output.as_deref(), Some("src/generated"));
        match &config.sql[0].gen_config {
            Some(GenTargets::Legacy(legacy)) => {
                assert_eq!(legacy.rust.as_ref().unwrap().target, "sqlx");
            }
            other => panic!("expected GenTargets::Legacy, got {other:?}"),
        }
    }

    /// A `[[sql]]` block with no `gen` key at all (relying entirely on the
    /// legacy `output` default) must still parse.
    #[test]
    fn config_with_no_gen_key_parses() {
        let toml = sql_block("output = \"src/generated\"\n");
        let config: ScytheConfig = toml::from_str(&toml).expect("config with no `gen` key must parse");
        assert!(config.sql[0].gen_config.is_none());
    }

    /// `RawSqlConfig`'s `#[serde(deny_unknown_fields)]` must catch a typo'd
    /// `[[sql]]` key -- this is the narrow, safe application of
    /// `deny_unknown_fields` described on `RawSqlConfig`'s doc comment; it is
    /// deliberately not applied to `ScytheConfig` or to the separate
    /// `SqlConfig` copies in `audit.rs`/`lint_cmd.rs`/`fmt.rs`.
    #[test]
    fn unknown_field_on_sql_block_is_rejected() {
        let toml = sql_block("outptu = \"src/generated\"\n");
        let err = toml::from_str::<ScytheConfig>(&toml).expect_err("typo'd key must fail to parse");
        assert!(
            err.to_string().contains("unknown field"),
            "expected an unknown-field error, got: {err}"
        );
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
            schema: scythe_inspect::SchemaDescription::new(),
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
            code: scythe_codegen::GeneratedCode::build(|c| {
                c.model_struct = model_struct.map(str::to_string);
                c.row_struct = row_struct.map(str::to_string);
                c.query_fn = query_fn.map(str::to_string);
            }),
            enums: Vec::new(),
            nested_enum_names: Vec::new(),
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

    /// board #166: two queries whose `model_struct` renders the identical text --
    /// exactly the shape two queries selecting the same composite column
    /// produce, since `generate_composite_def` is a pure function of the
    /// composite's shape, not of which query asked for it -- must not be
    /// emitted twice. `enum_def` and `nested_struct_defs` above are already
    /// deduped by name; before this fix `model_struct` was not deduped at
    /// all.
    ///
    /// Before the fix, `assemble_body` pushed every result's `model_struct`
    /// unconditionally, so this printed the declaration **twice**, back to
    /// back -- the literal cause of javac's `record Address is already
    /// defined` and kotlinc's `redeclaration:`. See
    /// `assemble_body_dedupes_composite_declared_by_two_queries_across_all_ten_languages`
    /// below for the same reproduction driven through real composite
    /// generation, across every shipped language, rather than a hand-typed
    /// stand-in for just one of them.
    #[test]
    fn assemble_body_dedupes_identical_model_struct_declared_by_two_queries() {
        let backend = get_backend("java-jdbc", "postgresql").expect("java-jdbc should support postgresql");
        let composite_decl = "public record Address(String street) {}";

        let results = vec![
            query_result(Some(composite_decl), Some("ROW_A"), Some("FN_A")),
            query_result(Some(composite_decl), Some("ROW_B"), Some("FN_B")),
        ];

        let output = assemble_body(backend.as_ref(), &results);

        let occurrences = output.matches(composite_decl).count();
        assert_eq!(
            occurrences, 1,
            "the composite declaration must appear exactly once, got {occurrences}. Output:\n{output}"
        );
    }

    /// board #166, established rather than assumed, and not scoped to the JVM:
    /// the defect lives entirely in this CLI's `assemble_body` -- no
    /// backend-specific branch decides whether `model_struct` gets deduped
    /// -- so it is language-agnostic. Every one of the ten backends below
    /// independently renders the identical composite declaration for two
    /// queries that select the same composite column, and before the fix
    /// every one of them printed it twice.
    ///
    /// Whether that duplication actually *breaks compilation* does vary by
    /// language -- established here by inspecting each backend's
    /// `generate_composite_def` source, not by running a compiler (this
    /// crate has none available, so that half stays unverified):
    ///
    /// - Java, Kotlin, Rust, Go, C#, and PHP each declare the composite as a
    ///   named `record`/`data class`/`struct`/`type ... struct`/`record`/
    ///   `class`, and each of those languages rejects a redeclaration as a
    ///   hard compile error (`record ... is already defined`,
    ///   `redeclaration:`, Rust's E0428 duplicate-definition, Go's
    ///   `... redeclared in this block`, C#'s CS0101, PHP's fatal
    ///   `Cannot redeclare class`).
    /// - TypeScript's composite is an `interface`, which the language
    ///   *merges* rather than rejects when redeclared with an identical
    ///   shape -- so this duplication would not have broken a `tsc` build,
    ///   only bloated the output.
    /// - Python's and Ruby's are a redefinition (a `class` statement, or a
    ///   `Data.define(...)` constant reassignment) that silently shadows the
    ///   first at runtime rather than raising a syntax error.
    /// - Elixir's is a `defmodule` redefinition that only warns
    ///   (`redefining module ...`) under the compiler's default settings.
    ///
    /// All ten still emit dead, duplicated code before this fix; only six of
    /// them would have refused to build because of it.
    #[test]
    fn assemble_body_dedupes_composite_declared_by_two_queries_across_all_ten_languages() {
        use scythe_core::analyzer::{AnalyzedColumn, CompositeFieldInfo, CompositeInfo};
        use scythe_core::parser::QueryCommand;

        let composite = CompositeInfo {
            sql_name: "address".to_string(),
            fields: vec![CompositeFieldInfo {
                name: "street".to_string(),
                neutral_type: "string".to_string(),
            }],
        };

        let make_query = |name: &str| {
            AnalyzedQuery::build(|q| {
                q.name = name.to_string();
                q.command = QueryCommand::One;
                q.columns = vec![AnalyzedColumn {
                    name: "addr".to_string(),
                    neutral_type: "composite::address".to_string(),
                    nullable: false,
                    ..Default::default()
                }];
                q.composites = vec![composite.clone()];
            })
        };

        // (backend, the substring that opens the composite's declaration)
        let cases: &[(&str, &str)] = &[
            ("rust-sqlx", "pub struct Address {"),
            ("python-psycopg3", "class Address:"),
            ("typescript-pg", "export interface Address {"),
            ("go-pgx", "type Address struct {"),
            // ~keep Opening brace only, like every other entry here. This used to match the
            // whole `{}` because a java-jdbc composite record had an empty body; it now carries
            // the `fromText` parser board #196 added, so a closed-body marker would pin the
            // record to having no members.
            ("java-jdbc", "public record Address(String street) {"),
            ("kotlin-exposed", "data class Address("),
            ("ruby-pg", "Address = Data.define("),
            ("php-pdo", "readonly class Address {"),
            ("csharp-npgsql", "public record Address("),
            ("elixir-postgrex", "defmodule Address do"),
        ];

        for &(backend_name, marker) in cases {
            let backend = get_backend(backend_name, "postgresql").unwrap_or_else(|e| panic!("{backend_name}: {e}"));

            let code_a = scythe_codegen::generate_with_backend(&make_query("GetOneAddress"), backend.as_ref())
                .unwrap_or_else(|e| panic!("{backend_name}: query_a failed to generate: {e}"));
            let code_b = scythe_codegen::generate_with_backend(&make_query("GetAnotherAddress"), backend.as_ref())
                .unwrap_or_else(|e| panic!("{backend_name}: query_b failed to generate: {e}"));

            assert!(
                code_a.model_struct.as_deref().is_some_and(|s| s.contains(marker)),
                "{backend_name}: test premise, query_a must render the composite declaration; got: {:?}",
                code_a.model_struct
            );

            let results = vec![
                QueryResult {
                    code: code_a,
                    enums: Vec::new(),
                    nested_enum_names: Vec::new(),
                },
                QueryResult {
                    code: code_b,
                    enums: Vec::new(),
                    nested_enum_names: Vec::new(),
                },
            ];

            let output = assemble_body(backend.as_ref(), &results);
            let occurrences = output.matches(marker).count();
            assert_eq!(
                occurrences, 1,
                "{backend_name}: the composite declaration must be emitted exactly once when two queries \
                 select it; before the board #166 fix this printed 2 (one per query). Output:\n{output}"
            );
        }
    }

    /// board #200: two queries whose row struct names collapse onto the same
    /// identifier under `struct_case`, but whose columns differ, must be
    /// rejected before assembly. `resolve::check_type_name_collisions` only
    /// ever compares a query's enums against that same query's own row/model
    /// type -- it never sees a second query's row struct at all, so before
    /// this check the file would have declared `GetUserRow` twice with
    /// different fields (`E0428` and its equivalent in every other target).
    #[test]
    fn check_file_level_type_name_collisions_rejects_row_structs_with_different_bodies() {
        use scythe_core::analyzer::AnalyzedColumn;

        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");

        let make_query = |name: &str, column: &str| {
            AnalyzedQuery::build(|q| {
                q.name = name.to_string();
                q.command = QueryCommand::One;
                q.columns = vec![AnalyzedColumn {
                    name: column.to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                }];
            })
        };

        // "get_user" and "GetUser" are both valid, distinct @name values that
        // `row_struct_name` case-converts to the identical "GetUserRow".
        let query_a = make_query("get_user", "id");
        let query_b = make_query("GetUser", "name");

        let code_a = scythe_codegen::generate_with_backend(&query_a, backend.as_ref()).expect("query_a generates");
        let code_b = scythe_codegen::generate_with_backend(&query_b, backend.as_ref()).expect("query_b generates");

        let analyzed = vec![query_a, query_b];
        let results = vec![query_result_with_code(code_a), query_result_with_code(code_b)];

        let err = check_file_level_type_name_collisions(&analyzed, &results, backend.as_ref())
            .expect_err("get_user and GetUser both derive GetUserRow with different columns");
        let message = err.to_string();
        assert!(message.contains("GetUserRow"), "{message}");
        assert!(message.contains("get_user"), "{message}");
        assert!(message.contains("GetUser"), "{message}");
    }

    /// The identical-name counterpart of the test above: when both queries'
    /// row structs render byte-identical text under the same name, that is
    /// the intended dedup (the same shape `assemble_body` already grants a
    /// shared table model or composite), not a collision, and must pass.
    #[test]
    fn check_file_level_type_name_collisions_allows_identical_row_structs() {
        use scythe_core::analyzer::AnalyzedColumn;

        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");

        let make_query = |name: &str| {
            AnalyzedQuery::build(|q| {
                q.name = name.to_string();
                q.command = QueryCommand::One;
                q.columns = vec![AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                }];
            })
        };

        let query_a = make_query("get_user");
        let query_b = make_query("GetUser");

        let code_a = scythe_codegen::generate_with_backend(&query_a, backend.as_ref()).expect("query_a generates");
        let code_b = scythe_codegen::generate_with_backend(&query_b, backend.as_ref()).expect("query_b generates");
        assert_eq!(
            code_a.row_struct, code_b.row_struct,
            "test premise: identical columns under the same generated name must render identical text"
        );

        let analyzed = vec![query_a, query_b];
        let results = vec![query_result_with_code(code_a), query_result_with_code(code_b)];

        check_file_level_type_name_collisions(&analyzed, &results, backend.as_ref())
            .expect("identical row struct bodies under the same generated name must dedupe, not collide");
    }

    /// board #200's other shape: two *different* queries each declaring an
    /// enum whose SQL name case-converts to the same identifier, with
    /// different variants. `resolve::check_type_name_collisions` runs once
    /// per query and never compares across queries, so this reached
    /// `assemble_body`'s per-`sql_name` enum dedup unguarded -- which keys on
    /// the raw SQL name, not the generated one, so `order_status` and
    /// `order-status` were treated as two distinct enums and both emitted as
    /// `pub enum OrderStatus`.
    #[test]
    fn check_file_level_type_name_collisions_rejects_enums_with_different_bodies() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");

        let analyzed = vec![
            AnalyzedQuery::build(|q| {
                q.name = "GetOrder".to_string();
                q.command = QueryCommand::One;
            }),
            AnalyzedQuery::build(|q| {
                q.name = "GetOtherOrder".to_string();
                q.command = QueryCommand::One;
            }),
        ];

        let enum_a = EnumInfo {
            sql_name: "order_status".to_string(),
            values: vec!["pending".to_string(), "shipped".to_string()],
        };
        let enum_b = EnumInfo {
            sql_name: "order-status".to_string(),
            values: vec!["open".to_string(), "closed".to_string()],
        };

        let results = vec![
            QueryResult {
                code: scythe_codegen::GeneratedCode::default(),
                enums: vec![enum_a],
                nested_enum_names: Vec::new(),
            },
            QueryResult {
                code: scythe_codegen::GeneratedCode::default(),
                enums: vec![enum_b],
                nested_enum_names: Vec::new(),
            },
        ];

        let err = check_file_level_type_name_collisions(&analyzed, &results, backend.as_ref())
            .expect_err("order_status and order-status both derive OrderStatus with different variants");
        assert!(err.to_string().contains("OrderStatus"), "{err}");
    }

    fn query_result_with_code(code: scythe_codegen::GeneratedCode) -> QueryResult {
        QueryResult {
            code,
            enums: Vec::new(),
            nested_enum_names: Vec::new(),
        }
    }

    /// With no analyzed queries at all, there is nothing to write; the file
    /// must fall back to a placeholder comment rather than an empty string
    /// (an empty generated file reads as a build failure, not "no queries").
    #[test]
    fn assemble_body_falls_back_to_placeholder_when_results_are_empty() {
        // ~keep A hand-rolled backend (rather than a real one from `get_backend`)
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
    // Provenance header production (`provenance_header_line`,
    // `assemble_output`)
    //
    // The comment-prefix table, the field sanitizer, and the
    // preamble/header/body ordering they feed are unit-tested in
    // ~keep `scythe_codegen::provenance`, where they live. What is tested here is
    // this crate's use of them: that the embedded version is *this binary's*,
    // and that the round trip back through `parse_provenance_header` below
    // holds.
    // -----------------------------------------------------------------------

    #[test]
    fn provenance_header_line_contains_sentinel_and_all_six_fields() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = provenance_header_line(
            backend.as_ref(),
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
            "opt1:0011223344556677",
        );

        assert!(line.starts_with("// scythe:provenance "), "got: {line:?}");
        assert!(line.contains(&format!("v={}", env!("CARGO_PKG_VERSION"))));
        assert!(line.contains("backend=rust-sqlx"));
        assert!(line.contains("engine=postgresql"));
        assert!(line.contains("schema=sch1:0123456789abcdef"));
        assert!(line.contains("queries=q1:fedcba9876543210"));
        assert!(line.contains("options=opt1:0011223344556677"));
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn provenance_header_line_uses_hash_comment_for_ruby() {
        let backend = get_backend("ruby-pg", "postgresql").expect("ruby-pg should support postgresql");
        let line = provenance_header_line(
            backend.as_ref(),
            "postgresql",
            "sch1:aaaa",
            "q1:fedcba9876543210",
            "opt1:aaaa",
        );
        assert!(line.starts_with("# scythe:provenance "), "got: {line:?}");
    }

    /// Regression test for the header-injection defect: a raw newline in an
    /// `engine` value must never reach the embedded header, or it would
    /// terminate the comment early and turn everything after it into live,
    /// uncommented content in the generated file. It reaches the header
    /// escaped instead (`\u{a}`), along with the spaces that would
    /// otherwise split the field -- see
    /// `scythe_codegen::provenance::sanitize_field`. Asserts on the *line
    /// count* of the produced header, not a substring -- a substring check
    /// (e.g. `!line.contains("evil")`) would still pass on the broken,
    /// unescaped version, since the injected text is still present, just on
    /// its own line.
    #[test]
    fn provenance_header_line_sanitizes_newline_in_engine() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let malicious_engine = "postgresql\nfn evil() {}";

        let line = provenance_header_line(
            backend.as_ref(),
            malicious_engine,
            "sch1:ffff",
            "q1:fedcba9876543210",
            "opt1:ffff",
        );

        assert_eq!(
            line.lines().count(),
            1,
            "a sanitized header must be exactly one line, got: {line:?}"
        );
        assert_eq!(
            line,
            format!(
                "// scythe:provenance v={} backend=rust-sqlx \
                 engine=postgresql\\u{{a}}fn\\u{{20}}evil()\\u{{20}}{{}} schema=sch1:ffff \
                 queries=q1:fedcba9876543210 options=opt1:ffff\n",
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

        let output = assemble_output(
            backend.as_ref(),
            &[],
            malicious_engine,
            "sch1:ffff",
            "q1:fedcba9876543210",
            "opt1:ffff",
        );

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
        // escape itself. The space-bearing case is covered separately by
        // `parse_provenance_header_round_trips_engine_values_containing_spaces`.
        let malicious_engine = "postgresql\ndroptable";

        let output = assemble_output(
            backend.as_ref(),
            &[],
            malicious_engine,
            "sch1:ffff",
            "q1:fedcba9876543210",
            "opt1:ffff",
        );
        let header = parse_provenance_header(&output).expect("assembled output must carry a parseable header");

        let expected = scythe_codegen::provenance::sanitize_field(malicious_engine);
        assert_eq!(header.engine.as_deref(), Some(expected.as_ref()));
    }

    /// Regression test for #133. `parse_provenance_header` tokenizes the
    /// header tail with `split_ascii_whitespace()`, so an `engine` value
    /// carrying an internal space used to be silently truncated: everything
    /// from the first space onward had no `=`, failed `split_once('=')`,
    /// and was dropped, leaving `header.engine` as the bare prefix
    /// `"postgresqlfn"`. That is not a comment-injection risk (the line
    /// stayed one line, stayed a comment) but it is a correctness gap --
    /// `verify_provenance`'s SC-PRV04 check compares `header.engine`
    /// against the configured engine, and a truncated value could
    /// spuriously match or spuriously fail depending on what the surviving
    /// prefix collided with.
    ///
    /// `scythe_codegen::provenance::sanitize_field` now escapes whitespace
    /// rather than only stripping `\n`/`\r`, so the embedded value carries
    /// no token separator and the field survives the tokenizer whole.
    /// SC-PRV04 compares encoded-to-encoded (`verify_provenance` runs the
    /// configured engine through the same function), which is equivalent to
    /// comparing the decoded values because the encoding is injective.
    #[test]
    fn parse_provenance_header_round_trips_engine_values_containing_spaces() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let malicious_engine = "postgresql\nfn evil() {}";

        let output = assemble_output(
            backend.as_ref(),
            &[],
            malicious_engine,
            "sch1:ffff",
            "q1:fedcba9876543210",
            "opt1:ffff",
        );
        let header = parse_provenance_header(&output).expect("assembled output must carry a parseable header");

        let encoded = scythe_codegen::provenance::sanitize_field(malicious_engine);
        assert_eq!(header.engine.as_deref(), Some(encoded.as_ref()), "no truncation");
        assert_eq!(
            scythe_codegen::provenance::decode_field(header.engine.as_deref().expect("engine field is present")),
            malicious_engine,
            "the decoded field must equal what the caller passed in"
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

        let line = provenance_header_line(
            alias_backend.as_ref(),
            "postgresql",
            "sch1:aaaa",
            "q1:fedcba9876543210",
            "opt1:aaaa",
        );
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

        let output = assemble_output(
            backend.as_ref(),
            &results,
            "postgresql",
            "sch1:fedcba9876543210",
            "q1:0123456789abcdef",
            "opt1:0123456789abcdef",
        );

        let header = parse_provenance_header(&output).expect("assembled output must carry a parseable header");
        assert_eq!(header.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(header.backend.as_deref(), Some("rust-sqlx"));
        assert_eq!(header.engine.as_deref(), Some("postgresql"));
        assert_eq!(header.schema.as_deref(), Some("sch1:fedcba9876543210"));
        assert_eq!(header.queries.as_deref(), Some("q1:0123456789abcdef"));
        assert_eq!(header.options.as_deref(), Some("opt1:0123456789abcdef"));
    }

    /// `<?php` must be the literal first five bytes of the assembled file --
    /// not merely present somewhere in it. A provenance comment (or
    /// anything else) landing above it would silently degrade the file to
    /// HTML output in a PHP interpreter.
    #[test]
    fn assemble_output_keeps_php_open_tag_as_the_first_bytes() {
        for backend_name in ["php-pdo", "php-amphp"] {
            let backend = get_backend(backend_name, "postgresql").unwrap_or_else(|e| panic!("{backend_name}: {e}"));
            let output = assemble_output(
                backend.as_ref(),
                &[],
                "postgresql",
                "sch1:aaaa",
                "q1:fedcba9876543210",
                "opt1:aaaa",
            );
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
            let output = assemble_output(
                backend.as_ref(),
                &[],
                "postgresql",
                "sch1:bbbb",
                "q1:fedcba9876543210",
                "opt1:bbbb",
            );
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

    /// Python's header carries a `# noqa: E501` suppression (see
    /// `scythe_codegen::provenance::header_suffix`), and that suffix must be
    /// invisible to the verifier. `parse_provenance_header` tokenizes the
    /// text after the sentinel on whitespace and skips tokens without an
    /// `=`, so `#`, `noqa:` and `E501` are dropped -- but nothing else pins
    /// that the two sides agree, and a parser change that started treating
    /// unrecognized tokens as errors (or a suffix that grew a `key=value`
    /// shape) would break every Python target's drift detection silently.
    ///
    /// Doubles as the regression test for the suffix itself: without it,
    /// every generated `.py` file opens with a ~99-character line and
    /// `ruff check --select E` reports E501 on line 1.
    #[test]
    fn assemble_output_python_header_carries_noqa_and_still_round_trips() {
        let backend = get_backend("python-psycopg3", "postgresql").expect("python-psycopg3 should support postgresql");
        assert!(
            backend.file_preamble().is_empty(),
            "test assumes python-psycopg3 has no preamble, so the header is line 1"
        );

        let output = assemble_output(
            backend.as_ref(),
            &[],
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
            "opt1:0123456789abcdef",
        );

        let first_line = output.lines().next().expect("output must have a first line");
        assert!(first_line.starts_with("# scythe:provenance "), "got: {first_line:?}");
        assert!(
            first_line.ends_with("  # noqa: E501"),
            "a Python provenance header must suppress E501, or `ruff check --select E` \
             fails on line 1 of every generated file: {first_line:?}"
        );

        let header = parse_provenance_header(&output).expect("assembled output must carry a parseable header");
        assert!(
            header.is_complete(),
            "the noqa suffix must not cost the header a field: {header:?}"
        );
        assert_eq!(header.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(header.backend.as_deref(), Some("python-psycopg3"));
        assert_eq!(header.engine.as_deref(), Some("postgresql"));
        assert_eq!(header.schema.as_deref(), Some("sch1:0123456789abcdef"));
        assert_eq!(
            header.queries.as_deref(),
            Some("q1:fedcba9876543210"),
            "the last field before the suffix must not absorb any of it"
        );
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

        let output = assemble_output(
            backend.as_ref(),
            &[],
            "postgresql",
            "sch1:cccc",
            "q1:fedcba9876543210",
            "opt1:cccc",
        );
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

        let output = assemble_output(
            backend.as_ref(),
            &[],
            "postgresql",
            "sch1:dddd",
            "q1:fedcba9876543210",
            "opt1:dddd",
        );
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

        let output = assemble_output(
            backend.as_ref(),
            &[],
            "postgresql",
            "sch1:eeee",
            "q1:fedcba9876543210",
            "opt1:eeee",
        );
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

    /// #94: `parse_provenance_header` must read a `queries=` field when
    /// present.
    #[test]
    fn parse_provenance_header_reads_queries_field() {
        let content = "// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:eeee \
                        queries=q1:fedcba9876543210\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.queries.as_deref(), Some("q1:fedcba9876543210"));
    }

    /// #94: `queries` is not one of the four fields
    /// `ProvenanceHeader::is_complete` requires. A header from before #94 has
    /// no `queries=` field at all and must still verify as complete -- that
    /// is the backward-compatibility contract `verify_artifact`'s SC-PRV08
    /// gate depends on.
    #[test]
    fn parse_provenance_header_without_queries_field_is_still_complete() {
        let content = "// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:eeee\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.queries, None);
        assert!(
            header.is_complete(),
            "a pre-#94 header with no queries= field must still be complete: {header:?}"
        );
    }

    /// Round-trip through the real assembler: `header_line` writes the
    /// `queries=` field scythe generate embeds, and `parse_provenance_header`
    /// must read back the exact value handed to it -- the same contract
    /// already pinned for `schema` by
    /// `assemble_output_python_header_carries_noqa_and_still_round_trips`
    /// and friends.
    #[test]
    fn queries_field_round_trips_through_header_line_and_parse() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = scythe_codegen::provenance::header_line(
            backend.as_ref(),
            "0.15.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        );

        let header = parse_provenance_header(&line).expect("header must be found");
        assert!(header.is_complete());
        assert_eq!(header.queries.as_deref(), Some("q1:fedcba9876543210"));
        assert_eq!(header.schema.as_deref(), Some("sch1:0123456789abcdef"));
    }

    /// GH #155: `parse_provenance_header` must read an `options=`
    /// field when present.
    #[test]
    fn parse_provenance_header_reads_options_field() {
        let content = "// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:eeee \
                        queries=q1:fedcba9876543210 options=opt1:0011223344556677\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.options.as_deref(), Some("opt1:0011223344556677"));
    }

    /// `options` is not one of the four fields `ProvenanceHeader::is_complete`
    /// requires, exactly like `queries` above and for the identical
    /// backward-compatibility reason: a header written before GH #155 has no
    /// `options=` field at all and must still verify as complete -- that is
    /// the contract `verify_artifact`'s SC-PRV11 gate depends on.
    #[test]
    fn parse_provenance_header_without_options_field_is_still_complete() {
        let content = "// scythe:provenance v=0.13.0 backend=rust-sqlx engine=postgresql schema=sch1:eeee \
                        queries=q1:fedcba9876543210\n";
        let header = parse_provenance_header(content).expect("header must be found");
        assert_eq!(header.options, None);
        assert!(
            header.is_complete(),
            "a pre-#155 header with no options= field must still be complete: {header:?}"
        );
    }

    /// Round-trip through the real assembler: `header_line_with_options`
    /// writes the `options=` field scythe generate now embeds, and
    /// `parse_provenance_header` must read back the exact value handed to
    /// it -- the same contract already pinned for `queries` above.
    #[test]
    fn options_field_round_trips_through_header_line_with_options_and_parse() {
        let backend = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        let line = scythe_codegen::provenance::header_line_with_options(
            backend.as_ref(),
            "0.15.0",
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
            "opt1:0011223344556677",
        );

        let header = parse_provenance_header(&line).expect("header must be found");
        assert!(header.is_complete());
        assert_eq!(header.options.as_deref(), Some("opt1:0011223344556677"));
    }

    // -----------------------------------------------------------------------
    // End-to-end provenance verification (`verify_provenance`)
    // -----------------------------------------------------------------------

    fn provenance_test_catalog() -> Catalog {
        Catalog::from_ddl(&["CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);"]).unwrap()
    }

    /// Severities as an unconfigured project gets them: straight out of
    /// `provenance_registry()`, with no `[lint]` overrides applied. Resolved
    /// through the registry rather than hand-written here so that a change
    /// to a rule's `default_severity()` shows up in these tests instead of
    /// being masked by a second copy of the defaults.
    fn default_severities() -> ProvenanceSeverities {
        ProvenanceSeverities::from_registry(&scythe_lint::provenance_registry())
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
        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
        assert!(violations.is_empty(), "expected no violations, got {violations:?}");
    }

    /// #94: a header with no `queries=` field at all -- exactly what every
    /// artifact scythe 0.14.0 or earlier wrote -- must verify clean, even
    /// though the current queries fingerprint disagrees with... nothing,
    /// because there is nothing in the header to compare it against. This is
    /// the backward-compatibility contract `ProvenanceHeader::is_complete`
    /// and the `header.queries` gate in `verify_artifact` exist to guarantee:
    /// SC-PRV08 must never fire, and SC-PRV06 (malformed header) must not
    /// fire either, since `queries` is not part of that four-field
    /// completeness contract.
    #[test]
    fn verify_provenance_queries_less_header_verifies_clean() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        // No `queries=` field -- the pre-#94 header shape.
        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={}",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        // Deliberately does not match anything real -- if `queries` ever
        // leaked into a comparison it would have to fire, so a nonsense
        // value pins that it never does.
        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:does-not-matter",
            std::path::Path::new("."),
            default_severities(),
        );
        assert!(violations.is_empty(), "expected no violations, got {violations:?}");
    }

    /// A header that *does* carry `queries=` and matches the current
    /// fingerprint must produce no violations -- the companion positive case
    /// to `verify_provenance_detects_query_drift_as_error` below.
    #[test]
    fn verify_provenance_matching_queries_header_produces_no_violations() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={} queries=q1:fedcba9876543210",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:fedcba9876543210",
            std::path::Path::new("."),
            default_severities(),
        );
        assert!(violations.is_empty(), "expected no violations, got {violations:?}");
    }

    /// #94: editing a `.sql` query file without touching the schema must be
    /// visible to `scythe check` -- the whole point of SC-PRV08. Distinct
    /// from SC-PRV01: the schema fingerprint in the header still matches, so
    /// SC-PRV01 must *not* fire alongside it.
    #[test]
    fn verify_provenance_detects_query_drift_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={} queries=q1:0000000000000000",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:fedcba9876543210",
            std::path::Path::new("."),
            default_severities(),
        );
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV08");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Error);
    }

    /// GH #155: a header with no `options=` field at all -- exactly
    /// what every artifact written before this field existed carries -- must
    /// verify clean, mirroring `verify_provenance_queries_less_header_verifies_clean`'s
    /// backward-compatibility contract for `queries`. SC-PRV11 must never
    /// fire, and SC-PRV06 (malformed header) must not fire either, since
    /// `options` is not part of the four-field completeness contract.
    #[test]
    fn verify_provenance_options_less_header_verifies_clean() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        // No `options=` field -- the pre-#155 header shape.
        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={} queries=q1:0000000000000000",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
        assert!(violations.is_empty(), "expected no violations, got {violations:?}");
    }

    /// A header that *does* carry `options=` and matches the current
    /// target's resolved options must produce no violations -- the
    /// companion positive case to
    /// `verify_provenance_detects_options_drift_as_error` below.
    #[test]
    fn verify_provenance_matching_options_header_produces_no_violations() {
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
                backend: "rust-sqlx".to_string(),
                output: dir.path().to_string_lossy().into_owned(),
                manifest: None,
                options: std::collections::HashMap::from([(
                    "row_type".to_string(),
                    toml::Value::String("pydantic".to_string()),
                )]),
            }])),
            type_overrides: None,
        };

        // Computed through the real function, not hand-derived, so this test
        // cannot be wrong about what "current" actually is.
        let current_options = scythe_codegen::provenance::options_fingerprint([("row_type", "pydantic")], None);

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={} queries=q1:0000000000000000 options={}",
                env!("CARGO_PKG_VERSION"),
                schema,
                current_options
            ),
        );

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
        assert!(violations.is_empty(), "expected no violations, got {violations:?}");
    }

    /// GH #155: a header whose `options=` field disagrees with the
    /// current target's resolved options must be reported as SC-PRV11
    /// drift, mirroring `verify_provenance_detects_query_drift_as_error`'s
    /// shape for `queries`. The schema and query fingerprints both match, so
    /// SC-PRV01 and SC-PRV08 must not fire alongside it.
    #[test]
    fn verify_provenance_detects_options_drift_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let sql_config = provenance_test_sql_config(dir.path());

        // `provenance_test_sql_config` resolves to the default `rust-sqlx`
        // target with no `[[sql.gen]]` options at all. Computed through the
        // real function (not hand-derived) purely to prove the premise below
        // rather than to build the header -- the header intentionally
        // embeds a different, wrong value.
        let current_options = scythe_codegen::provenance::options_fingerprint(std::iter::empty(), None);
        assert_ne!(
            current_options, "opt1:0000000000000000",
            "test premise: the header below must actually disagree with the current fingerprint"
        );

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={} queries=q1:0000000000000000 \
                 options=opt1:0000000000000000",
                env!("CARGO_PKG_VERSION"),
                schema
            ),
        );

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV11");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Error);
    }

    /// GH #155: a target configured with a manifest overlay whose
    /// file *content* on disk no longer matches what the header's
    /// `options=` fingerprint was computed from must be reported as
    /// SC-PRV11 drift, even though `target.options` itself never changed --
    /// the overlay is fingerprinted by content, not merely by "configured or
    /// not" (see `scythe_codegen::provenance::options_fingerprint`'s doc
    /// comment).
    #[test]
    fn verify_provenance_detects_manifest_overlay_content_drift_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();

        let overlay_path = dir.path().join("overlay.toml");
        std::fs::write(&overlay_path, "[naming]\nstruct_case = \"PascalCase\"\n").unwrap();

        let sql_config = SqlConfig {
            name: "main".to_string(),
            engine: "postgresql".to_string(),
            schema: Vec::new(),
            queries: Vec::new(),
            output: None,
            gen_config: Some(GenTargets::Array(vec![GenTarget {
                backend: "rust-sqlx".to_string(),
                output: dir.path().to_string_lossy().into_owned(),
                manifest: Some("overlay.toml".to_string()),
                options: std::collections::HashMap::new(),
            }])),
            type_overrides: None,
        };

        // The header was written when the overlay held its *original*
        // content.
        let original_options = scythe_codegen::provenance::options_fingerprint(
            std::iter::empty(),
            Some("[naming]\nstruct_case = \"PascalCase\"\n"),
        );

        write_artifact(
            dir.path(),
            &format!(
                "v={} backend=rust-sqlx engine=postgresql schema={} queries=q1:0000000000000000 options={}",
                env!("CARGO_PKG_VERSION"),
                schema,
                original_options
            ),
        );

        // The overlay file is edited after the fact, with no regeneration --
        // `target.options` (empty) never changes.
        std::fs::write(&overlay_path, "[naming]\nstruct_case = \"snake_case\"\n").unwrap();

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            dir.path(),
            default_severities(),
        );
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV11");
    }

    /// The stability half of GH #155: calling
    /// `resolve_options_fingerprint` twice for the identical target and
    /// `base_dir` -- nothing regenerated, nothing edited -- must produce the
    /// identical fingerprint both times. A fingerprint that drifts on its
    /// own between two runs of `scythe check` against an unchanged project
    /// would report false drift on every single run, which is worse than
    /// not fingerprinting options at all.
    #[test]
    fn resolve_options_fingerprint_is_stable_when_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("overlay.toml"),
            "[naming]\nstruct_case = \"PascalCase\"\n",
        )
        .unwrap();

        let target = ResolvedGenTarget {
            backend: "rust-sqlx".to_string(),
            output: "generated".to_string(),
            manifest_override: Some("overlay.toml".to_string()),
            options: std::collections::HashMap::from([("row_type".to_string(), "pydantic".to_string())]),
        };

        let first = resolve_options_fingerprint(&target, dir.path()).expect("fingerprint must resolve");
        let second = resolve_options_fingerprint(&target, dir.path()).expect("fingerprint must resolve");

        assert_eq!(
            first, second,
            "regenerating with nothing changed must produce a byte-identical fingerprint"
        );
    }

    /// The companion sensitivity case: an option value changing must move
    /// the fingerprint `resolve_options_fingerprint` returns, not just the
    /// pure `options_fingerprint` it delegates to (already covered directly
    /// in `scythe_codegen::provenance`'s own tests).
    #[test]
    fn resolve_options_fingerprint_changes_when_an_option_value_changes() {
        let dir = tempfile::tempdir().unwrap();

        let make_target = |row_type: &str| ResolvedGenTarget {
            backend: "rust-sqlx".to_string(),
            output: "generated".to_string(),
            manifest_override: None,
            options: std::collections::HashMap::from([("row_type".to_string(), row_type.to_string())]),
        };

        let pydantic = resolve_options_fingerprint(&make_target("pydantic"), dir.path()).unwrap();
        let dataclass = resolve_options_fingerprint(&make_target("dataclass"), dir.path()).unwrap();

        assert_ne!(pydantic, dataclass);
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
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

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );
        let mut ids: Vec<&str> = violations.iter().map(|v| v.rule_id.as_ref()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["SC-PRV01", "SC-PRV03", "SC-PRV04"]);
    }

    // -----------------------------------------------------------------------
    // Targets that cannot be verified at all (SC-PRV07)
    // -----------------------------------------------------------------------

    /// The exact configuration that used to abort the whole run: `engine =
    /// "oracle"` with **no `[[sql.gen]]` block**, so `resolve_gen_targets`
    /// synthesizes a `rust-sqlx` target, and `SqlxBackend::new("oracle")`
    /// fails because `supported_engines` is postgresql/mysql/mariadb/sqlite/
    /// redshift. That failure used to `?`-propagate out of `run_check`
    /// before `emit_findings` ran, handing a SARIF or JSON consumer an empty
    /// report plus exit 1 and discarding every lint finding from every
    /// preceding `[[sql]]` block. It must now be an ordinary finding.
    #[test]
    fn verify_provenance_reports_unconstructable_backend_as_a_finding_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();

        let sql_config = SqlConfig {
            name: "main".to_string(),
            engine: "oracle".to_string(),
            schema: Vec::new(),
            queries: Vec::new(),
            output: Some(dir.path().to_string_lossy().into_owned()),
            gen_config: None,
            type_overrides: None,
        };

        // Guards the premise: if `rust-sqlx` ever gains Oracle support this
        // test stops exercising the path it was written for, and should be
        // repointed at another unsupported pair rather than quietly passing.
        assert!(
            get_backend("rust-sqlx", "oracle").is_err(),
            "test premise: rust-sqlx must not support oracle"
        );

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV07");
        assert_eq!(
            violations[0].severity,
            scythe_lint::Severity::Warn,
            "an unverifiable target must never be able to fail a check run on its own"
        );
    }

    /// A typo'd or removed `backend = "..."` is the same class of failure as
    /// the engine mismatch above, reached through an explicit `[[sql.gen]]`
    /// block rather than through the synthesized default target.
    #[test]
    fn verify_provenance_reports_unknown_backend_name_as_a_finding_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();

        let sql_config = SqlConfig {
            name: "main".to_string(),
            engine: "postgresql".to_string(),
            schema: Vec::new(),
            queries: Vec::new(),
            output: None,
            gen_config: Some(GenTargets::Array(vec![GenTarget {
                backend: "rust-sqlx-typo".to_string(),
                output: dir.path().to_string_lossy().into_owned(),
                manifest: None,
                options: std::collections::HashMap::new(),
            }])),
            type_overrides: None,
        };

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV07");
        assert!(
            violations[0].message.contains("rust-sqlx-typo"),
            "the finding must name the backend that could not be constructed: {}",
            violations[0].message
        );
    }

    /// An I/O failure that is not `NotFound` — here, a *directory* sitting
    /// where `generated/queries.rs` should be — must also degrade to a
    /// finding. The same applies to a non-UTF-8 artifact (`InvalidData`) and
    /// to a permissions failure; a directory is the one variant that can be
    /// provoked portably and without root.
    #[test]
    fn verify_provenance_reports_unreadable_artifact_as_a_finding_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let sql_config = provenance_test_sql_config(dir.path());

        std::fs::create_dir(dir.path().join("queries.rs")).unwrap();

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV07");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Warn);
    }

    /// A non-UTF-8 artifact reads as `InvalidData`, not `NotFound`, so it
    /// took the same aborting branch as the directory case above. Written as
    /// its own test because it is the variant a real project is most likely
    /// to hit: any generated file that a downstream tool re-encoded.
    #[test]
    fn verify_provenance_reports_non_utf8_artifact_as_a_finding_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let sql_config = provenance_test_sql_config(dir.path());

        // Lone 0x80 continuation byte: never valid UTF-8 in any position.
        std::fs::write(dir.path().join("queries.rs"), [0x80_u8]).unwrap();

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV07");
    }

    /// Provenance severities must come from the lint registry, so a project
    /// can turn schema drift off (or escalate a version mismatch) from
    /// `[lint]` in `scythe.toml` like any other rule. Before this, SC-PRV01,
    /// SC-PRV03 and SC-PRV04 were hardcoded `Error` with no opt-out from
    /// failing CI on drift.
    #[test]
    fn verify_provenance_severities_follow_the_lint_config() {
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

        // ~keep Built the way `run_check` builds it: `provenance_registry()` plus
        // the user's `[lint]` table, the same table applied to the default
        // registry.
        let mut registry = scythe_lint::provenance_registry();
        let mut lint_config = scythe_lint::LintConfig::default();
        lint_config
            .rules
            .insert("SC-PRV01".to_string(), scythe_lint::Severity::Warn);
        registry.apply_config(&lint_config);

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            ProvenanceSeverities::from_registry(&registry),
        );

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV01");
        assert_eq!(
            violations[0].severity,
            scythe_lint::Severity::Warn,
            "a downgraded SC-PRV01 must be reported at the configured severity"
        );
    }

    // -----------------------------------------------------------------------
    // Ruby's second artifact (`queries.rbs`)
    // -----------------------------------------------------------------------

    /// Ruby targets write two tracked files, and both must be verified —
    /// `queries.rbs` is what `steep` type-checks caller code against, so a
    /// stale one reports type errors against a schema that no longer exists.
    #[test]
    fn artifact_paths_includes_the_rbs_file_for_ruby_backends_only() {
        let output_dir = std::path::Path::new("/out");

        let ruby = get_backend("ruby-pg", "postgresql").expect("ruby-pg should support postgresql");
        assert!(backend_emits_rbs(ruby.as_ref()), "test premise: ruby-pg emits RBS");
        assert_eq!(
            artifact_paths(ruby.as_ref(), output_dir),
            vec![output_dir.join("queries.rb"), output_dir.join("queries.rbs")]
        );

        let rust = get_backend("rust-sqlx", "postgresql").expect("rust-sqlx should support postgresql");
        assert!(
            !backend_emits_rbs(rust.as_ref()),
            "test premise: rust-sqlx emits no RBS"
        );
        assert_eq!(
            artifact_paths(rust.as_ref(), output_dir),
            vec![output_dir.join("queries.rs")]
        );
    }

    /// A drifted `queries.rbs` must be reported even when `queries.rb` next
    /// to it is perfectly current — the exact situation a schema change
    /// produces when only the RBS signatures move.
    #[test]
    fn verify_provenance_detects_drift_in_the_rbs_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = provenance_test_catalog();
        let schema = catalog.fingerprint();
        let version = env!("CARGO_PKG_VERSION");

        let sql_config = SqlConfig {
            name: "main".to_string(),
            engine: "postgresql".to_string(),
            schema: Vec::new(),
            queries: Vec::new(),
            output: None,
            gen_config: Some(GenTargets::Array(vec![GenTarget {
                backend: "ruby-pg".to_string(),
                output: dir.path().to_string_lossy().into_owned(),
                manifest: None,
                options: std::collections::HashMap::new(),
            }])),
            type_overrides: None,
        };

        std::fs::write(
            dir.path().join("queries.rb"),
            format!("# scythe:provenance v={version} backend=ruby-pg engine=postgresql schema={schema}\n"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("queries.rbs"),
            format!("# scythe:provenance v={version} backend=ruby-pg engine=postgresql schema=sch1:0000000000000000\n"),
        )
        .unwrap();

        let violations = verify_provenance(
            &sql_config,
            &catalog,
            "q1:0000000000000000",
            std::path::Path::new("."),
            default_severities(),
        );

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV01");
        assert!(
            violations[0].message.contains("queries.rbs"),
            "the finding must point at the RBS file, not the .rb next to it: {}",
            violations[0].message
        );
    }

    // -----------------------------------------------------------------------
    // `check_exit_code`
    // -----------------------------------------------------------------------

    #[test]
    fn check_exit_code_is_none_when_clean() {
        assert_eq!(check_exit_code(0, false), None);
    }

    #[test]
    fn check_exit_code_is_two_when_errors_present() {
        assert_eq!(check_exit_code(1, false), Some(2));
    }

    #[test]
    fn check_exit_code_is_none_when_exit_zero_overrides_errors() {
        assert_eq!(check_exit_code(3, true), None);
    }

    #[test]
    fn check_exit_code_is_none_when_clean_even_with_exit_zero() {
        assert_eq!(check_exit_code(0, true), None);
    }

    // -----------------------------------------------------------------------
    // `run_check` end to end
    // -----------------------------------------------------------------------

    /// The regression this whole class of fix exists for: a `[[sql]]` block
    /// whose generation target cannot be constructed must still emit the
    /// lint findings from its queries. Before the original fix,
    /// `verify_provenance`'s `?` unwound out of `run_check` before
    /// `emit_findings` ran, so the report file was created but left empty
    /// and every SQL finding was thrown away.
    ///
    /// `exit_zero: true` is required here for a different reason than the
    /// original version of this test needed it to be absent: #209 made an
    /// unconstructable `[[sql.gen]]` target its own `SC-PRV09` *error*
    /// finding (previously it was folded into `verify_provenance`'s
    /// `SC-PRV07`, which is deliberately `Warn`-only). `run_check` calls
    /// `std::process::exit(2)` directly on an error-severity finding
    /// (matching `audit`/`inspect`), which would tear down the whole test
    /// binary rather than returning -- `exit_zero` is what keeps this an
    /// in-process, `Result`-returning call so the report content can still
    /// be asserted on.
    ///
    /// Asserted through the emitted JSON report rather than through
    /// `verify_provenance` directly, because the discarded findings were
    /// produced by an entirely different part of `run_check` — only an
    /// end-to-end run can show they survive.
    #[test]
    fn run_check_still_emits_lint_findings_when_a_gen_target_cannot_be_constructed() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(
            dir.path().join("schema.sql"),
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);\n",
        )
        .unwrap();
        // `SELECT *` trips SC-S03 (no-select-star), a `Warn` — the finding
        // that must survive alongside the now-`Error`-severity SC-PRV09.
        std::fs::write(
            dir.path().join("queries.sql"),
            "-- @name ListUsers\n-- @returns :many\nSELECT * FROM users;\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("scythe.toml"),
            concat!(
                "[scythe]\nversion = \"1\"\n\n",
                "[[sql]]\nname = \"main\"\nengine = \"postgresql\"\n",
                "schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n",
                "[[sql.gen]]\nbackend = \"rust-sqlx-typo\"\noutput = \"generated\"\n",
            ),
        )
        .unwrap();

        let report_path = dir.path().join("report.json");
        let result = run_check(RunCheckOpts {
            config_path: dir.path().join("scythe.toml").to_string_lossy().into_owned(),
            database_url: None,
            format: "json".to_string(),
            output: Some(report_path.to_string_lossy().into_owned()),
            exit_zero: true,
        });

        assert!(
            result.is_ok(),
            "an unconstructable gen target must not abort the run before findings are emitted: {result:?}"
        );

        let report = std::fs::read_to_string(&report_path).expect("a report file must have been written");
        assert!(
            report.contains("SC-S03"),
            "the lint finding must survive provenance verification:\n{report}"
        );
        assert!(
            report.contains("SC-PRV07"),
            "the unverifiable target must itself be reported:\n{report}"
        );
        assert!(
            report.contains("SC-PRV09"),
            "an unconstructable [sql.gen] target must be its own error finding, matching what \
             `scythe generate` would refuse (#209):\n{report}"
        );
    }

    /// #209, item 1: without `--exit-zero`, an unconstructable `[[sql.gen]]`
    /// target must move `check`'s exit code -- previously it was invisible
    /// to the exit code entirely (only a `Warn`-severity SC-PRV07), so
    /// `check` could report success (exit 0) on a config `scythe generate`
    /// refuses outright.
    #[test]
    fn check_exit_code_reflects_an_unconstructable_gen_target() {
        let violations = validate_gen_targets_constructible(
            &SqlConfig {
                name: "main".to_string(),
                engine: "postgresql".to_string(),
                schema: Vec::new(),
                queries: Vec::new(),
                output: None,
                gen_config: Some(GenTargets::Array(vec![GenTarget {
                    backend: "rust-sqlx-typo".to_string(),
                    output: "generated".to_string(),
                    manifest: None,
                    options: std::collections::HashMap::new(),
                }])),
                type_overrides: None,
            },
            std::path::Path::new("."),
        );

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV09");
        assert_eq!(
            violations[0].severity,
            scythe_lint::Severity::Error,
            "an unconstructable gen target must be an error, unlike SC-PRV07's Warn default, so it moves check's exit code"
        );
        assert!(violations[0].message.contains("rust-sqlx-typo"));
    }

    /// #207: `scythe generate` refuses (by default) to write to an `output`
    /// that escapes the project root, via `resolve_contained_output` inside
    /// `run_generate_inner`. Before this test's fix,
    /// `validate_gen_targets_constructible` never called
    /// `resolve_contained_output` at all, so `scythe check` reported a
    /// config with `output = "../../ESCAPED"` as clean -- green where
    /// `scythe generate` on the very same config is red. This pins that
    /// `check` now reports the same containment violation `generate` would
    /// refuse, as an SC-PRV09 error.
    #[test]
    fn check_reports_a_gen_target_output_that_escapes_the_project_root() {
        let violations = validate_gen_targets_constructible(
            &SqlConfig {
                name: "main".to_string(),
                engine: "postgresql".to_string(),
                schema: Vec::new(),
                queries: Vec::new(),
                output: None,
                gen_config: Some(GenTargets::Array(vec![GenTarget {
                    backend: "rust-sqlx".to_string(),
                    output: "../../ESCAPED".to_string(),
                    manifest: None,
                    options: std::collections::HashMap::new(),
                }])),
                type_overrides: None,
            },
            std::path::Path::new("proj"),
        );

        assert_eq!(
            violations.len(),
            1,
            "an output escaping the project root must be its own SC-PRV09 finding, matching what \
             `scythe generate` refuses by default: {violations:?}"
        );
        assert_eq!(violations[0].rule_id, "SC-PRV09");
        assert_eq!(violations[0].severity, scythe_lint::Severity::Error);
        assert!(
            violations[0].message.contains("escapes the project root"),
            "message should explain the containment rejection: {}",
            violations[0].message
        );
    }
}
