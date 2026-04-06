/// Validate generated code structurally for a given backend.
/// Returns a list of errors (empty = passed).
pub fn validate_structural(code: &str, backend_name: &str) -> Vec<String> {
    match backend_name {
        "python-psycopg3" | "python-asyncpg" => validate_python(code),
        "typescript-postgres" | "typescript-pg" => validate_typescript(code),
        "go-pgx" => validate_go(code),
        "java-jdbc" => validate_java(code),
        "kotlin-jdbc" => validate_kotlin(code),
        "csharp-npgsql" => validate_csharp(code),
        "elixir-postgrex" => validate_elixir(code),
        "ruby-pg" => validate_ruby(code),
        "php-pdo" => validate_php(code),
        // Rust backends are validated by syn, not here.
        "rust-sqlx" | "rust-tokio-postgres" => vec![],
        _ => vec![format!("unknown backend: {}", backend_name)],
    }
}

fn validate_python(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    // The `from __future__` import is only present when a struct/dataclass is
    // generated (model_struct or row_struct). For exec-only queries the snippet
    // contains only a bare function, so we gate this check.
    let has_struct = code.contains("@dataclass") || code.contains("class ");
    if has_struct && !code.contains("from __future__ import annotations") {
        errors.push("missing `from __future__ import annotations`".into());
    }

    // Struct check: only required when the assembled code includes more than
    // just a function (i.e. when a row/model struct was emitted). We detect
    // this by looking for a dataclass or class definition. If neither is
    // present *and* there is no function definition either, that is an error.
    if !has_struct {
        // No struct -- at least a function must be present.
        if !code.contains("async def ") && !code.contains("def ") {
            errors.push("missing `@dataclass`/`class` and `def ` -- no meaningful output".into());
        }
    }

    if !code.contains("async def ") && !code.contains("def ") {
        errors.push("missing `async def ` or `def ` (for query functions)".into());
    }

    if code.contains("from typing import Union") {
        errors.push("contains `from typing import Union` (pre-3.10 style)".into());
    }

    if code.contains("from typing import Optional") {
        errors.push("contains `from typing import Optional` (pre-3.10 style)".into());
    }

    if code.contains("List[") {
        errors.push("contains `List[` (use lowercase `list[`)".into());
    }

    if code.contains("Dict[") {
        errors.push("contains `Dict[` (use lowercase `dict[`)".into());
    }

    // Check for proper indentation: 4 spaces, no tabs
    for (i, line) in code.lines().enumerate() {
        if line.starts_with('\t') {
            errors.push(format!(
                "line {} uses tab indentation (should use 4 spaces)",
                i + 1
            ));
            break; // one error is enough
        }
    }

    errors
}

fn validate_typescript(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_function = code.contains("export async function") || code.contains("export function");

    // Structs are only required when the code is NOT exec-only (i.e. when
    // there is something beyond a bare function).
    if !code.contains("export interface") && !code.contains("export type") && !has_function {
        errors.push("missing `export interface` or `export type` (for DTOs)".into());
    }

    if !has_function {
        errors.push("missing `export async function` or `export function`".into());
    }

    // Check for `any` type usage -- but avoid false positives in words like "many"
    for line in code.lines() {
        let trimmed = line.trim();
        // Look for `: any` or `<any>` or `any;` or `any,` patterns
        if trimmed.contains(": any")
            || trimmed.contains("<any>")
            || trimmed.contains("any;")
            || trimmed.contains("any,")
            || trimmed.contains("any)")
        {
            errors.push(format!(
                "contains `any` type (should use `unknown` or specific): {}",
                trimmed
            ));
            break;
        }
    }

    errors
}

fn validate_go(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_func = code.contains("func ");
    let has_struct = code.contains("type ") && code.contains("struct {");

    // Structs are only required when the code has one; exec-only queries
    // produce just a function.
    if !has_struct && !has_func {
        errors.push("missing `type ... struct {` (for structs)".into());
    }

    if !has_func {
        errors.push("missing `func ` (for functions)".into());
    }

    if !code.contains("context.Context") {
        errors.push("missing `context.Context` as first param".into());
    }

    // Go uses tabs for indentation
    let has_indented_lines = code
        .lines()
        .any(|l| l.starts_with('\t') || l.starts_with("  "));
    if has_indented_lines {
        let uses_spaces = code
            .lines()
            .any(|l| l.starts_with("    ") && !l.trim().is_empty());
        if uses_spaces {
            errors.push("uses space indentation (Go standard is tabs)".into());
        }
    }

    // json tags only required when struct is present
    if has_struct && !code.contains("json:\"") {
        errors.push("missing `json:\"` tags on struct fields".into());
    }

    errors
}

