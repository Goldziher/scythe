use crate::lint::sqruff_adapter;

/// Run the `fmt` command: format SQL files using sqruff.
///
/// - If `files` is non-empty, format those files directly.
/// - If `files` is empty, read query file paths from the scythe config.
/// - `check_only`: report what would change without modifying files (exit 1 if changes needed).
/// - `diff`: show a unified diff of changes.
/// - Otherwise: write formatted SQL back to files.
pub fn run_fmt(
    config_path: &str,
    check_only: bool,
    diff: bool,
    dialect: Option<&str>,
    files: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let dialect = dialect.unwrap_or("ansi");

    let file_paths = if files.is_empty() {
        resolve_files_from_config(config_path)?
    } else {
        files.to_vec()
    };

    if file_paths.is_empty() {
        eprintln!("No SQL files found to format.");
        return Ok(());
    }

    let mut needs_formatting = false;

    for path in &file_paths {
        let original = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read '{}': {}", path, e))?;

        let formatted = sqruff_adapter::format_sql(&original, dialect)
            .map_err(|e| format!("sqruff error on '{}': {}", path, e))?;

        if original == formatted {
            continue;
        }

        needs_formatting = true;

        if check_only {
            eprintln!("{} needs formatting", path);
        } else if diff {
            print_diff(path, &original, &formatted);
        } else {
            std::fs::write(path, &formatted)
                .map_err(|e| format!("failed to write '{}': {}", path, e))?;
            eprintln!("formatted {}", path);
        }
    }

    if check_only && needs_formatting {
        return Err("Some files need formatting.".into());
    }

    if !check_only && !diff && !needs_formatting {
        eprintln!("All files already formatted.");
    }

    Ok(())
}

/// Resolve SQL query files from the scythe config (reads `sql[*].queries` globs).
fn resolve_files_from_config(config_path: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct MinConfig {
        sql: Vec<MinSqlConfig>,
    }

    #[derive(Deserialize)]
    struct MinSqlConfig {
        queries: Vec<String>,
        #[serde(default)]
        schema: Vec<String>,
    }

    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: MinConfig = toml::from_str(&config_str)
        .map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let mut all_files = Vec::new();
    for sql_config in &config.sql {
        // Include both query files and schema files
        for patterns in [&sql_config.queries, &sql_config.schema] {
            for pattern in patterns {
                let matches: Vec<_> = glob::glob(pattern)?.collect::<Result<Vec<_>, _>>()?;
                for m in matches {
                    all_files.push(m.display().to_string());
                }
            }
        }
    }

    Ok(all_files)
}

/// Print a simple unified diff between original and formatted content.
fn print_diff(path: &str, original: &str, formatted: &str) {
    eprintln!("--- {}", path);
    eprintln!("+++ {} (formatted)", path);

    let orig_lines: Vec<&str> = original.lines().collect();
    let fmt_lines: Vec<&str> = formatted.lines().collect();

    // Simple line-by-line diff (not a full unified diff algorithm, but useful enough)
    let max_lines = orig_lines.len().max(fmt_lines.len());
    let mut in_hunk = false;
    for i in 0..max_lines {
        let orig = orig_lines.get(i).copied().unwrap_or("");
        let fmt = fmt_lines.get(i).copied().unwrap_or("");
        if orig != fmt {
            if !in_hunk {
                eprintln!("@@ line {} @@", i + 1);
                in_hunk = true;
            }
            if i < orig_lines.len() {
                eprintln!("-{}", orig);
            }
            if i < fmt_lines.len() {
                eprintln!("+{}", fmt);
            }
        } else {
            in_hunk = false;
        }
    }
}
