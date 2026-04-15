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
