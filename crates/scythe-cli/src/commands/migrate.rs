use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use regex::Regex;
use serde::Deserialize;

use scythe_core::errors::{ErrorCode, ScytheError};

use super::shared::rebase_pattern;

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
    /// Any other sqlc plugin under `gen:` (e.g. a future/experimental
    /// language, or the `json` schema-description plugin) that scythe has no
    /// backend for. Captured via flatten -- rather than left to serde's
    /// default unknown-field handling, which would silently drop it -- so
    /// `convert_config` can name it in a hard error instead of writing a
    /// `scythe.toml` that quietly omits a language the sqlc config asked
    /// for. See issue #97.
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
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

/// Build an error for something wrong with the *user's* sqlc config, as
/// opposed to a fault in scythe.
///
/// `migrate` reads a file the user wrote, so most of its failure modes are
/// theirs to fix: an unparseable YAML document, a `gen:` target scythe has no
/// backend for, a language/engine pair with no driver. Reporting those as
/// `INTERNAL_ERROR` reads as "file a bug" for input scythe diagnosed
/// correctly. Genuine internal faults -- a regex that fails to compile, a
/// TOML serializer that rejects a struct we built ourselves -- keep
/// [`internal`].
fn invalid_config(msg: impl Into<String>) -> ScytheError {
    ScytheError::invalid_config(msg)
}

/// Normalize a sqlc `codegen: [{ plugin: "..." }]` plugin name (v2's
/// `codegen:` array form, distinct from the `gen:` block's already-scoped
/// `go`/`kotlin`/`python` keys) to the scythe language key
/// [`default_driver_for`] and `scythe generate`'s `[sql.gen.<lang>]` expect.
///
/// sqlc's real plugin names for its own official Go/Python/Kotlin/Rust
/// plugins (`sqlc-gen-go`, `sqlc-gen-python`, `sqlc-gen-kotlin`,
/// `sqlc-gen-rust` -- and their bare-name shorthands `golang`/`go`,
/// `python`/`py`, `kotlin`, `rust`) all map to the single scythe language
/// key each language uses. Anything else passes through unchanged: it is
/// not a plugin `migrate` recognizes, so `default_driver_for`'s catch-all
/// reports it by name rather than this function silently guessing.
fn normalize_sqlc_codegen_plugin_lang(plugin: &str) -> String {
    match plugin.to_ascii_lowercase().as_str() {
        "go" | "golang" | "sqlc-gen-go" => "go".to_string(),
        "python" | "py" | "sqlc-gen-python" => "python".to_string(),
        "kotlin" | "sqlc-gen-kotlin" => "kotlin".to_string(),
        "rust" | "sqlc-gen-rust" => "rust".to_string(),
        other => other.to_string(),
    }
}

