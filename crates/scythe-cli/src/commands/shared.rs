use std::borrow::Cow;
use std::path::Path;

/// Splits a .sql file containing multiple queries separated by `-- name:` or
/// `-- @name` annotations. Returns one string per query block (annotation +
/// SQL). Content before the first annotation is discarded.
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

/// Try to read the SQL dialect from a scythe.toml config file.
/// Returns None if the config doesn't exist or can't be parsed.
pub fn dialect_from_config(config_path: &str) -> Option<String> {
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

    let config_str = std::fs::read_to_string(config_path).ok()?;
    let config: MinConfig = toml::from_str(&config_str).ok()?;
    config
        .sql
        .first()
        .and_then(|s| s.engine.as_deref())
        .map(|e| engine_to_sqruff_dialect(e).to_string())
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
pub fn config_dir(config_path: &str) -> &Path {
    Path::new(config_path).parent().unwrap_or_else(|| Path::new("."))
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
fn rebase_pattern<'a>(pattern: &'a str, base_dir: &Path) -> Cow<'a, str> {
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
}