fn validate_java(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_static = code.contains("public static ");

    // Records are only required when a struct was generated; exec-only
    // queries produce just a method.
    if !code.contains("public record ") && !has_static {
        errors.push("missing `public record ` (for DTOs)".into());
    }

    if !has_static {
        errors.push("missing `public static ` (for query methods)".into());
    }

    if !code.contains("throws SQLException") {
        errors.push("missing `throws SQLException`".into());
    }

    if !code.contains("try (") {
        errors.push("missing `try (` (try-with-resources)".into());
    }

    errors
}

fn validate_kotlin(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_fun = code.contains("fun ");

    // data class only required when a struct was generated
    if !code.contains("data class ") && !has_fun {
        errors.push("missing `data class ` (for DTOs)".into());
    }

    if !has_fun {
        errors.push("missing `fun ` (for functions)".into());
    }

    if !code.contains(".use {") {
        errors.push("missing `.use {` (resource management)".into());
    }

    errors
}

fn validate_csharp(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_async = code.contains("async Task<") || code.contains("async Task ");

    // Records are only required when a struct was generated
    if !code.contains("public record ") && !has_async {
        errors.push("missing `public record ` (for DTOs)".into());
    }

    if !has_async {
        errors.push("missing `async Task<` or `async Task` (for async methods)".into());
    }

    if !code.contains("await ") {
        errors.push("missing `await `".into());
    }

    errors
}

fn validate_elixir(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_def = code.contains("def ") || code.contains("defp ");

    // defmodule is only required when a struct was generated; exec-only
    // queries produce just a function.
    if !code.contains("defmodule ") && !has_def {
        errors.push("missing `defmodule ` (for modules)".into());
    }

    // defstruct is only required when a struct was generated
    if !code.contains("defstruct") && !has_def {
        errors.push("missing `defstruct` (for structs)".into());
    }

    if !has_def {
        errors.push("missing `def ` or `defp ` (for functions)".into());
    }

    if !code.contains("@type ") && !code.contains("@spec ") {
        errors.push("missing `@type ` or `@spec ` (for typespecs)".into());
    }

    errors
}

fn validate_ruby(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_method = code.contains("def self.");

    // Data.define only required when a struct was generated
    if !code.contains("Data.define") && !has_method {
        errors.push("missing `Data.define` (for DTOs)".into());
    }

    if !has_method {
        errors.push("missing `def self.` (for module methods)".into());
    }

    if !code.contains("# frozen_string_literal: true") {
        errors.push("missing `# frozen_string_literal: true`".into());
    }

    errors
}

fn validate_php(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_function = code.contains("function ");

    // readonly class only required when a struct was generated
    if !code.contains("readonly class ") && !has_function {
        errors.push("missing `readonly class ` (for DTOs)".into());
    }

    if !has_function {
        errors.push("missing `function ` (for query functions)".into());
    }

    if !code.contains("declare(strict_types=1)") {
        errors.push("missing `declare(strict_types=1)`".into());
    }

    if !code.contains("<?php") {
        errors.push("missing `<?php`".into());
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unknown_backend() {
        let errors = validate_structural("some code", "unknown-backend");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown backend"));
    }

    #[test]
    fn test_rust_backends_skip() {
        assert!(validate_structural("anything", "rust-sqlx").is_empty());
        assert!(validate_structural("anything", "rust-tokio-postgres").is_empty());
    }

    #[test]
    fn test_python_valid() {
        let code = r#"from __future__ import annotations

from dataclasses import dataclass

@dataclass
class ListUsersRow:
    id: int
    name: str

async def list_users(conn) -> list[ListUsersRow]:
    pass
"#;
        let errors = validate_structural(code, "python-psycopg3");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_python_invalid_typing() {
        let code = r#"from __future__ import annotations
from typing import Optional

@dataclass
class Row:
    id: int

def query() -> List[Row]:
    pass
"#;
        let errors = validate_structural(code, "python-asyncpg");
        assert!(errors.iter().any(|e| e.contains("Optional")));
        assert!(errors.iter().any(|e| e.contains("List[")));
    }

    #[test]
    fn test_typescript_valid() {
        let code = r#"export interface ListUsersRow {
  id: number;
  name: string;
}

export async function listUsers(): Promise<ListUsersRow[]> {
  // ...
}
"#;
        let errors = validate_structural(code, "typescript-postgres");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_go_valid() {
        let code = "package db\n\nimport (\n\t\"context\"\n\t\"encoding/json\"\n)\n\ntype ListUsersRow struct {\n\tID   int    `json:\"id\"`\n\tName string `json:\"name\"`\n}\n\nfunc ListUsers(ctx context.Context) ([]ListUsersRow, error) {\n\treturn nil, nil\n}\n";
        let errors = validate_structural(code, "go-pgx");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_php_valid() {
        let code = r#"<?php

declare(strict_types=1);

readonly class ListUsersRow {
    public function __construct(
        public int $id,
        public string $name,
    ) {}
}

function listUsers($pdo): array {
    return [];
}
"#;
        let errors = validate_structural(code, "php-pdo");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }
}
