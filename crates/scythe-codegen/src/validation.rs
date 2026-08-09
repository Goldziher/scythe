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

/// Environment variable that turns on strict tool validation.
///
/// Set to `1` (or `true`) to make a missing tool a hard failure instead of a
/// skip. CI sets it after installing every checker; a developer's laptop
/// leaves it unset and still gets whatever checkers they happen to have.
pub const STRICT_ENV_VAR: &str = "SCYTHE_VALIDATE_STRICT";

/// Whether strict mode is on, read from [`STRICT_ENV_VAR`].
///
/// Deliberately an environment variable rather than a separate
/// `validate_with_tools_strict` entry point. The call sites that matter are
/// the generated-code assertions in `tests/tool_validation.rs`, and the
/// property we want is "CI checks every language, a laptop checks what it
/// can" -- with a second entry point every one of those call sites would have
/// to branch on the environment itself, which is precisely the per-call-site
/// policy decision that let the original `None`-is-a-pass bug spread to nine
/// places. One switch, read in one function, cannot drift.
pub fn strict_mode_enabled() -> bool {
    matches!(
        std::env::var(STRICT_ENV_VAR).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

/// What happened when one external checker was pointed at a generated file.
///
/// The distinction between these variants is the whole point of this type.
/// `Ran { errors: [] }` means a real tool inspected the code and found
/// nothing wrong; `Missing` means nothing was inspected at all. Collapsing
/// the two -- which is what returning `Option<Vec<String>>` did, since both
/// arrived at the call site as "no errors to report" -- turns an uninstalled
/// linter into a green check mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutcome {
    /// The tool was found and ran to completion. An empty `errors` genuinely
    /// means this checker is satisfied.
    Ran {
        /// Executable name, as it would be typed on a command line.
        tool: &'static str,
        /// Findings, already prefixed with the tool name.
        errors: Vec<String>,
    },
    /// The tool is not on `PATH`, so whatever it would have caught went
    /// unchecked. Never an error outside strict mode, never a pass inside it.
    Missing {
        /// Executable name that was looked for and not found.
        tool: &'static str,
    },
    /// The tool is installed but the harness could not run it -- a temp file
    /// that would not write, a process that would not spawn.
    ///
    /// This is neither a pass nor a skip: it is always an error, strict mode
    /// or not. The previous implementation reached these paths through `?` on
    /// an `Option` and returned `None`, i.e. reported a broken harness as an
    /// uninstalled tool, which then read as a pass.
    Failed {
        /// Executable name the harness was trying to drive.
        tool: &'static str,
        /// What went wrong, in the harness rather than in the checked code.
        reason: String,
    },
}

/// Every checker's verdict for one generated file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolValidation {
    /// No tool-based validator exists for this backend's language.
    ///
    /// Distinct from "the tools are missing": Java, C#, Elixir and Rust have
    /// no validator written for them at all, and reporting that as a skip
    /// would suggest installing something would help.
    Unsupported,
    /// A validator exists; one entry per checker it tried to run.
    Attempted(Vec<ToolOutcome>),
}

impl ToolValidation {
    /// Findings from the checkers that actually ran.
    pub fn errors(&self) -> Vec<&str> {
        match self {
            Self::Unsupported => vec![],
            Self::Attempted(outcomes) => outcomes
                .iter()
                .flat_map(|outcome| match outcome {
                    ToolOutcome::Ran { errors, .. } => errors.iter().map(String::as_str).collect::<Vec<_>>(),
                    ToolOutcome::Missing { .. } => vec![],
                    ToolOutcome::Failed { reason, .. } => vec![reason.as_str()],
                })
                .collect(),
        }
    }

