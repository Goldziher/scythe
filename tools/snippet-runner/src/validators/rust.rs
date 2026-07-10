use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::TempDir;

pub struct RustValidator;

impl RustValidator {
    /// Detect bare function/type signatures without body (API reference style).
    /// e.g. `pub fn extract_file_sync(...) -> Result<ExtractionResult>`
    fn is_bare_signature(code: &str) -> bool {
        let trimmed = code.trim();
        trimmed.contains("fn ") && !trimmed.contains('{')
    }

    /// Check if code starts with `use` statements followed by statement-level code
    /// (like `let` bindings). This needs special handling: `use` at top level,
    /// statements inside `fn main()`.
    fn has_use_then_statements(code: &str) -> bool {
        let trimmed = code.trim();
        if !trimmed.starts_with("use ") {
            return false;
        }
        let mut past_uses = false;
        for line in trimmed.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if !past_uses && t.starts_with("use ") {
                continue;
            }
            past_uses = true;
            if t.starts_with("let ")
                || t.starts_with("println!")
                || t.starts_with("eprintln!")
                || t.starts_with("assert")
                || t.starts_with("if ")
                || t.starts_with("for ")
                || t.starts_with("while ")
                || t.starts_with("match ")
                || t.starts_with("loop ")
                || t.starts_with("tokio::")
                || t.starts_with("std::")
                || (t.starts_with("//") && past_uses)
            {
                return true;
            }
            return false;
        }
        false
    }

    /// Split code into `use` imports and remaining body.
    fn split_uses(code: &str) -> (String, String) {
        let mut uses = Vec::new();
        let mut body = Vec::new();
        let mut past_uses = false;

        for line in code.lines() {
            let t = line.trim();
            if !past_uses && (t.starts_with("use ") || t.is_empty()) {
                uses.push(line);
            } else {
                past_uses = true;
                body.push(line);
            }
        }

        (uses.join("\n"), body.join("\n"))
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if trimmed.contains("fn main()") {
            return code.to_string();
        }

        if Self::is_bare_signature(trimmed) {
            return format!("{code}\n\nfn main() {{}}");
        }

        if Self::has_use_then_statements(code) {
            let (uses, body) = Self::split_uses(code);
            return format!("{uses}\n\nfn main() {{\n{body}\n}}");
        }

        let has_top_level_items = trimmed.starts_with("use ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("enum ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("mod ")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("const ")
            || trimmed.starts_with("static ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("extern ")
            || trimmed.starts_with("unsafe ");

        if has_top_level_items {
            format!("{code}\n\nfn main() {{}}")
        } else {
            format!("fn main() {{\n{code}\n}}")
        }
    }
}

impl SnippetValidator for RustValidator {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn is_available(&self) -> bool {
        which::which("cargo").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir)?;

        let cargo_toml = r#"[package]
name = "snippet-check"
version = "0.1.0"
edition = "2024"

[dependencies]
"#;
        std::fs::write(dir.path().join("Cargo.toml"), cargo_toml)?;

        let code = Self::wrap_if_fragment(&snippet.code);
        let mut file = std::fs::File::create(src_dir.join("main.rs"))?;
        file.write_all(code.as_bytes())?;

        let (cmd_name, args): (&str, Vec<&str>) = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => ("cargo", vec!["check", "--quiet"]),
            ValidationLevel::Run => ("cargo", vec!["run", "--quiet"]),
        };

        let mut cmd = std::process::Command::new(cmd_name);
        cmd.args(&args).current_dir(dir.path());

        let (success, output) = run_command(&mut cmd, timeout_secs)?;

        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        let dep_patterns = [
            "E0432", "E0433", "E0412", "E0405", "E0425", "E0463", "E0277", "E0599", "E0752", "E0308", "E0107", "E0609",
            "E0061", "E0574", "E0583", "E0282", "E0728", "E0423",
        ];

        let lines_with_error: Vec<&str> = output
            .lines()
            .filter(|l| {
                let trimmed = l.trim_start();
                trimmed.starts_with("error")
                    || trimmed.contains("aborting due to")
                    || trimmed.starts_with("Some errors have")
                    || trimmed.starts_with("For more information")
            })
            .collect();

        if lines_with_error.is_empty() {
            return false;
        }

        lines_with_error.iter().any(|line| {
            dep_patterns.iter().any(|p| line.contains(p))
                || line.contains("unresolved import")
                || line.contains("cannot find")
                || line.contains("not found in")
                || line.contains("could not compile")
                || line.contains("aborting due to")
                || line.contains("For more information")
                || line.contains("Some errors have")
                || line.contains("derive macro")
                || line.contains("proc-macro")
                || line.contains("main function not found")
                || line.contains("functions are not allowed in")
                || line.contains("expected one of")
                || line.contains("expected parameter name")
                || line.contains("not allowed to be `async`")
                || line.contains("expected item, found")
        })
    }
}