/// Map a sqlc `gen.<lang>` plugin (or, for v1 `packages`, the implicit Go
/// target) plus a SQL engine to a concrete scythe backend driver suffix --
/// the part after the `<lang>-` prefix in backend names like `go-pgx`.
///
/// sqlc's `gen:` block only ever selects a *language*; the driver is
/// implicit in whichever sqlc plugin the user installed. scythe backends,
/// unlike sqlc plugins, are one-per-driver, so `migrate` has to pick a
/// concrete default. This is the single place that mapping lives -- keeping
/// it here (instead of writing the sqlc plugin name straight into `target`,
/// which is what caused issue #97's `go-go`) means every `(lang, engine)`
/// pair either produces a backend name `scythe_codegen::get_backend`
/// actually recognizes, or fails migration loudly, naming both the language
/// and the engine, rather than writing a `scythe.toml` that only fails much
/// later at `scythe generate` with a confusing "unknown backend" error.
fn default_driver_for(lang: &str, engine: &str) -> Result<&'static str, ScytheError> {
    let normalized_engine = match engine {
        "postgresql" | "postgres" | "pg" | "redshift" => "postgresql",
        "mysql" | "mariadb" => "mysql",
        "sqlite" | "sqlite3" => "sqlite",
        "mssql" | "sqlserver" => "mssql",
        other => other,
    };

    let driver = match (lang, normalized_engine) {
        ("go", "postgresql") => "pgx",
        ("go", "mysql" | "sqlite" | "duckdb" | "mssql") => "database-sql",
        ("go", "oracle") => "godror",
        ("go", "snowflake") => "gosnowflake",

        ("python", "postgresql") => "asyncpg",
        ("python", "mysql") => "aiomysql",
        ("python", "sqlite") => "aiosqlite",
        ("python", "duckdb") => "duckdb",
        ("python", "oracle") => "oracledb",
        ("python", "mssql") => "pyodbc",
        ("python", "snowflake") => "snowflake",

        // kotlin-jdbc is the only Kotlin backend that covers every engine
        // scythe supports, so it is the safe universal default here (unlike
        // kotlin-exposed, which is Postgres-only, or kotlin-r2dbc, which
        // doesn't cover duckdb/mssql/snowflake/oracle).
        ("kotlin", "postgresql" | "mysql" | "sqlite" | "duckdb" | "mssql" | "snowflake" | "oracle") => "jdbc",

        _ => {
            return Err(invalid_config(format!(
                "sqlc gen target '{lang}' has no scythe backend for engine '{engine}'; remove the \
                 `gen.{lang}` block from the sqlc config, or configure `[sql.gen.{lang}]` manually in \
                 scythe.toml after migration"
            )));
        }
    };

    Ok(driver)
}