    /// Checkers that were not installed.
    pub fn missing_tools(&self) -> Vec<&'static str> {
        match self {
            Self::Unsupported => vec![],
            Self::Attempted(outcomes) => outcomes
                .iter()
                .filter_map(|outcome| match outcome {
                    ToolOutcome::Missing { tool } => Some(*tool),
                    _ => None,
                })
                .collect(),
        }
    }

    /// Checkers that ran to completion.
    pub fn tools_run(&self) -> Vec<&'static str> {
        match self {
            Self::Unsupported => vec![],
            Self::Attempted(outcomes) => outcomes
                .iter()
                .filter_map(|outcome| match outcome {
                    ToolOutcome::Ran { tool, .. } => Some(*tool),
                    _ => None,
                })
                .collect(),
        }
    }

    /// Whether every checker this validator knows about actually ran.
    ///
    /// False for [`Self::Unsupported`]: nothing checked the code, so claiming
    /// it was fully checked would be the same lie in a different shape.
    pub fn fully_checked(&self) -> bool {
        match self {
            Self::Unsupported => false,
            Self::Attempted(outcomes) => {
                !outcomes.is_empty()
                    && outcomes
                        .iter()
                        .all(|outcome| matches!(outcome, ToolOutcome::Ran { .. }))
            }
        }
    }

    /// Collapse to pass/fail under the current [`strict_mode_enabled`] policy.
    ///
    /// Outside strict mode a missing tool is tolerated. Inside it, a missing
    /// tool is a failure -- that is what stops a checker from quietly falling
    /// out of CI and taking its coverage with it.
    pub fn into_result(self) -> Result<(), Vec<String>> {
        self.into_result_with_strictness(strict_mode_enabled())
    }

    /// [`Self::into_result`] with the policy passed explicitly, so tests can
    /// exercise both modes without mutating process-global environment state
    /// (which races under `cargo test`'s thread-per-test execution).
    ///
    /// [`Self::Unsupported`] is deliberately **not** a strict-mode failure.
    /// Strict mode exists to catch a checker that fell out of the CI image,
    /// and every one of its failures should be fixable by installing
    /// something. A language with no validator written for it is a real gap
    /// -- Java, C#, Elixir and Rust are all in it -- but no CI configuration
    /// closes it, so failing here would leave a permanently red build with no
    /// action attached. That gap is tracked by an explicit inventory test
    /// (`backends_with_no_tool_validator_are_a_known_and_shrinking_set` in
    /// `tests/tool_validation.rs`) which fails when the set changes in either
    /// direction, rather than by a signal nobody can act on.
    pub fn into_result_with_strictness(self, strict: bool) -> Result<(), Vec<String>> {
        let mut failures: Vec<String> = self.errors().into_iter().map(str::to_string).collect();

        if strict {
            for tool in self.missing_tools() {
                failures.push(format!(
                    "strict validation: `{tool}` is not installed, so nothing it checks was verified"
                ));
            }
        }

        if failures.is_empty() { Ok(()) } else { Err(failures) }
    }
}

/// A generated source file written to a temporary path, removed on drop.
///
/// `Drop` rather than a trailing `remove_file` call: every validator below
/// has early-return paths, and the previous hand-rolled cleanup was skipped
/// on all of them, leaking a temp file per failed validation.
struct TempSource {
    path: std::path::PathBuf,
}

impl TempSource {
    fn new(tool: &'static str, code: &str, ext: &str) -> Result<Self, ToolOutcome> {
        write_temp(code, ext)
            .map(|path| Self { path })
            .ok_or_else(|| ToolOutcome::Failed {
                tool,
                reason: format!("{tool}: could not write a temporary `{ext}` file"),
            })
    }

    fn arg(&self, tool: &'static str) -> Result<&str, ToolOutcome> {
        self.path.to_str().ok_or_else(|| ToolOutcome::Failed {
            tool,
            reason: format!("{tool}: temporary file path is not valid UTF-8"),
        })
    }
}

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether `tool` is on `PATH`, probed by running it with `probe_arg`.
fn tool_present(tool: &str, probe_arg: &str) -> bool {
    Command::new(tool).arg(probe_arg).output().is_ok()
}

/// Which of a process's streams a checker writes its diagnostics to.
#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
    /// Both, concatenated. `poly` splits its report across the two, so
    /// picking either one alone drops half the diagnostic.
    Both,
}

/// Run `tool` with `args` and turn a non-zero exit into findings.
///
/// `max_lines` caps how much of the diagnostic output is kept; a syntax error
/// can produce hundreds of cascading lines and only the first few locate it.
fn run_tool(tool: &'static str, args: &[&str], stream: Stream, max_lines: usize) -> Result<Vec<String>, ToolOutcome> {
    let output = Command::new(tool)
        .args(args)
        .output()
        .map_err(|error| ToolOutcome::Failed {
            tool,
            reason: format!("{tool}: could not be executed: {error}"),
        })?;

    if output.status.success() {
        return Ok(vec![]);
    }

    let raw = match stream {
        Stream::Stdout => String::from_utf8_lossy(&output.stdout),
        Stream::Stderr => String::from_utf8_lossy(&output.stderr),
        Stream::Both => std::borrow::Cow::Owned(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )),
    };

    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(max_lines)
        .map(|line| format!("{tool}: {line}"))
        .collect())
}

