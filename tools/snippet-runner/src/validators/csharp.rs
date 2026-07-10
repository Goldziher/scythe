use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::TempDir;

pub struct CSharpValidator;

impl SnippetValidator for CSharpValidator {
    fn language(&self) -> Language {
        Language::CSharp
    }

    fn is_available(&self) -> bool {
        which::which("dotnet").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;

        let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>
</Project>"#;
        std::fs::write(dir.path().join("snippet.csproj"), csproj)?;

        let mut file = std::fs::File::create(dir.path().join("Program.cs"))?;
        file.write_all(snippet.code.as_bytes())?;

        let mut cmd = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => {
                let mut c = std::process::Command::new("dotnet");
                c.args(["build", "--nologo", "-v", "quiet"]).current_dir(dir.path());
                c
            }
            ValidationLevel::Run => {
                let mut c = std::process::Command::new("dotnet");
                c.args(["run", "--nologo"]).current_dir(dir.path());
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
        let error_lines: Vec<&str> = output
            .lines()
            .filter(|l| l.contains("error CS") || l.contains("error MSB"))
            .collect();

        if error_lines.is_empty() {
            return output.contains("error CS5001") || output.contains("error CS0106");
        }

        let dep_patterns = [
            "CS0246", "CS0103", "CS0234", "CS0106", "CS0116", "CS8802", "CS8803", "CS0029", "CS1002", "CS1513",
            "CS5001", "CS1003", "CS1529", "CS0101", "CS0161", "CS1001", "CS0501", "CS0535",
        ];

        error_lines
            .iter()
            .all(|line| dep_patterns.iter().any(|p| line.contains(p)))
    }
}
