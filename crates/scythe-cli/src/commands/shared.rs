use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Splits a .sql file containing multiple queries separated by `-- name:` or
/// `-- @name` annotations. Returns one string per query block (annotation +
/// SQL). Content before the first annotation is discarded -- see
/// [`has_unannotated_sql`] for detecting when that discard is silently
/// dropping real SQL rather than an intentional header comment.
pub fn split_query_file(content: &str) -> Vec<String> {
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

/// Whether `content` has SQL text that [`split_query_file`] would silently
/// discard: a non-blank, non-`--`-comment line appearing before the first
/// `-- name:`/`-- @name` annotation, or -- when the file has no annotation
/// at all -- anywhere in the file.
///
/// A line counts as "content" only if it is non-blank and not an ordinary
/// `--` comment, so a file that opens with a license header or a plain
/// explanatory comment does not trip this check; only a file containing a
/// forgotten or mistyped annotation (or none at all) does. See issue #204:
/// `scythe generate`/`scythe check` used to report success on a query file
/// reduced to zero blocks, and `scythe audit` on the same file found real
/// findings the other commands never saw because they never looked at the
/// SQL at all.
pub fn has_unannotated_sql(content: &str) -> bool {
    let mut saw_content = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("-- name:") || trimmed.starts_with("-- @name") {
            return saw_content;
        }
        if !trimmed.starts_with("--") {
            saw_content = true;
        }
    }
    saw_content
}

/// Map scythe engine names to sqruff dialect names.
pub fn engine_to_sqruff_dialect(engine: &str) -> &str {
    match engine {
        "postgresql" | "postgres" | "pg" => "postgres",
        "mysql" | "mariadb" => "mysql",
        "sqlite" | "sqlite3" => "sqlite",
        "duckdb" => "duckdb",
        "mssql" | "sqlserver" | "tsql" => "tsql",
        "redshift" => "redshift",
        "snowflake" => "snowflake",
        "oracle" => "oracle",
        _ => "ansi",
    }
}

/// Every dialect name `sqruff_lib_core::dialects::init::DialectKind` accepts
/// (`DialectKind::name()`). scythe-cli does not depend on sqruff-lib
/// directly -- `scythe-lint` wraps it -- so this list is duplicated here
/// deliberately: it is what lets [`validate_dialect`] reject a bad
/// `--dialect` value *before* it reaches `FluffConfig::from_source`, which
/// resolves an unrecognized dialect string via `DialectKind::from_str(..).unwrap()`
/// and panics (exit 101) rather than returning an error. See issue #205.
const KNOWN_SQRUFF_DIALECTS: &[&str] = &[
    "ansi",
    "athena",
    "bigquery",
    "clickhouse",
    "databricks",
    "db2",
    "duckdb",
    "greenplum",
    "mysql",
    "oracle",
    "postgres",
    "redshift",
    "snowflake",
    "sparksql",
    "sqlite",
    "trino",
    "tsql",
];

/// scythe engine aliases [`engine_to_sqruff_dialect`] recognizes -- listed so
/// [`validate_dialect`] can tell "a recognized scythe engine alias" (which
/// must translate through [`engine_to_sqruff_dialect`], e.g. `postgresql` ->
/// `postgres`) apart from "unrecognized input" (which must be rejected, not
/// silently degraded to `ansi` the way [`engine_to_sqruff_dialect`]'s
/// catch-all does for config-derived engine values).
const KNOWN_ENGINE_ALIASES: &[&str] = &[
    "postgresql",
    "postgres",
    "pg",
    "mysql",
    "mariadb",
    "sqlite",
    "sqlite3",
    "duckdb",
    "mssql",
    "sqlserver",
    "tsql",
    "redshift",
    "snowflake",
    "oracle",
];

/// Validate and normalize a `--dialect` value supplied on the command line
/// (`fmt --dialect`, `lint --dialect`) into a sqruff dialect name sqruff-lib
/// is guaranteed to accept.
///
/// Accepts two kinds of input, case-insensitively:
/// - A sqruff-native dialect name (`postgres`, `bigquery`, `tsql`, ...).
/// - A scythe engine alias ([`engine_to_sqruff_dialect`]'s canonical
///   input set, e.g. `postgresql`, `pg`, `mariadb`), translated to its
///   sqruff equivalent.
///
/// Anything else -- a typo, an engine scythe has no sqruff dialect for, or
/// gibberish -- is rejected with a message listing every accepted value,
/// instead of being passed through to sqruff-lib where it panics (see
/// [`KNOWN_SQRUFF_DIALECTS`]'s doc comment).
pub fn validate_dialect(raw: &str) -> Result<String, String> {
    let lower = raw.to_ascii_lowercase();
    if KNOWN_SQRUFF_DIALECTS.contains(&lower.as_str()) {
        return Ok(lower);
    }
    if KNOWN_ENGINE_ALIASES.contains(&lower.as_str()) {
        return Ok(engine_to_sqruff_dialect(&lower).to_string());
    }
    Err(format!(
        "unknown SQL dialect '{raw}'; accepted values: {}",
        KNOWN_SQRUFF_DIALECTS.join(", ")
    ))
}