fn convert_config(sqlc: &SqlcConfig, base_dir: &Path) -> Result<String, ScytheError> {
    let mut sql_blocks: Vec<ScytheSqlBlock> = Vec::new();

    let version = sqlc.version.as_deref().unwrap_or("2");

    if version == "1" || (!sqlc.packages.is_empty() && sqlc.sql.is_empty()) {
        for (idx, pkg) in sqlc.packages.iter().enumerate() {
            let name = pkg.name.clone().unwrap_or_else(|| {
                if sqlc.packages.len() == 1 {
                    "main".to_string()
                } else {
                    format!("sql_{idx}")
                }
            });
            let engine = pkg.engine.clone().unwrap_or_else(|| "postgresql".to_string());
            let schema = pkg.schema.as_ref().map(|s| vec![s.clone()]).unwrap_or_default();
            let queries: Vec<String> = pkg
                .queries
                .as_ref()
                .map(|s| vec![ensure_glob_pattern(s)])
                .unwrap_or_default();
            let output = pkg.path.clone().unwrap_or_else(|| "generated".to_string());

            // ~keep sqlc's v1 `packages` config predates the multi-language `gen:`
            // block and only ever generated Go code, so the equivalent
            // scythe target is always Go -- emitting no gen block here would
            // leave `scythe generate` to silently fall back to its own
            // `rust-sqlx` default, generating the wrong language entirely
            // with no error (issue #97).
            let driver = default_driver_for("go", &engine)?;
            let mut gen_map = BTreeMap::new();
            gen_map.insert(
                "go".to_string(),
                ScytheGenTarget {
                    target: driver.to_string(),
                    derive: Vec::new(),
                },
            );

            sql_blocks.push(ScytheSqlBlock {
                name,
                engine,
                schema,
                queries,
                output,
                gen_block: Some(gen_map),
                type_overrides: Vec::new(),
            });
        }
    } else {
        for (idx, entry) in sqlc.sql.iter().enumerate() {
            let engine = entry.engine.clone().unwrap_or_else(|| "postgresql".to_string());

            let schema: Vec<String> = entry.schema.as_ref().map(|v| v.to_vec()).unwrap_or_default();

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

            for cg in &entry.codegen {
                if let Some(out) = &cg.out {
                    output = out.clone();
                }
                let raw_plugin = cg.plugin.clone().unwrap_or_else(|| "rust".to_string());
                let lang = normalize_sqlc_codegen_plugin_lang(&raw_plugin);

                // Rust keeps the `crate` option (or its "tokio-postgres"
                // default) as the driver suffix, exactly as before -- those
                // values are themselves real `rust-*` backend suffixes
                // (`rust-sqlx`, `rust-tokio-postgres`), so the existing
                // mapping is already correct for Rust. Every other
                // language routes through `default_driver_for`, the same
                // sqlc-plugin -> scythe-driver mapping the `gen:` block
                // below uses: before this, a non-Rust `codegen:` plugin
                // (e.g. `plugin: golang`) wrote the raw, un-normalized
                // plugin name as the language key and unconditionally
                // defaulted `target` to `"tokio-postgres"` -- a Rust driver
                // name -- producing `[sql.gen.golang] target =
                // "tokio-postgres"`, which `scythe generate` then rejected
                // as `has no backend for language(s): golang`. See issue
                // #211, item 2.
                let target = if lang == "rust" {
                    cg.options
                        .as_ref()
                        .and_then(|o| o.crate_name.clone())
                        .unwrap_or_else(|| "tokio-postgres".to_string())
                } else {
                    default_driver_for(&lang, &engine)?.to_string()
                };
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
                        // `target` must be a real driver suffix (e.g.
                        // "pgx"), never the bare language name -- `scythe
                        // generate` builds the backend as
                        // `format!("{lang}-{target}")`, so `target = lang`
                        // produced unusable names like "go-go" (issue #97).
                        let driver = default_driver_for(lang, &engine)?;
                        gen_map.insert(
                            lang.to_string(),
                            ScytheGenTarget {
                                target: driver.to_string(),
                                derive: Vec::new(),
                            },
                        );
                    }
                }

                if !g.other.is_empty() {
                    let mut unsupported: Vec<&str> = g.other.keys().map(String::as_str).collect();
                    unsupported.sort_unstable();
                    return Err(invalid_config(format!(
                        "sqlc gen target(s) not supported by scythe migrate: {}; remove them from `gen:` \
                         or configure their scythe backend manually in scythe.toml after migration",
                        unsupported.join(", ")
                    )));
                }
            }

            let name = if sqlc.sql.len() == 1 {
                "main".to_string()
            } else {
                format!("sql_{idx}")
            };

            let gen_opt = if gen_map.is_empty() { None } else { Some(gen_map) };

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

    let toml_string = toml::to_string_pretty(&config).map_err(|e| internal(format!("toml serialize: {e}")))?;

    let dest = base_dir.join("scythe.toml");

    // Back up an existing `scythe.toml` before overwriting it, exactly like
    // every converted query file gets a `.sql.bak` -- `migrate` is a
    // best-effort, one-shot conversion tool, and a hand-written or
    // previously-migrated `scythe.toml` sitting at the destination is
    // exactly as much the user's own work as a hand-annotated query file
    // is. Before this, `migrate` silently discarded it with no backup,
    // prompt, `--dry-run`, or `--force`. See issue #211, item 1.
    if dest.exists() {
        let bak = dest.with_extension("toml.bak");
        let existing =
            fs::read_to_string(&dest).map_err(|e| internal(format!("read existing {}: {e}", dest.display())))?;
        fs::write(&bak, &existing).map_err(|e| internal(format!("backup {}: {e}", bak.display())))?;
    }

    fs::write(&dest, &toml_string).map_err(|e| internal(format!("write {}: {e}", dest.display())))?;

    Ok(dest.display().to_string())
}

struct ConvertStats {
    files: usize,
    queries: usize,
    params_renamed: usize,
}

