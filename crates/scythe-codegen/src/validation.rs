use std::process::Command;

/// Validate generated code structurally for a given backend.
/// Returns a list of errors (empty = passed).
pub fn validate_structural(code: &str, backend_name: &str) -> Vec<String> {
    match backend_name {
        "python-psycopg3" | "python-asyncpg" | "python-aiomysql" | "python-aiosqlite" | "python-duckdb"
        | "python-pyodbc" | "python-oracledb" | "python-snowflake" => validate_python(code),
        "typescript-postgres"
        | "typescript-pg"
        | "typescript-mysql2"
        | "typescript-better-sqlite3"
        | "typescript-node-sqlite"
        | "typescript-wasm-sqlite"
        | "typescript-duckdb"
        | "typescript-kysely"
        | "typescript-mssql"
        | "typescript-oracledb"
        | "typescript-snowflake" => validate_typescript(code),
        "javascript-pg" | "javascript-postgres" | "javascript-mysql2" | "javascript-better-sqlite3" => {
            validate_javascript(code)
        }
        "go-pgx" | "go-database-sql" | "go-godror" | "go-gosnowflake" => validate_go(code),
        "java-jdbc" => validate_java(code),
        "java-r2dbc" => validate_java_r2dbc(code),
        "kotlin-exposed" => validate_kotlin_exposed(code),
        "kotlin-jdbc" => validate_kotlin(code),
        "kotlin-r2dbc" => validate_kotlin_r2dbc(code),
        "csharp-npgsql"
        | "csharp-mysqlconnector"
        | "csharp-microsoft-sqlite"
        | "csharp-sqlclient"
        | "csharp-oracle"
        | "csharp-snowflake" => validate_csharp(code),
        "elixir-postgrex" | "elixir-ecto" | "elixir-myxql" | "elixir-exqlite" | "elixir-tds" | "elixir-jamdb" => {
            validate_elixir(code)
        }
        "ruby-pg" | "ruby-mysql2" | "ruby-sqlite3" | "ruby-trilogy" | "ruby-oci8" | "ruby-tiny-tds" => {
            validate_ruby(code)
        }
        "php-pdo" | "php-amphp" => validate_php(code),
        "rust-sqlx" | "rust-tokio-postgres" | "rust-tiberius" | "rust-sibyl" => vec![],
        _ => vec![format!("unknown backend: {}", backend_name)],
    }
}