/// Run one checker end to end: probe, write the source, execute, collect.
fn check_with(
    tool: &'static str,
    probe_arg: &str,
    code: &str,
    ext: &str,
    build_args: impl Fn(&str) -> Vec<String>,
    stream: Stream,
    max_lines: usize,
) -> ToolOutcome {
    if !tool_present(tool, probe_arg) {
        return ToolOutcome::Missing { tool };
    }

    let source = match TempSource::new(tool, code, ext) {
        Ok(source) => source,
        Err(outcome) => return outcome,
    };
    let path = match source.arg(tool) {
        Ok(path) => path,
        Err(outcome) => return outcome,
    };

    let args = build_args(path);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match run_tool(tool, &args, stream, max_lines) {
        Ok(errors) => ToolOutcome::Ran { tool, errors },
        Err(outcome) => outcome,
    }
}

/// Validate generated code with the real compilers and linters for its
/// language.
///
/// Returns [`ToolValidation::Unsupported`] when no validator exists for the
/// backend's language, and a per-tool [`ToolOutcome`] list otherwise. Callers
/// must not treat "no errors" as "checked" without consulting
/// [`ToolValidation::fully_checked`] or [`ToolValidation::into_result`] --
/// see [`ToolOutcome`] for why.
pub fn validate_with_tools(code: &str, backend_name: &str) -> ToolValidation {
    let outcomes = match backend_name {
        name if name.starts_with("python") => validate_python_tools(code),
        name if name.starts_with("javascript") => validate_javascript_tools(code),
        name if name.starts_with("typescript") => validate_typescript_tools(code),
        name if name.starts_with("go") => validate_go_tools(code),
        name if name.starts_with("ruby") => validate_ruby_tools(code),
        name if name.starts_with("php") => validate_php_tools(code),
        // Kotlin has no validator: `poly` delegates Kotlin to `ktlint` rather
        // than bundling it, and standing up a JVM plus a downloaded jar in CI
        // to lint generated Kotlin is out of proportion to what it catches.
        // `validate_structural` still covers these backends, and the
        // inventory test in `tests/tool_validation.rs` keeps the gap visible.
        _ => return ToolValidation::Unsupported,
    };

    ToolValidation::Attempted(outcomes)
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

/// Run `poly` against a generated file.
///
/// `poly` is this repository's linter and formatter, and it bundles the
/// engines it needs in-process: `oxc` for JavaScript and TypeScript, `ruff`
/// for Python, `mago` for PHP. One already-required binary therefore replaces
/// three separately-installed ones -- `biome`, `ruff` and `php` -- and
/// because it parses as well as lints, it also subsumes the `python3 -m ast`
/// syntax pass.
///
/// `--no-workspace` is not optional. Without it `poly lint` also runs the
/// whole-project tier -- `cargo clippy` over this very workspace -- once per
/// generated file, turning a millisecond check into minutes.
///
/// `poly` exits non-zero only on error-severity findings, so warnings appear
/// in the message without failing the check. That is the same threshold the
/// repository's own `poly lint .` gate uses.
fn poly_check(code: &str, ext: &str) -> ToolOutcome {
    let config = generated_code_poly_config();
    check_with(
        "poly",
        "--version",
        code,
        ext,
        |path| {
            ["lint", "--no-workspace", "--config", &config.to_string_lossy(), path]
                .iter()
                .map(|arg| (*arg).to_string())
                .collect()
        },
        Stream::Both,
        6,
    )
}

/// Path to the poly config used for generated code.
///
/// Passed explicitly rather than left to discovery: poly resolves config by
/// walking up from the file it is given, and the file here is a temporary one
/// outside the repository, so poly would find nothing and fall back to its
/// built-in defaults. What CI enforced would then depend on where the system
/// temp directory happens to sit. See the config file's own header for why it
/// differs from the repository's `poly.toml`.
fn generated_code_poly_config() -> std::path::PathBuf {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/generated-code-poly.toml")).to_path_buf()
}

/// `poly`'s bundled `ruff`, which reports syntax errors and the `E`/`F`/`I`
/// rule families -- the same ground the previous `python3 -m ast` plus
/// standalone `ruff` pair covered, in one tool that needs no install step.
fn validate_python_tools(code: &str) -> Vec<ToolOutcome> {
    vec![poly_check(code, ".py")]
}

/// `poly`'s bundled `oxc`.
///
/// This was `biome check` until #98 installed biome and discovered the choice
/// had never been tested: `check` also runs the formatter, whose only
/// complaints about generated code are that scythe indents with spaces where
/// biome would use tabs, and where it would break a line. Generated-code
/// layout is the backends' contract and `scythe fmt` owns it, so running the
/// formatter here failed 8 TypeScript backends purely on indentation.
///
/// `poly` lints with `oxc` and does not impose a formatter, so it keeps what
/// is worth knowing -- unused bindings, unsafe casts, `useLiteralKeys` and the
/// rest of the correctness rules -- and drops a third-party install entirely.
fn validate_typescript_tools(code: &str) -> Vec<ToolOutcome> {
    vec![poly_check(code, ".ts")]
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
/// #81's verification requirement is specifically "runs under `node`, not
/// `tsx`, plus `tsc --checkJs --strict`", so both are reported as separate
/// [`ToolOutcome`]s: a machine with `node` but no `tsc` has verified that the
/// generated source parses, and verified nothing at all about whether its
/// JSDoc annotations typecheck.
///
/// Writes the temp file with a `.mjs` extension rather than `.js`: `node`'s
/// ESM-vs-CommonJS auto-detection for a bare `.js` file depends on Node
/// version and on whether some enclosing directory happens to have a
/// `package.json` with `"type": "module"` -- both outside this test's
/// control, and both irrelevant to what's actually being verified (that the
/// generated *source* is valid ESM). `.mjs` is an unambiguous, version-
/// independent signal that sidesteps that entirely.
fn validate_javascript_tools(code: &str) -> Vec<ToolOutcome> {
    // Real `node`, not `tsx`/`ts-node`/a build step: the generated file must
    // parse as plain ESM as-is. `--check` parses without executing,
    // mirroring the `ruby -c` / `php -l` precedent in this file.
    let parse = check_with(
        "node",
        "--version",
        code,
        ".mjs",
        |path| ["--check", path].iter().map(|arg| (*arg).to_string()).collect(),
        Stream::Stderr,
        5,
    );

    let stub = js_mode_driver_stub_path();
    let typecheck = check_with(
        "tsc",
        "--version",
        code,
        ".mjs",
        |path| {
            let mut args: Vec<String> = [
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
            ]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
            args.push(path.to_string());
            args.push(stub.to_string_lossy().into_owned());
            args
        },
        // tsc writes diagnostics to stdout by default.
        Stream::Stdout,
        10,
    );

    vec![parse, typecheck]
}

fn validate_go_tools(code: &str) -> Vec<ToolOutcome> {
    vec![check_with(
        "gofmt",
        "-h",
        code,
        ".go",
        |path| ["-e", path].iter().map(|arg| (*arg).to_string()).collect(),
        Stream::Stderr,
        3,
    )]
}

fn validate_ruby_tools(code: &str) -> Vec<ToolOutcome> {
    vec![check_with(
        "ruby",
        "--version",
        code,
        ".rb",
        |path| ["-c", path].iter().map(|arg| (*arg).to_string()).collect(),
        Stream::Stderr,
        1,
    )]
}

/// `poly`'s bundled `mago`, which parses PHP -- the same thing `php -l` did,
/// without needing a PHP runtime installed.
fn validate_php_tools(code: &str) -> Vec<ToolOutcome> {
    vec![poly_check(code, ".php")]
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

    /// The bug this whole type exists to prevent: before `ToolValidation`,
    /// a tool that was never installed and a tool that ran and found nothing
    /// both arrived at the call site as "no errors", so an uninstalled linter
    /// read as a green check mark.
    #[test]
    fn a_missing_tool_is_not_the_same_as_a_clean_run() {
        let clean = ToolValidation::Attempted(vec![ToolOutcome::Ran {
            tool: "ruff",
            errors: vec![],
        }]);
        let absent = ToolValidation::Attempted(vec![ToolOutcome::Missing { tool: "ruff" }]);

        // Both report zero findings -- that is exactly why the old
        // `Option<Vec<String>>` could not tell them apart.
        assert!(clean.errors().is_empty());
        assert!(absent.errors().is_empty());

        // The type still distinguishes them.
        assert!(clean.fully_checked(), "a tool that ran is a real check");
        assert!(!absent.fully_checked(), "a tool that never ran checked nothing");
        assert_eq!(absent.missing_tools(), vec!["ruff"]);
        assert!(clean.missing_tools().is_empty());
    }

    #[test]
    fn strict_mode_turns_a_missing_tool_into_a_failure() {
        let absent = ToolValidation::Attempted(vec![ToolOutcome::Missing { tool: "poly" }]);

        assert!(
            absent.clone().into_result_with_strictness(false).is_ok(),
            "outside strict mode a missing tool is tolerated"
        );

        let failures = absent
            .into_result_with_strictness(true)
            .expect_err("strict mode must fail on a missing tool");
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0].contains("poly") && failures[0].contains("not installed"),
            "the failure must name the tool and say it never ran: {failures:?}"
        );
    }

    /// A validator that runs two tools and finds only one of them has
    /// verified half of what it claims to. `validate_python_tools` shipped
    /// exactly this shape -- `python3` present, `ruff` absent, `Some([])`
    /// returned -- which is the strongest possible false signal.
    #[test]
    fn a_partially_run_validation_is_not_fully_checked() {
        let partial = ToolValidation::Attempted(vec![
            ToolOutcome::Ran {
                tool: "python3",
                errors: vec![],
            },
            ToolOutcome::Missing { tool: "ruff" },
        ]);

        assert!(partial.errors().is_empty());
        assert!(!partial.fully_checked(), "one of two tools running is not a full check");
        assert_eq!(partial.tools_run(), vec!["python3"]);
        assert_eq!(partial.missing_tools(), vec!["ruff"]);
        assert!(partial.into_result_with_strictness(true).is_err());
    }

    /// The languages `poly` bundles an engine for are checked by `poly` alone
    /// -- one binary, no per-language install. A regression that reintroduced
    /// a separate `ruff`/`biome`/`php` invocation would show up here as an
    /// extra outcome.
    #[test]
    fn poly_backed_languages_report_exactly_one_tool() {
        for (label, outcomes) in [
            ("python", validate_python_tools("x = 1\n")),
            ("typescript", validate_typescript_tools("export const x = 1;\n")),
            ("php", validate_php_tools("<?php\ndeclare(strict_types=1);\n")),
        ] {
            assert_eq!(outcomes.len(), 1, "{label} must be checked by poly alone");
            let tool = match &outcomes[0] {
                ToolOutcome::Ran { tool, .. } | ToolOutcome::Missing { tool } | ToolOutcome::Failed { tool, .. } => {
                    *tool
                }
            };
            assert_eq!(tool, "poly", "{label} must go through poly");
        }
    }

    /// A language with no validator must not masquerade as a clean pass
    /// either -- `_ => None` previously made Java, C#, Elixir and Rust
    /// unconditionally green.
    #[test]
    fn a_backend_with_no_validator_is_unsupported_not_clean() {
        let validation = validate_with_tools("public class Foo {}", "java-jdbc");

        assert_eq!(validation, ToolValidation::Unsupported);
        assert!(validation.errors().is_empty());
        assert!(!validation.fully_checked(), "no validator means nothing was checked");
        assert!(validation.clone().into_result_with_strictness(false).is_ok());
        assert!(
            validation.into_result_with_strictness(true).is_ok(),
            "strict mode is for tools that can be installed; a language with no \
             validator is tracked by an inventory test instead -- see \
             into_result_with_strictness"
        );
    }

    /// A harness that cannot run an installed tool is a failure in both
    /// modes. The old code reached these paths through `?` on an `Option`
    /// and reported them as "tool not installed", which then read as a pass.
    #[test]
    fn a_harness_failure_is_an_error_even_outside_strict_mode() {
        let broken = ToolValidation::Attempted(vec![ToolOutcome::Failed {
            tool: "gofmt",
            reason: "gofmt: could not be executed: permission denied".into(),
        }]);

        assert!(!broken.fully_checked());
        let failures = broken
            .into_result_with_strictness(false)
            .expect_err("a broken harness is never a pass");
        assert!(failures[0].contains("gofmt"));
    }

    #[test]
    fn strict_mode_reads_the_documented_environment_variable() {
        // Guard: the constant and the docs must not drift apart.
        assert_eq!(STRICT_ENV_VAR, "SCYTHE_VALIDATE_STRICT");
    }
}