/// Build the glob pattern used to find `.sql` files under `base_dir` for a
/// single `queries` entry.
///
/// The directory-vs-glob decision — is `qp` already a glob pattern, or is it
/// a bare directory that needs `/*.sql` appended — is made on the raw,
/// un-rebased `qp`, never on the string after `base_dir` has been prefixed
/// onto it: a `*`, `?`, or `[...]` that appears only in `base_dir` (e.g. a
/// project literally named `a[b]`) must not affect this decision. The
/// `base_dir.join(qp).is_dir()` filesystem probe below is safe to build with
/// a plain [`Path::join`] because it is only used to test for existence —
/// never fed to [`glob::glob`] — unlike the pattern itself.
///
/// The result is rebased onto `base_dir` via [`rebase_pattern`], which
/// escapes glob metacharacters in `base_dir` (never in `qp`, which is meant
/// to be a glob) and joins with `/` regardless of platform, so a `base_dir`
/// containing `[`, `]`, `*`, or `?` is treated as a literal path component
/// instead of compiled as glob syntax, and Windows's `\` separator is never
/// fed to `glob::glob` (which treats `\` as an escape character).
fn resolve_query_glob(qp: &str, base_dir: &Path) -> String {
    let dir_pattern = if qp.contains('*') {
        qp.to_string()
    } else if base_dir.join(qp).is_dir() {
        format!("{}/*.sql", qp.trim_end_matches('/'))
    } else {
        qp.to_string()
    };

    rebase_pattern(&dir_pattern, base_dir).into_owned()
}

/// Convert all query files found under the given paths.
///
/// A `queries` entry that matches no files is reported as a warning on
/// stderr (naming the pattern, the resolved glob, and the base directory)
/// rather than aborting the whole migration: `migrate` walks a list of
/// `queries` entries potentially spanning several `[[sql]]`/`packages`
/// blocks, and one stale or empty entry should not stop the rest of the
/// project from being converted. This intentionally diverges from
/// `shared::resolve_globs` (used by `generate`/`check`/`lint`/`audit`/`fmt`),
/// which hard-errors on a zero match — those commands resolve a single
/// config's globs before doing any work, so failing fast is cheap and there
/// is nothing partial to preserve. `migrate` is a one-shot, best-effort
/// conversion tool where partial progress (and a `.sql.bak` trail) has more
/// value than an all-or-nothing abort.
fn convert_query_files(query_paths: &[String], base_dir: &Path) -> Result<ConvertStats, ScytheError> {
    let mut stats = ConvertStats {
        files: 0,
        queries: 0,
        params_renamed: 0,
    };

    for qp in query_paths {
        let glob_pattern = resolve_query_glob(qp, base_dir);

        let entries = glob::glob(&glob_pattern).map_err(|e| internal(format!("glob {glob_pattern}: {e}")))?;

        let mut matched_any = false;
        for entry in entries {
            let path = entry.map_err(|e| internal(format!("glob entry: {e}")))?;
            if !path.is_file() {
                continue;
            }
            matched_any = true;
            let (q, p) = convert_single_file(&path)?;
            // Only a file `migrate` actually rewrote counts as "converted".
            // A file with no `-- name:` annotations passes through
            // unchanged (`convert_query_content` returns `query_count: 0`
            // for it) and previously still incremented `stats.files`,
            // reporting a no-op pass as a conversion. See issue #211, item 3.
            if q > 0 {
                stats.files += 1;
            }
            stats.queries += q;
            stats.params_renamed += p;
        }

        if !matched_any {
            eprintln!(
                "warning: queries pattern '{qp}' matched no files (resolved: {glob_pattern}, base dir: {base})",
                base = base_dir.display()
            );
        }
    }

    Ok(stats)
}

