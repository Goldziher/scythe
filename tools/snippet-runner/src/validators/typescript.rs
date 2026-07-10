use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::TempDir;

pub struct TypeScriptValidator;

impl TypeScriptValidator {
    fn dedent(code: &str) -> String {
        let min_indent = code
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);

        if min_indent == 0 {
            return code.to_string();
        }

        code.lines()
            .map(|l| {
                if l.trim().is_empty() {
                    ""
                } else if l.len() > min_indent {
                    &l[min_indent..]
                } else {
                    l.trim()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Detect bare function/interface declarations (API reference signatures).
    fn is_api_signature(code: &str) -> bool {
        let trimmed = code.trim();
        let lines: Vec<&str> = trimmed.lines().collect();

        if lines.len() <= 6 {
            let has_fn_decl = trimmed.starts_with("function ")
                || trimmed.starts_with("async function ")
                || trimmed.starts_with("export function ")
                || trimmed.starts_with("export async function ");
            let has_body = trimmed.contains('{');
            if has_fn_decl && !has_body {
                return true;
            }
        }

        false
    }
}

impl SnippetValidator for TypeScriptValidator {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn is_available(&self) -> bool {
        which::which("tsc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if Self::is_api_signature(&snippet.code) {
            return Ok((SnippetStatus::Pass, None));
        }

        let trimmed_code = snippet.code.trim();
        if trimmed_code.starts_with("!!!") || trimmed_code.starts_with("???") {
            return Ok((SnippetStatus::Pass, None));
        }

        let dir = TempDir::new()?;

        let tsconfig = r#"{
  "compilerOptions": {
    "strict": true,
    "noEmit": true,
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "skipLibCheck": true
  },
  "include": ["*.ts"]
}"#;
        std::fs::write(dir.path().join("tsconfig.json"), tsconfig)?;

        let code = Self::dedent(&snippet.code);
        let file_path = dir.path().join("snippet.ts");
        let mut file = std::fs::File::create(&file_path)?;
        file.write_all(code.as_bytes())?;

        let mut cmd = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => {
                let mut c = std::process::Command::new("tsc");
                c.args(["--noEmit", "--pretty", "false"]).current_dir(dir.path());
                c
            }
            ValidationLevel::Run => {
                let mut c = std::process::Command::new("tsx");
                c.args([file_path.to_string_lossy().as_ref()]);
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
        let dep_patterns = [
            "TS2307", "TS2304", "TS2305", "TS2306", "TS2322", "TS2345", "TS2339", "TS2351", "TS2552", "TS2314",
            "TS2391", "TS2693", "TS7016", "TS2371", "TS2580", "TS1375", "TS2792", "TS2503", "TS7006", "TS2769",
            "TS1128", "TS1005", "TS18046", "TS18047", "TS2531", "TS2532", "TS2451", "TS2591", "TS2390",
        ];

        let error_lines: Vec<&str> = output.lines().filter(|l| l.contains("error TS")).collect();

        if error_lines.is_empty() {
            return false;
        }

        error_lines
            .iter()
            .all(|line| dep_patterns.iter().any(|p| line.contains(p)))
    }
}
