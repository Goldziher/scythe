use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct PythonValidator;

impl PythonValidator {
    /// Add `...` body to bare function/class signatures and fix indented fragments.
    fn patch_code(code: &str) -> String {
        let trimmed = code.trim();

        if trimmed.starts_with(' ') || trimmed.starts_with('\t') {
            let min_indent = trimmed
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.len() - l.trim_start().len())
                .min()
                .unwrap_or(0);
            if min_indent > 0 {
                let dedented: Vec<&str> = trimmed
                    .lines()
                    .map(|l| {
                        if l.trim().is_empty() {
                            ""
                        } else if l.len() > min_indent {
                            &l[min_indent..]
                        } else {
                            l.trim()
                        }
                    })
                    .collect();
                return Self::patch_signatures(&dedented.join("\n"));
            }
        }

        Self::patch_signatures(code)
    }

    /// Add `...` body to bare function/class signatures.
    fn patch_signatures(code: &str) -> String {
        let lines: Vec<&str> = code.lines().collect();
        let mut output = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            output.push(lines[i].to_string());

            let trimmed = lines[i].trim();
            let is_def_start =
                trimmed.starts_with("def ") || trimmed.starts_with("async def ") || trimmed.starts_with("class ");

            if is_def_start {
                let mut sig_end = i;
                let mut has_inline_body = false;
                while sig_end < lines.len() {
                    let t = lines[sig_end].trim();
                    if t.ends_with(':') {
                        break;
                    }
                    if let Some(arrow_pos) = t.find("->") {
                        let after_arrow = &t[arrow_pos + 2..];
                        if let Some(colon_pos) = after_arrow.find(':') {
                            let after_colon = after_arrow[colon_pos + 1..].trim();
                            if !after_colon.is_empty() {
                                has_inline_body = true;
                                break;
                            }
                            break;
                        }
                        let last = output.len() - 1;
                        if sig_end == i {
                            output[last] = format!("{}:", lines[sig_end]);
                        }
                        break;
                    }
                    if t.contains("): ") || t.contains("):\t") {
                        has_inline_body = true;
                        break;
                    }
                    if t.ends_with(')') && sig_end > i {
                        let last = output.len() - 1;
                        output[last] = format!("{}:", output[last]);
                        break;
                    }
                    if sig_end > i {
                        output.push(lines[sig_end].to_string());
                    }
                    sig_end += 1;
                }

                if sig_end >= lines.len() {
                    let last = output.len() - 1;
                    if !output[last].trim().ends_with(':') {
                        output[last] = format!("{}:", output[last]);
                    }
                    let indent = lines[i].chars().take_while(|c| c.is_whitespace()).count();
                    let body_indent = " ".repeat(indent + 4);
                    output.push(format!("{body_indent}..."));
                    i = sig_end;
                    continue;
                }

                if has_inline_body {
                    i = sig_end + 1;
                    continue;
                }

                let next_content = (sig_end + 1..lines.len())
                    .find(|&j| !lines[j].trim().is_empty())
                    .map(|j| lines[j]);

                let has_body = next_content.is_some_and(|l| l.starts_with(' ') || l.starts_with('\t'));

                if !has_body {
                    let indent = lines[i].chars().take_while(|c| c.is_whitespace()).count();
                    let body_indent = " ".repeat(indent + 4);
                    let last = output.len() - 1;
                    if !output[last].trim().ends_with(':') {
                        output[last] = format!("{}:", output[last]);
                    }
                    output.push(format!("{body_indent}..."));
                }

                i = sig_end + 1;
                continue;
            }
            i += 1;
        }

        output.join("\n")
    }
}

impl SnippetValidator for PythonValidator {
    fn language(&self) -> Language {
        Language::Python
    }

    fn is_available(&self) -> bool {
        which::which("python3").is_ok() || which::which("python").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let code = Self::patch_code(&snippet.code);
        let snippet_path = dir.path().join("snippet.py");
        std::fs::write(&snippet_path, &code)?;

        let python = if which::which("python3").is_ok() {
            "python3"
        } else {
            "python"
        };

        let path = snippet_path.to_string_lossy().to_string();

        let mut cmd = match level {
            ValidationLevel::Syntax => {
                let checker_path = dir.path().join("check.py");
                let checker = "\
import ast, sys
try:
    with open(sys.argv[1]) as f:
        ast.parse(f.read())
except SyntaxError as e:
    print(f\"{e}\", file=sys.stderr)
    sys.exit(1)
";
                std::fs::write(&checker_path, checker)?;

                let mut c = std::process::Command::new(python);
                c.args([checker_path.to_string_lossy().as_ref(), &path]);
                c
            }
            ValidationLevel::Compile => {
                let mut c = std::process::Command::new(python);
                c.args(["-m", "py_compile", &path]);
                c
            }
            ValidationLevel::Run => {
                let mut c = std::process::Command::new(python);
                c.arg(&path);
                c
            }
        };

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
        output.contains("unexpected indent") || output.contains("was never closed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patch_class_with_inline_methods() {
        let code = r#"class ExtractionResult:
    content: str
    mime_type: str
    metadata: Metadata
    tables: list[ExtractedTable]
    detected_languages: list[str] | None
    chunks: list[Chunk] | None
    images: list[ExtractedImage] | None
    pages: list[PageContent] | None
    elements: list[Element] | None
    djot_content: DjotContent | None
    output_format: str | None
    result_format: str | None
    def get_page_count(self) -> int: ...
    def get_chunk_count(self) -> int: ...
    def get_detected_language(self) -> str | None: ...
    def get_metadata_field(self, field_name: str) -> Any | None: ..."#;

        let patched = PythonValidator::patch_code(code);
        eprintln!("PATCHED OUTPUT:");
        for (i, line) in patched.lines().enumerate() {
            eprintln!("  {:3} | {}", i + 1, line);
        }
        assert_eq!(patched.trim(), code.trim());
    }
}
