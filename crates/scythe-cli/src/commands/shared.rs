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

pub fn resolve_globs(patterns: &[String]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
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
}