/// Convert a single .sql query file in-place (with .bak backup).
fn convert_single_file(path: &Path) -> Result<(usize, usize), ScytheError> {
    let content = fs::read_to_string(path).map_err(|e| internal(format!("read {}: {e}", path.display())))?;

    let (converted, query_count, param_count) = convert_query_content(&content)
        .map_err(|e| ScytheError::new(e.code, format!("{}: {}", path.display(), e.message)))?;

    if converted != content {
        let bak = path.with_extension("sql.bak");
        fs::write(&bak, &content).map_err(|e| internal(format!("backup {}: {e}", bak.display())))?;
        fs::write(path, &converted).map_err(|e| internal(format!("write {}: {e}", path.display())))?;
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

    let sqlc_arg_re = Regex::new(r"sqlc\.arg\((\w+)\)").map_err(|e| internal(format!("regex: {e}")))?;

    let sqlc_narg_re = Regex::new(r"sqlc\.narg\((\w+)\)").map_err(|e| internal(format!("regex: {e}")))?;

    let positional_re = Regex::new(r"\$(\d+)").map_err(|e| internal(format!("regex: {e}")))?;

    // A typo'd sqlc directive (wrong-case return type, an unsupported keyword, a missing
    // colon) does not match `annotation_re`, so before this check it fell straight through to
    // the output untouched: the query it names was never converted, `migrate` still reported
    // success, and nothing told the caller that query was skipped. Loose enough to catch any
    // line that is clearly *trying* to be a `-- name:` directive (case-insensitive, tolerant of
    // spacing) without caring about the return-type keyword, so it never matches an unrelated
    // comment that happens to contain the word "name". See #152.
    let loose_name_re = Regex::new(r"(?mi)^--\s*name\s*:\s*.*$").map_err(|e| internal(format!("regex: {e}")))?;

    let mut output = String::with_capacity(input.len());
    let mut query_count: usize = 0;
    let mut param_rename_count: usize = 0;

    let mut match_positions: Vec<(usize, usize, String, String)> = Vec::new();
    for caps in annotation_re.captures_iter(input) {
        let m = caps.get(0).unwrap();
        let name = caps[1].to_string();
        let return_type = caps[2].to_string();
        match_positions.push((m.start(), m.end(), name, return_type));
    }

    // Checked before the `is_empty` early return below: a file whose *only* directive is
    // malformed has no strict matches at all, and would otherwise pass through as if it had no
    // sqlc annotations to convert.
    let strict_starts: std::collections::HashSet<usize> = match_positions.iter().map(|(start, ..)| *start).collect();
    for m in loose_name_re.find_iter(input) {
        if !strict_starts.contains(&m.start()) {
            return Err(invalid_config(format!(
                "unrecognised sqlc query annotation -- expected `-- name: <Identifier> \
                 :<one|many|exec|execrows|execresult|batchone|batchmany|batchexec|copyfrom>`, got: {:?}",
                m.as_str().trim()
            )));
        }
    }

    if match_positions.is_empty() {
        return Ok((input.to_string(), 0, 0));
    }

    if match_positions[0].0 > 0 {
        output.push_str(&input[..match_positions[0].0]);
    }

    for (i, (_, end, name, return_type)) in match_positions.iter().enumerate() {
        query_count += 1;

        let body_end = if i + 1 < match_positions.len() {
            match_positions[i + 1].0
        } else {
            input.len()
        };
        let body = &input[*end..body_end];

        let mut max_positional: usize = 0;
        for caps in positional_re.captures_iter(body) {
            if let Ok(n) = caps[1].parse::<usize>()
                && n > max_positional
            {
                max_positional = n;
            }
        }

        let mut next_param = max_positional + 1;
        let mut param_names: Vec<String> = Vec::new();
        let mut converted_body = body.to_string();

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

            let matched_text = m.as_str();
            let pname = if let Some(caps) = sqlc_arg_re.captures(matched_text) {
                caps[1].to_string()
            } else if let Some(caps) = sqlc_narg_re.captures(matched_text) {
                caps[1].to_string()
            } else {
                break;
            };

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

        output.push_str(&format!("-- @name {name}\n"));
        output.push_str(&format!("-- @returns :{return_type}\n"));

        for pname in &param_names {
            output.push_str(&format!("-- @param {pname}\n"));
        }

        output.push_str(&converted_body);
    }

    // A `sqlc.arg`/`sqlc.narg` call survives the loop above only when its parameter name didn't
    // match `\w+` (stray whitespace inside the parens, an empty name) -- the exact same
    // silent-passthrough failure mode as the annotation check above, just for parameter names
    // instead of query names. Left unchecked, the call ships verbatim in a file scythe's own SQL
    // parser does not understand, while `migrate` still reports every query as converted. See
    // #152.
    let loose_param_re = Regex::new(r"(?i)sqlc\s*\.\s*n?arg\s*\(").map_err(|e| internal(format!("regex: {e}")))?;
    if let Some(m) = loose_param_re.find(&output) {
        return Err(invalid_config(format!(
            "unrecognised sqlc parameter reference -- expected `sqlc.arg(name)` or \
             `sqlc.narg(name)` with a plain identifier and no internal whitespace, got: {:?}",
            m.as_str()
        )));
    }

    Ok((output, query_count, param_rename_count))
}

pub fn run_migrate(sqlc_config_path: &Path) -> Result<(), ScytheError> {
    if !sqlc_config_path.exists() {
        return Err(invalid_config(format!(
            "config file not found: {}",
            sqlc_config_path.display()
        )));
    }

    let raw = fs::read_to_string(sqlc_config_path)
        .map_err(|e| internal(format!("read {}: {e}", sqlc_config_path.display())))?;

    let sqlc: SqlcConfig = if sqlc_config_path.extension().is_some_and(|ext| ext == "json") {
        serde_json::from_str(&raw).map_err(|e| invalid_config(format!("parse json config: {e}")))?
    } else {
        serde_yaml::from_str(&raw).map_err(|e| invalid_config(format!("parse yaml config: {e}")))?
    };

    let base_dir = sqlc_config_path.parent().unwrap_or_else(|| Path::new("."));

    let config_dest = convert_config(&sqlc, base_dir)?;
    println!("Generated config: {config_dest}");

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

    let stats = convert_query_files(&all_query_paths, base_dir)?;

    println!(
        "Migration complete: {} file(s) converted, {} query/queries found, {} param(s) renamed",
        stats.files, stats.queries, stats.params_renamed
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A base directory containing glob metacharacters (e.g. a project
    /// literally named `a[b]`) must be escaped before being prefixed onto
    /// the query pattern, or the bracket would be compiled as a character
    /// class and silently match nothing. Regression test for issue #88.
    #[test]
    fn resolve_query_glob_escapes_base_dir_metacharacters() {
        let base = Path::new("a[b]");
        assert_eq!(resolve_query_glob("queries/*.sql", base), "a[[]b[]]/queries/*.sql");
    }

    /// The directory-vs-glob decision (does the pattern already contain `*`)
    /// must be made on the raw, un-rebased pattern — never on the string
    /// after `base_dir` has been joined on. A `*` living only in `base_dir`
    /// must not cause a bare-directory `queries` pattern to skip the
    /// `/*.sql` append.
    #[test]
    fn resolve_query_glob_decides_on_raw_pattern_not_joined_string() {
        let base = Path::new("a*b");
        // "queries" has no '*' of its own, and (being a nonexistent path)
        // is not a real directory either, so it passes through unchanged —
        // proving the decision used the raw "queries", not the joined
        // "a*b/queries" (which does contain '*' and would otherwise have
        // skipped the is_dir() branch for the wrong reason).
        assert_eq!(resolve_query_glob("queries", base), "a[*]b/queries");
    }

    /// When the raw pattern names a real directory (checked by joining
    /// `base_dir` purely for the filesystem probe, never for the pattern
    /// text itself), `/*.sql` must be appended.
    #[test]
    fn resolve_query_glob_appends_glob_for_real_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("queries")).unwrap();

        assert_eq!(
            resolve_query_glob("queries", tmp.path()),
            format!("{}/queries/*.sql", tmp.path().display())
        );
    }

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

    /// Verified against board #127/#152's suspicion that this file pins defective output around
    /// (then-)lines 848-849: hand-traced `convert_query_content` for `page_limit` and
    /// `page_offset` and confirmed each `sqlc.arg(...)` call is replaced with its own
    /// sequentially-numbered placeholder and its own `-- @param` line -- the intended mapping,
    /// not a bug this test happens to encode. Left un-inverted: inverting a correct assertion
    /// would itself become a bug-pinning test. The same check was made for
    /// `test_sqlc_narg_conversion`, `test_repeated_arg_same_name` and
    /// `test_mixed_arg_and_narg` -- their `-- @param` assertions are correct output too.
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

    /// See `test_sqlc_arg_conversion`'s doc comment: verified correct, not bug-pinning.
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

    /// See `test_sqlc_arg_conversion`'s doc comment: verified correct, not bug-pinning.
    #[test]
    fn test_repeated_arg_same_name() {
        let input = "\
-- name: Test :one
SELECT * FROM t WHERE a = sqlc.arg(x) AND b = sqlc.arg(x);
";
        let (out, _qc, pc) = convert_query_content(input).unwrap();
        assert_eq!(pc, 2);
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

    /// See `test_sqlc_arg_conversion`'s doc comment: verified correct, not bug-pinning.
    #[test]
    fn test_mixed_arg_and_narg() {
        let input = "\
-- name: Mixed :many
SELECT * FROM t
WHERE a = sqlc.arg(foo) AND b = sqlc.narg(bar) AND c = $1;
";
        let (out, _, pc) = convert_query_content(input).unwrap();
        assert_eq!(pc, 2);
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

    /// Regression for #152: a wrong-case return-type keyword (sqlc itself is always lowercase)
    /// used to fail the strict `annotation_re` silently -- the query was left unconverted and
    /// `migrate` still reported success. It must now be a load error.
    #[test]
    fn convert_query_content_rejects_a_malformed_return_type_keyword() {
        let input = "-- name: GetProject :One\nSELECT id FROM projects WHERE id = $1;\n";
        let error = convert_query_content(input)
            .expect_err("a wrong-case return-type keyword must be rejected, not silently skipped")
            .to_string();
        assert!(
            error.contains("unrecognised sqlc query annotation"),
            "error must identify the problem, got: {error}"
        );
    }

    /// Regression for #152: a file whose *only* directive is malformed has no strict matches at
    /// all, and the early `match_positions.is_empty()` return used to let it through as if the
    /// file had no sqlc annotations to convert -- the check above must run before that return.
    #[test]
    fn convert_query_content_rejects_a_malformed_annotation_when_it_is_the_only_one() {
        let input = "-- name: GetProject\nSELECT id FROM projects WHERE id = $1;\n";
        assert!(
            convert_query_content(input).is_err(),
            "a directive-only file with no valid annotations must not pass through silently"
        );
    }

    /// Regression for #152: internal whitespace inside `sqlc.arg(...)` doesn't match `\w+`, so
    /// the call used to survive verbatim in the converted output -- a query scythe's own parser
    /// does not understand, shipped while `migrate` reported success.
    #[test]
    fn convert_query_content_rejects_a_sqlc_arg_call_with_internal_whitespace() {
        let input = "-- name: GetPage :many\nSELECT * FROM t LIMIT sqlc.arg( page_limit );\n";
        let error = convert_query_content(input)
            .expect_err("a malformed sqlc.arg call must be rejected, not left unconverted")
            .to_string();
        assert!(
            error.contains("unrecognised sqlc parameter reference"),
            "error must identify the problem, got: {error}"
        );
    }
}