/// Try to read the SQL dialect from a scythe.toml config file.
///
/// Returns `Ok(None)` when the config file does not exist -- a legitimate,
/// common case in explicit-file mode (`scythe fmt`/`scythe lint` on a bare
/// file list, with no project config at all). Returns `Err` when the file
/// *does* exist but cannot be read or fails to parse as valid TOML, instead
/// of silently degrading to `Ok(None)` (which every caller then reads as "no
/// dialect configured" and defaults to `ansi`).
///
/// Before this distinction, `scythe lint --config broken.toml pg.sql`
/// swallowed a genuinely invalid `--config` and linted with the wrong
/// dialect with no error at all -- see issue #206, item 4.
pub fn dialect_from_config(config_path: &str) -> Result<Option<String>, String> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct MinConfig {
        sql: Vec<MinSql>,
    }

    #[derive(Deserialize)]
    struct MinSql {
        #[serde(default)]
        engine: Option<String>,
    }

    let config_str = match std::fs::read_to_string(config_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("failed to read config '{config_path}': {e}")),
    };
    let config: MinConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{config_path}': {e}"))?;
    Ok(config
        .sql
        .first()
        .and_then(|s| s.engine.as_deref())
        .map(|e| engine_to_sqruff_dialect(e).to_string()))
}

/// Redact the password component of a connection URL like
/// `postgres://user:secret@host:5432/db` before it is printed to stderr, a
/// log line, or a CI console — never the URL itself.
///
/// Only the password between `:` and `@` in the userinfo component is
/// replaced with `***`; the scheme, username, host, port, path, and query
/// string are left intact so the message stays useful for debugging. URLs
/// with no `user:password@` component (or non-URL strings) are returned
/// unchanged since there is nothing to redact.
pub fn redact_url_password(url: &str) -> String {
    let Ok(re) = regex::Regex::new(r"(?P<prefix>://[^:/?#@\s]+):[^@/?#\s]+@") else {
        return url.to_string();
    };
    re.replace(url, "${prefix}:***@").into_owned()
}

/// The directory a `scythe.toml` config file lives in. `schema`/`queries`
/// glob patterns and `output` directories in the config are resolved
/// relative to this directory, not the process's current working directory.
///
/// Falls back to `"."` when `config_path` has no parent component (e.g. a
/// bare filename like `"scythe.toml"`).
///
/// `Path::parent()` returns `Some(Path::new(""))` -- not `None` -- for a
/// single-component relative path like `"scythe.toml"`, so the `None`
/// fallback above never actually fires for the common case of running from
/// the project root with the default config name. Left unhandled, every
/// message that prints this directory (e.g. `resolve_globs`'s "config dir:"
/// line) rendered an empty string instead of the intended `.`. Both the
/// `None` and `Some("")` cases are normalized to `"."` here.
pub fn config_dir(config_path: &str) -> &Path {
    let parent = Path::new(config_path).parent().unwrap_or_else(|| Path::new("."));
    if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    }
}

/// Returns `true` when `output` (a relative path from `[[sql.gen]]`'s
/// `output = "..."`) would, once joined onto the project root, resolve
/// outside it -- i.e. a leading run of `..` components that is never
/// balanced by a preceding normal component. Absolute paths (including
/// Windows-style rooted paths, via [`Path::has_root`]) always escape,
/// regardless of the base directory.
///
/// Purely lexical: it walks `output`'s own components and tracks a
/// directory-depth counter, going negative exactly when a `..` pops past
/// the join point. This is deliberately *not* implemented by canonicalizing
/// `base_dir.join(output)` and checking `starts_with` -- the output
/// directory frequently does not exist yet (that is what `scythe generate`
/// is about to create), and canonicalization requires the path to exist.
/// It is also not implemented by naively collapsing `..` against an
/// accumulator that silently drops unmatched leading `..` components (the
/// way e.g. `cargo_util::paths::normalize_path` does) -- that approach is
/// designed for cleaning up already-safe paths, not for a security
/// containment check, and would make `"../../ESCAPED"` normalize to
/// `"ESCAPED"` and pass. See issue #207.
pub fn output_escapes_base(output: &Path) -> bool {
    use std::path::Component;

    if output.is_absolute() || output.has_root() {
        return true;
    }

    let mut depth: i64 = 0;
    for component in output.components() {
        match component {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return true,
        }
    }
    false
}