fn validate_python(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    if code.contains("from __future__ import annotations") {
        errors.push("unnecessary `from __future__ import annotations` — target is Python 3.10+".into());
    }

    let has_struct = code.contains("@dataclass")
        || code.contains("(BaseModel)")
        || code.contains("(msgspec.Struct)")
        || code.contains("class ");
    if !has_struct && !code.contains("async def ") && !code.contains("def ") {
        errors.push("missing `@dataclass`/`class` and `def ` -- no meaningful output".into());
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

    for (i, line) in code.lines().enumerate() {
        if line.starts_with('\t') {
            errors.push(format!("line {} uses tab indentation (should use 4 spaces)", i + 1));
            break;
        }
    }

    errors
}

/// Structurally validate the `javascript-*` (JSDoc emit mode, #81) backends.
///
/// Unlike [`validate_typescript`], this rejects the TypeScript-only
/// declaration forms those backends' generated `.ts` output legitimately
/// uses (`export interface`, `export enum`) -- a plain `.js` file can never
/// contain them, so their presence here means a `js_mode` branch fell
/// through to the TypeScript path by mistake.
fn validate_javascript(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_function = code.contains("export async function") || code.contains("export function");
    let has_typedef = code.contains("@typedef");
    if !has_typedef && !has_function {
        errors.push("missing `@typedef` (for DTOs) and `export function`/`export async function`".into());
    }
    if !has_function {
        errors.push("missing `export async function` or `export function`".into());
    }
    if code.contains("export interface") {
        errors.push("contains `export interface` -- TypeScript-only syntax, invalid in plain .js".into());
    }
    if code.contains("export enum") {
        errors.push("contains `export enum` -- TypeScript-only syntax, invalid in plain .js".into());
    }

    for line in code.lines() {
        let trimmed = line.trim();
        if find_disallowed_any_usage(trimmed).is_some() {
            errors.push(format!(
                "contains `any` type (should use `unknown` or specific): {}",
                trimmed
            ));
            break;
        }
    }

    errors
}

fn validate_typescript(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_function = code.contains("export async function") || code.contains("export function");

    let has_zod = code.contains("z.object(") || code.contains("z.infer<");
    if !code.contains("export interface") && !code.contains("export type") && !has_zod && !has_function {
        errors.push("missing `export interface` or `export type` (for DTOs)".into());
    }

    if !has_function {
        errors.push("missing `export async function` or `export function`".into());
    }

    for line in code.lines() {
        let trimmed = line.trim();
        if find_disallowed_any_usage(trimmed).is_some() {
            errors.push(format!(
                "contains `any` type (should use `unknown` or specific): {}",
                trimmed
            ));
            break;
        }
    }

    errors
}

/// Scan a line of generated TypeScript for a standalone `any` type token.
///
/// This is token-aware (matches `any` at identifier boundaries) rather than
/// the previous fixed set of punctuation suffixes (`: any`, `<any>`, `any;`,
/// `any,`, `any)`), which happened to omit `any>` and let the Kysely
/// backend's `<DB = any>` generic-default slip through undetected rather
/// than deliberately allowed.
///
/// The Kysely `<DB = any>` idiom is explicitly allowlisted here: `DB` is a
/// real generic type parameter threaded through `Kysely<DB>` in the
/// function signature, not an escape hatch. A caller who supplies
/// `Kysely<MyDB>` gets full column typing; the `= any` default only lets
/// callers who don't care about DB-shape typing omit the type argument.
fn find_disallowed_any_usage(trimmed: &str) -> Option<&str> {
    let mut offset = 0;
    while let Some(rel) = trimmed[offset..].find("any") {
        let start = offset + rel;
        let end = start + "any".len();
        let before_is_ident = start > 0 && is_ident_byte(trimmed.as_bytes()[start - 1]);
        let after_is_ident = end < trimmed.len() && is_ident_byte(trimmed.as_bytes()[end]);
        if !before_is_ident && !after_is_ident && !is_kysely_db_generic_default(trimmed, start, end) {
            return Some(trimmed);
        }
        offset = end;
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True when the `any` token spanning `[start, end)` in `line` is the
/// default of a `<DB = any>` generic type parameter, e.g.
/// `export async function foo<DB = any>(db: Kysely<DB>): ... {`.
fn is_kysely_db_generic_default(line: &str, start: usize, end: usize) -> bool {
    line[..start].ends_with("<DB = ") && line[end..].starts_with('>')
}

fn validate_go(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_func = code.contains("func ");
    let has_struct = code.contains("type ") && code.contains("struct {");

    if !has_struct && !has_func {
        errors.push("missing `type ... struct {` (for structs)".into());
    }

    if !has_func {
        errors.push("missing `func ` (for functions)".into());
    }

    if !code.contains("context.Context") {
        errors.push("missing `context.Context` as first param".into());
    }

    let has_indented_lines = code.lines().any(|l| l.starts_with('\t') || l.starts_with("  "));
    if has_indented_lines {
        let uses_spaces = code.lines().any(|l| l.starts_with("    ") && !l.trim().is_empty());
        if uses_spaces {
            errors.push("uses space indentation (Go standard is tabs)".into());
        }
    }

    if has_struct && !code.contains("json:\"") {
        errors.push("missing `json:\"` tags on struct fields".into());
    }

    errors
}

fn validate_java(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_static = code.contains("public static ");

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

fn validate_kotlin_exposed(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_fun = code.contains("fun ");

    if !code.contains("data class ") && !code.contains("object ") && !has_fun {
        errors.push("missing `data class ` or `object ` (for DTOs/Tables)".into());
    }

    if !has_fun {
        errors.push("missing `fun ` (for functions)".into());
    }

    if !code.contains("transaction {") {
        errors.push("missing `transaction {` (Exposed transaction block)".into());
    }

    errors
}

fn validate_java_r2dbc(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_static = code.contains("public static ");

    if !code.contains("public record ") && !has_static {
        errors.push("missing `public record ` (for DTOs)".into());
    }

    if !has_static {
        errors.push("missing `public static ` (for query methods)".into());
    }

    if !code.contains("Mono<") && !code.contains("Flux<") {
        errors.push("missing `Mono<` or `Flux<` (reactive types)".into());
    }

    if !code.contains("ConnectionFactory") {
        errors.push("missing `ConnectionFactory` parameter".into());
    }

    errors
}

fn validate_kotlin_r2dbc(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_fun = code.contains("fun ");

    if !code.contains("data class ") && !has_fun {
        errors.push("missing `data class ` (for DTOs)".into());
    }

    if !has_fun {
        errors.push("missing `fun ` (for functions)".into());
    }

    if !code.contains("ConnectionFactory") {
        errors.push("missing `ConnectionFactory` parameter".into());
    }

    if !code.contains("suspend fun") && !code.contains("Flow<") {
        errors.push("missing `suspend fun` or `Flow<` (coroutine/reactive types)".into());
    }

    errors
}

fn validate_csharp(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_async = code.contains("async Task<") || code.contains("async Task ");

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

    if !code.contains("defmodule ") && !has_def {
        errors.push("missing `defmodule ` (for modules)".into());
    }

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

    if !code.contains("Data.define") && !has_method {
        errors.push("missing `Data.define` (for DTOs)".into());
    }

    if !has_method {
        errors.push("missing `def self.` (for module methods)".into());
    }

    if !code.contains("# frozen_string_literal: true") {
        errors.push("missing `# frozen_string_literal: true`".into());
    }

    if !code.contains("module Queries") {
        errors.push("missing `module Queries` wrapper".into());
    }

    errors
}

fn validate_php(code: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let has_function = code.contains("function ");

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

/// Validate generated code using real language tools (if available).
/// Returns None if the tool is not installed, Some(errors) otherwise.
pub fn validate_with_tools(code: &str, backend_name: &str) -> Option<Vec<String>> {
    match backend_name {
        name if name.starts_with("python") => validate_python_tools(code),
        name if name.starts_with("javascript") => validate_javascript_tools(code),
        name if name.starts_with("typescript") => validate_typescript_tools(code),
        name if name.starts_with("go") => validate_go_tools(code),
        name if name.starts_with("ruby") => validate_ruby_tools(code),
        name if name.starts_with("php") => validate_php_tools(code),
        name if name.starts_with("kotlin") => validate_kotlin_tools(code),
        _ => None,
    }
}

fn write_temp(code: &str, ext: &str) -> Option<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let basename = if ext == ".kt" {
        format!("ScytheValidate{n}")
    } else {
        format!("scythe_validate_{n}")
    };
    let path = std::env::temp_dir().join(format!("{basename}{ext}"));
    let trimmed = format!("{}\n", code.trim_end());
    std::fs::write(&path, trimmed).ok()?;
    Some(path)
}

fn validate_python_tools(code: &str) -> Option<Vec<String>> {
    if Command::new("python3").arg("--version").output().is_err() {
        return None;
    }
    let path = write_temp(code, ".py")?;
    let mut errors = vec![];

    let out = Command::new("python3")
        .args(["-c", &format!("import ast; ast.parse(open({:?}).read())", path)])
        .output()
        .ok()?;
    if !out.status.success() {
        errors.push(format!(
            "python syntax: {}",
            String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("")
        ));
    }

    if Command::new("ruff").arg("--version").output().is_ok() {
        let out = Command::new("ruff")
            .args([
                "check",
                "--select",
                "E,F,I",
                "--target-version",
                "py310",
                path.to_str()?,
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines().take(3) {
                if !line.trim().is_empty() {
                    errors.push(format!("ruff: {line}"));
                }
            }
        }
    }

    let _ = std::fs::remove_file(&path);
    Some(errors)
}

fn validate_typescript_tools(code: &str) -> Option<Vec<String>> {
    if Command::new("biome").arg("--version").output().is_err() {
        return None;
    }
    let path = write_temp(code, ".ts")?;
    let mut errors = vec![];

    let out = Command::new("biome")
        .args(["check", "--no-errors-on-unmatched", path.to_str()?])
        .output()
        .ok()?;
    if !out.status.success() {
        for line in String::from_utf8_lossy(&out.stderr).lines().take(3) {
            if !line.trim().is_empty() {
                errors.push(format!("biome: {line}"));
            }
        }
    }

    let _ = std::fs::remove_file(&path);
    Some(errors)
}

/// Path to the hand-written ambient `.d.ts` stubs for the driver packages
/// (`pg`, `postgres`, `mysql2/promise`, `better-sqlite3`) the
/// `javascript-*` (JSDoc emit mode, #81) backends reference via
/// `import("pkg").Type` JSDoc annotations.
///
/// See that file's own header comment for why these are hand-written
/// stand-ins rather than the real `@types/*` packages (no `npm install` at
/// test time -- this must stay hermetic and offline-safe).
fn js_mode_driver_stub_path() -> std::path::PathBuf {
    std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/js_mode_stubs/driver-stubs.d.ts"
    ))
    .to_path_buf()
}

/// Validate generated `javascript-*` (JSDoc emit mode, #81) code against the
/// real `node` runtime and, when available, real `tsc --checkJs --strict`.
///
/// Unlike every other `validate_*_tools` function in this file, this one
/// `eprintln!`s which tool it found or is skipping, and why. #81's
/// verification requirement is specifically "runs under `node`, not `tsx`,
/// plus `tsc --checkJs --strict`" -- the silent `None` this file's other
/// validators return when a tool is missing is easy to mistake for
/// "checked, and clean" instead of "not checked at all", which is exactly
/// the verification gap #81's review called out. A caller that only inspects
/// `Some(errors).is_empty()` and ignores a `None` return risks exactly that
/// mistake, so this also prints loudly enough that `cargo test -- --nocapture`
/// makes the skip impossible to miss.
///
/// Writes the temp file with a `.mjs` extension rather than `.js`: `node`'s
/// ESM-vs-CommonJS auto-detection for a bare `.js` file depends on Node
/// version and on whether some enclosing directory happens to have a
/// `package.json` with `"type": "module"` -- both outside this test's
/// control, and both irrelevant to what's actually being verified (that the
/// generated *source* is valid ESM). `.mjs` is an unambiguous, version-
/// independent signal that sidesteps that entirely.
fn validate_javascript_tools(code: &str) -> Option<Vec<String>> {
    let Ok(node_version) = Command::new("node").arg("--version").output() else {
        eprintln!("  SKIP javascript tool validation: `node` not found on PATH -- nothing was checked with any tool");
        return None;
    };
    eprintln!(
        "  RUN javascript tool validation: node {} found on PATH",
        String::from_utf8_lossy(&node_version.stdout).trim()
    );

    let path = write_temp(code, ".mjs")?;
    let mut errors = vec![];

    // Real `node`, not `tsx`/`ts-node`/a build step: the generated file must
    // parse as plain ESM as-is. `--check` parses without executing,
    // mirroring this file's `ruby -c` / `php -l` precedent above.
    let out = Command::new("node").args(["--check", path.to_str()?]).output().ok()?;
    if !out.status.success() {
        for line in String::from_utf8_lossy(&out.stderr).lines().take(5) {
            if !line.trim().is_empty() {
                errors.push(format!("node --check: {line}"));
            }
        }
    }

    if let Ok(tsc_version) = Command::new("tsc").arg("--version").output() {
        eprintln!(
            "  RUN tsc --checkJs --strict: {} found on PATH",
            String::from_utf8_lossy(&tsc_version.stdout).trim()
        );
        let stub = js_mode_driver_stub_path();
        let out = Command::new("tsc")
            .args([
                "--checkJs",
                "--strict",
                "--allowJs",
                "--noEmit",
                "--module",
                "nodenext",
                "--moduleResolution",
                "nodenext",
                "--target",
                "es2022",
            ])
            .arg(path.to_str()?)
            .arg(stub.to_str()?)
            .output()
            .ok()?;
        if !out.status.success() {
            // tsc writes diagnostics to stdout by default; stderr is
            // included too so a crash/usage error is not silently dropped.
            let mut lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            lines.extend(String::from_utf8_lossy(&out.stderr).lines().map(str::to_string));
            for line in lines.into_iter().take(10) {
                if !line.trim().is_empty() {
                    errors.push(format!("tsc: {line}"));
                }
            }
        }
    } else {
        eprintln!("  SKIP tsc --checkJs --strict: `tsc` not found on PATH -- only `node --check` ran");
    }

    let _ = std::fs::remove_file(&path);
    Some(errors)
}

fn validate_go_tools(code: &str) -> Option<Vec<String>> {
    if Command::new("gofmt").arg("-h").output().is_err() {
        return None;
    }
    let path = write_temp(code, ".go")?;
    let mut errors = vec![];

    let out = Command::new("gofmt").args(["-e", path.to_str()?]).output().ok()?;
    if !out.status.success() {
        for line in String::from_utf8_lossy(&out.stderr).lines().take(3) {
            if !line.trim().is_empty() {
                errors.push(format!("gofmt: {line}"));
            }
        }
    }

    let _ = std::fs::remove_file(&path);
    Some(errors)
}

fn validate_ruby_tools(code: &str) -> Option<Vec<String>> {
    if Command::new("ruby").arg("--version").output().is_err() {
        return None;
    }
    let path = write_temp(code, ".rb")?;
    let mut errors = vec![];

    let out = Command::new("ruby").args(["-c", path.to_str()?]).output().ok()?;
    if !out.status.success() {
        errors.push(format!(
            "ruby syntax: {}",
            String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("")
        ));
    }

    let _ = std::fs::remove_file(&path);
    Some(errors)
}

fn validate_php_tools(code: &str) -> Option<Vec<String>> {
    if Command::new("php").arg("--version").output().is_err() {
        return None;
    }
    let path = write_temp(code, ".php")?;
    let mut errors = vec![];

    let out = Command::new("php").args(["-l", path.to_str()?]).output().ok()?;
    if !out.status.success() {
        errors.push(format!(
            "php syntax: {}",
            String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("")
        ));
    }

    let _ = std::fs::remove_file(&path);
    Some(errors)
}

fn validate_kotlin_tools(code: &str) -> Option<Vec<String>> {
    if Command::new("ktlint").arg("--version").output().is_err() {
        return None;
    }
    let path = write_temp(code, ".kt")?;
    let mut errors = vec![];

    let out = Command::new("ktlint")
        .args(["--log-level=error", path.to_str()?])
        .output()
        .ok()?;
    if !out.status.success() {
        for line in String::from_utf8_lossy(&out.stdout).lines().take(3) {
            if !line.trim().is_empty() {
                errors.push(format!("ktlint: {line}"));
            }
        }
    }

    let _ = std::fs::remove_file(&path);
    Some(errors)
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
        let code = r#"from dataclasses import dataclass

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
        let code = r#"from typing import Optional

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
    fn test_typescript_allows_kysely_db_generic_default() {
        let code = r#"export async function listUsers<DB = any>(db: Kysely<DB>): Promise<ListUsersRow[]> {
  return [];
}
"#;
        let errors = validate_structural(code, "typescript-kysely");
        assert!(
            errors.is_empty(),
            "the `<DB = any>` generic default must be allowlisted, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_typescript_rejects_any_before_closing_angle_bracket() {
        // Regression test: the old suffix-list check (`: any`, `<any>`,
        // `any;`, `any,`, `any)`) never looked for `any` immediately before
        // `>`, so `Array<any>` (or any other non-Kysely-DB `any>` usage)
        // slipped through undetected.
        let code = r#"export async function listUsers(): Promise<Array<any>> {
  return [];
}
"#;
        let errors = validate_structural(code, "typescript-postgres");
        assert!(
            errors.iter().any(|e| e.contains("any")),
            "expected an `any` violation, got: {:?}",
            errors
        );
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
