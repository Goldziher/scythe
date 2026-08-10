use scythe_lint::sqruff_adapter;
use scythe_lint::types::SqruffConfig;

use super::shared::{config_dir, dialect_from_config, engine_to_sqruff_dialect, resolve_globs, validate_dialect};

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
    let (file_paths, config_dialect, sqruff_config) = if files.is_empty() {
        let resolved = resolve_files_from_config(config_path)?;
        (resolved.files, resolved.dialect, resolved.sqruff_config)
    } else {
        let config_dialect = dialect_from_config(config_path)?;
        (files.to_vec(), config_dialect, None)
    };

    // `--dialect` is validated before it can reach sqruff-lib, which panics
    // (exit 101) on a value `DialectKind::from_str` does not recognize --
    // see #205. `config_dialect` is already a sqruff-native name (produced
    // by `engine_to_sqruff_dialect`), so it does not need re-validation.
    let dialect = match dialect {
        Some(raw) => validate_dialect(raw)?,
        None => config_dialect.unwrap_or_else(|| "ansi".to_string()),
    };

    if file_paths.is_empty() {
        eprintln!("No SQL files found to format.");
        return Ok(());
    }

    // `[lint.sqruff]` is validated once, up front, exactly like `scythe
    // lint` does (see `lint_from_config`'s `validate_config` call) -- #206:
    // before this, `fmt` accepted a `[lint.sqruff]` that `lint` rejects as
    // an invalid configuration, and silently ignored `[lint.sqruff.rules]`
    // (e.g. a disabled rule, or an excluded rule) entirely.
    sqruff_adapter::validate_config(&dialect, sqruff_config.as_ref())
        .map_err(|e| format!("invalid [lint.sqruff] configuration: {}", e))?;

    let mut needs_formatting = false;

    for path in &file_paths {
        let original = std::fs::read_to_string(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

        let formatted = sqruff_adapter::format_sql(&original, &dialect, sqruff_config.as_ref())
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
            std::fs::write(path, &formatted).map_err(|e| format!("failed to write '{}': {}", path, e))?;
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

/// Resolved files, dialect, and `[lint.sqruff]` config from the scythe config.
struct ResolvedConfig {
    files: Vec<String>,
    /// The sqruff dialect derived from the first sql block's engine.
    dialect: Option<String>,
    /// `[lint.sqruff]`, shared with `scythe lint`'s own config struct.
    sqruff_config: Option<SqruffConfig>,
}

/// Resolve SQL query files and dialect from the scythe config.
fn resolve_files_from_config(config_path: &str) -> Result<ResolvedConfig, Box<dyn std::error::Error>> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct MinConfig {
        sql: Vec<MinSqlConfig>,
        #[serde(default)]
        lint: Option<scythe_lint::types::LintConfig>,
    }

    #[derive(Deserialize)]
    struct MinSqlConfig {
        /// Falls back to the block's array index (matching every other
        /// command's `[sql[N]]` label) when a block predates the required
        /// `name` field or is otherwise malformed enough that this narrow,
        /// partial-projection struct still deserializes it.
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        engine: Option<String>,
        queries: Vec<String>,
        #[serde(default)]
        schema: Vec<String>,
    }

    let config_str =
        std::fs::read_to_string(config_path).map_err(|e| format!("failed to read config '{}': {}", config_path, e))?;
    let config: MinConfig =
        toml::from_str(&config_str).map_err(|e| format!("failed to parse config '{}': {}", config_path, e))?;

    let dialect = config
        .sql
        .first()
        .and_then(|s| s.engine.as_deref())
        .map(|e| engine_to_sqruff_dialect(e).to_string());

    let sqruff_config = config.lint.and_then(|lc| lc.sqruff);

    let base_dir = config_dir(config_path);

    let mut all_files = Vec::new();
    for (idx, sql_config) in config.sql.iter().enumerate() {
        // Uses the block's own `name`, like `audit`/`check`/`lint`'s
        // `[{sql_config.name}] ...` labels, instead of the positional
        // `[sql[{idx}]]` every other command replaced years ago -- #212.
        let label = sql_config.name.clone().unwrap_or_else(|| format!("sql[{idx}]"));
        all_files.extend(resolve_globs(
            &sql_config.queries,
            base_dir,
            &format!("[{label}] queries"),
        )?);
        all_files.extend(resolve_globs(
            &sql_config.schema,
            base_dir,
            &format!("[{label}] schema"),
        )?);
    }

    Ok(ResolvedConfig {
        files: all_files,
        dialect,
        sqruff_config,
    })
}

/// Print a simple unified diff between original and formatted content.
fn print_diff(path: &str, original: &str, formatted: &str) {
    eprintln!("--- {}", path);
    eprintln!("+++ {} (formatted)", path);

    let orig_lines: Vec<&str> = original.lines().collect();
    let fmt_lines: Vec<&str> = formatted.lines().collect();

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