/// Join `output` onto `base_dir`, rejecting the result when it escapes
/// `base_dir` (see [`output_escapes_base`]) unless `allow_escape` is set.
///
/// `label` identifies the config entry in the error message (e.g. the
/// backend name) so a rejected path can be traced back to the `[[sql.gen]]`
/// entry that produced it.
pub fn resolve_contained_output(
    base_dir: &Path,
    output: &str,
    label: &str,
    allow_escape: bool,
) -> Result<PathBuf, String> {
    let joined = base_dir.join(output);
    if allow_escape || !output_escapes_base(Path::new(output)) {
        return Ok(joined);
    }
    Err(format!(
        "{label}: output '{output}' escapes the project root '{base}'; scythe refuses to create \
         directories or write files outside the project by default -- pass --allow-output-escape \
         if writing there is deliberate",
        base = base_dir.display(),
    ))
}

/// Rebase a single glob pattern onto `base_dir`, unless the pattern is
/// already absolute or `base_dir` carries no information.
///
/// Rules, in order:
/// 1. Absolute patterns (checked via [`Path::has_root`], not
///    [`Path::is_absolute`] — on Windows a Unix-style absolute pattern like
///    `/abs/x` is *not* `is_absolute()` but *is* `has_root()`) pass through
///    unchanged.
/// 2. An empty or `"."` `base_dir` (e.g. `config_dir("scythe.toml")`, or
///    `config_dir("./scythe.toml")`) is treated as identity: the pattern is
///    returned unchanged rather than joined, so `"./db/*.sql"` and
///    `"db/*.sql"` report matches under the same spelling the user wrote.
///    This keeps `FileViolation.file` / SARIF / JSON output byte-stable
///    across equivalent `--config` spellings.
/// 3. Otherwise the pattern is prefixed with `base_dir`, escaping `base_dir`
///    (not the pattern) via [`glob::Pattern::escape`] so directory names
///    containing glob metacharacters (`[`, `]`, `?`, `*`, ...) are treated as
///    literal path components rather than compiled as glob syntax. Plain
///    `format!` string concatenation is used instead of [`Path::join`]: glob
///    accepts `/` as a separator on every platform, and `join` would
///    re-parse the escaped string and could reintroduce a platform separator
///    that disturbs the escaping.
pub(crate) fn rebase_pattern<'a>(pattern: &'a str, base_dir: &Path) -> Cow<'a, str> {
    let p = Path::new(pattern);
    if p.is_absolute() || p.has_root() {
        return Cow::Borrowed(pattern);
    }

    if base_dir.as_os_str().is_empty() || base_dir == Path::new(".") {
        return Cow::Borrowed(pattern);
    }

    let escaped_base = glob::Pattern::escape(&base_dir.to_string_lossy());
    let escaped_base = escaped_base.trim_end_matches(['/', '\\']);
    Cow::Owned(format!("{escaped_base}/{pattern}"))
}

