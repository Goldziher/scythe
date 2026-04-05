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