/// Resolve a list of glob patterns relative to `base_dir` (see
/// [`rebase_pattern`]) and return every matched path as a string.
///
/// `label` identifies the call site (e.g. `"[main] schema"`) for the error
/// message emitted when a pattern matches no files — matching nothing is
/// treated as a hard configuration error rather than a warning, since paths
/// in `scythe.toml` resolve relative to the config file's directory (not the
/// process's current working directory) as of 0.13.0, and an empty match is
/// the most common symptom of a stale CWD-relative pattern.
pub fn resolve_globs(
    patterns: &[String],
    base_dir: &Path,
    label: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut paths = Vec::new();
    for pattern in patterns {
        let rebased = rebase_pattern(pattern, base_dir);
        let matches: Vec<_> = glob::glob(&rebased)?.collect::<Result<Vec<_>, _>>()?;
        if matches.is_empty() {
            return Err(format!(
                "{label} pattern '{pattern}' matched no files\n  \
                 config dir: {config_dir}\n  \
                 resolved:   {resolved}\n  \
                 note: paths in scythe.toml resolve relative to the config file's directory,\n        \
                 not the current working directory (changed in 0.13.0)",
                config_dir = base_dir.display(),
                resolved = rebased,
            )
            .into());
        }
        for path in matches {
            paths.push(path.display().to_string());
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_password_masks_password_only() {
        let redacted = redact_url_password("postgres://scythe:hunter2@localhost:5432/mydb?sslmode=disable");
        assert_eq!(redacted, "postgres://scythe:***@localhost:5432/mydb?sslmode=disable");
        assert!(!redacted.contains("hunter2"), "password must not survive redaction");
    }

    #[test]
    fn redact_url_password_leaves_urls_without_password_unchanged() {
        assert_eq!(
            redact_url_password("postgres://localhost:5432/mydb"),
            "postgres://localhost:5432/mydb"
        );
        assert_eq!(
            redact_url_password("postgres://scythe@localhost:5432/mydb"),
            "postgres://scythe@localhost:5432/mydb"
        );
    }

    #[test]
    fn redact_url_password_leaves_non_url_strings_unchanged() {
        assert_eq!(redact_url_password("not a url at all"), "not a url at all");
        assert_eq!(redact_url_password(""), "");
    }

    /// Absolute patterns pass through untouched regardless of `base_dir`.
    ///
    /// `has_root()` (not `is_absolute()`) is what `rebase_pattern` checks,
    /// because a Unix-style absolute pattern like `/abs/x` is NOT
    /// `is_absolute()` on Windows but IS `has_root()` there — so this is the
    /// check that keeps such patterns passing through unchanged on every
    /// platform. A Windows drive-letter pattern (`C:\x`) is only meaningfully
    /// "absolute" under Windows path semantics, so that half of the
    /// assertion is compiled in for Windows only; `has_root()`/
    /// `is_absolute()` both report `false` for it on Unix, where it is just
    /// an ordinary (if unusual) relative filename.
    #[test]
    fn rebase_pattern_passes_absolute_through() {
        let base = Path::new("/some/base");
        assert_eq!(rebase_pattern("/abs/x", base), Cow::Borrowed("/abs/x"));

        #[cfg(windows)]
        {
            let win_base = Path::new(r"C:\some\base");
            assert_eq!(rebase_pattern(r"C:\x", win_base), Cow::Borrowed(r"C:\x"));
        }
    }

    /// An empty `base_dir` (from `config_dir("scythe.toml")`) and an
    /// explicit `"."` `base_dir` (from `config_dir("./scythe.toml")`) must
    /// both be treated as identity, so equivalent `--config` spellings
    /// report matched paths using the exact string the caller wrote instead
    /// of a `./`-prefixed variant.
    #[test]
    fn rebase_pattern_is_identity_for_empty_and_dot_base() {
        assert_eq!(rebase_pattern("db/*.sql", Path::new("")), Cow::Borrowed("db/*.sql"));
        assert_eq!(rebase_pattern("db/*.sql", Path::new(".")), Cow::Borrowed("db/*.sql"));
    }

    /// A base directory containing glob metacharacters (e.g. a project
    /// literally named `a[b]`) must be escaped via `glob::Pattern::escape`
    /// before being prefixed onto the pattern, or the bracket would be
    /// compiled as a character class and silently match nothing. Only the
    /// base is escaped — the user's own pattern passes through untouched.
    #[test]
    fn rebase_pattern_escapes_metacharacters_in_base() {
        let base = Path::new("a[b]");
        assert_eq!(
            rebase_pattern("x.sql", base),
            Cow::<str>::Owned("a[[]b[]]/x.sql".to_string())
        );
    }

    /// #204 regression: a file with real SQL and no annotation at all must
    /// be flagged, not treated the same as a comment-only stub.
    #[test]
    fn has_unannotated_sql_detects_a_file_with_no_annotation_at_all() {
        let content = "SELECT id, name FROM users WHERE id = $1;\nDELETE FROM users;\n";
        assert!(has_unannotated_sql(content));
    }

    /// A real statement sitting before the first annotation is exactly the
    /// case `split_query_file` silently discards.
    #[test]
    fn has_unannotated_sql_detects_content_before_first_annotation() {
        let content = "DELETE FROM users;\n-- name: GetUser :one\nSELECT * FROM users WHERE id = $1;\n";
        assert!(has_unannotated_sql(content));
    }

    /// A header comment (not an annotation) before the first real annotation
    /// is the normal, intentional case and must not be flagged.
    #[test]
    fn has_unannotated_sql_allows_a_leading_comment_header() {
        let content =
            "-- License: MIT\n-- Copyright 2024\n\n-- name: GetUser :one\nSELECT * FROM users WHERE id = $1;\n";
        assert!(!has_unannotated_sql(content));
    }

    /// A comment-only file (or an empty one) has nothing to silently drop.
    #[test]
    fn has_unannotated_sql_allows_comment_only_file() {
        assert!(!has_unannotated_sql("-- just a comment\n"));
        assert!(!has_unannotated_sql(""));
    }

    /// A well-formed file (annotation first, statement after) is never
    /// flagged regardless of how many blocks it has.
    #[test]
    fn has_unannotated_sql_allows_well_formed_file() {
        let content = "-- name: GetUser :one\nSELECT * FROM users WHERE id = $1;\n\n-- name: ListUsers :many\nSELECT * FROM users;\n";
        assert!(!has_unannotated_sql(content));
    }

    /// The exact canonical scythe engine name (`postgresql`) must validate,
    /// not just sqruff's own `postgres` spelling -- #205's headline
    /// reproduction is this alias panicking.
    #[test]
    fn validate_dialect_accepts_canonical_scythe_engine_aliases() {
        assert_eq!(validate_dialect("postgresql").unwrap(), "postgres");
        assert_eq!(validate_dialect("pg").unwrap(), "postgres");
        assert_eq!(validate_dialect("mariadb").unwrap(), "mysql");
        assert_eq!(validate_dialect("sqlite3").unwrap(), "sqlite");
    }

    /// sqruff-native dialect names not present in scythe's own engine alias
    /// table (e.g. `bigquery`) must still validate, case-insensitively.
    #[test]
    fn validate_dialect_accepts_native_sqruff_dialect_names() {
        assert_eq!(validate_dialect("bigquery").unwrap(), "bigquery");
        assert_eq!(validate_dialect("ANSI").unwrap(), "ansi");
        assert_eq!(validate_dialect("Postgres").unwrap(), "postgres");
    }

    /// Gibberish must be rejected with a message naming the accepted values,
    /// never passed through to sqruff-lib where `DialectKind::from_str(..).unwrap()`
    /// panics (exit 101).
    #[test]
    fn validate_dialect_rejects_unknown_values_with_accepted_list() {
        let err = validate_dialect("klingon").unwrap_err();
        assert!(err.contains("klingon"));
        assert!(err.contains("postgres"));
        assert!(err.contains("ansi"));
    }

    #[test]
    fn config_dir_normalizes_bare_filename_to_dot() {
        assert_eq!(config_dir("scythe.toml"), Path::new("."));
        assert_eq!(config_dir("./scythe.toml"), Path::new("."));
    }

    #[test]
    fn config_dir_keeps_a_real_parent() {
        assert_eq!(config_dir("project/scythe.toml"), Path::new("project"));
    }

    /// #207: an absolute `output` always escapes, regardless of `base_dir`.
    #[test]
    fn output_escapes_base_rejects_absolute_paths() {
        assert!(output_escapes_base(Path::new("/tmp/ABSOLUTE")));
    }

    /// #207: `../../ESCAPED` pops past the join point immediately.
    #[test]
    fn output_escapes_base_rejects_parent_traversal() {
        assert!(output_escapes_base(Path::new("../../ESCAPED")));
    }

    /// `..` that stays balanced by a preceding normal component (e.g.
    /// `sub/../thing`) never leaves the project root and must be allowed.
    #[test]
    fn output_escapes_base_allows_balanced_parent_traversal() {
        assert!(!output_escapes_base(Path::new("sub/../thing")));
        assert!(!output_escapes_base(Path::new("generated")));
        assert!(!output_escapes_base(Path::new("./generated")));
    }

    #[test]
    fn resolve_contained_output_rejects_escape_by_default() {
        let err = resolve_contained_output(Path::new("proj"), "../../ESCAPED", "python-psycopg3", false).unwrap_err();
        assert!(err.contains("python-psycopg3"));
        assert!(err.contains("../../ESCAPED"));
        assert!(err.contains("--allow-output-escape"));
    }

    #[test]
    fn resolve_contained_output_allows_escape_with_opt_out() {
        let result = resolve_contained_output(Path::new("proj"), "../../ESCAPED", "python-psycopg3", true);
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_contained_output_allows_contained_paths() {
        let result = resolve_contained_output(Path::new("proj"), "generated", "rust-sqlx", false);
        assert_eq!(result.unwrap(), Path::new("proj/generated"));
    }
}
