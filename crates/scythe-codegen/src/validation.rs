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
        // Both Java backends compile against real `javac`. `java-jdbc` needs
        // only the two-annotation JSR-305 stub (see
        // `java_annotation_stub_paths`); `java-r2dbc` additionally needs
        // `io.r2dbc.spi` and Reactor's `Mono`/`Flux`, stubbed in
        // `tests/java_stubs/` -- see `java_r2dbc_stub_paths` for why that is
        // worth doing despite the generic surface involved.
        "java-jdbc" => validate_java_tools(code),
        "java-r2dbc" => validate_java_r2dbc_tools(code),
        // Only the JDBC backend: `kotlin-exposed` needs the Exposed DSL
        // framework (`transaction { }`, `exec(sql, args) { rs -> }`, and a
        // `*ColumnType` per scalar), and `kotlin-r2dbc` needs
        // `kotlinx.coroutines.flow.Flow` plus the `awaitFirst`/`asFlow`
        // suspend bridges, whose stubs would have to reproduce the coroutines
        // compiler plugin's view of `suspend`. `kotlin-jdbc` itself touches
        // nothing but `java.sql`/`java.math`/`java.time`, so `kotlinc` alone
        // (no extra classpath) resolves it.
        "kotlin-jdbc" => validate_kotlin_tools(code),
        // Every `elixir-*` backend, not just one: unlike Java/Kotlin/C#,
        // Elixir does not resolve a remote *function* call at compile time --
        // `Postgrex.query/3` when `Postgrex` was never compiled is a
        // *warning* ("module is not available or is yet to be defined"), not
        // an error, and `elixirc` still exits 0. A struct reference is the
        // exception (verified against real `elixirc` output, not assumed):
        // `%Mod{...}` expands `__struct__/1` at compile time regardless, so
        // the two backends that construct or match one against an external
        // driver's struct (`elixir-myxql`, `elixir-tds`) need the small stubs
        // in `tests/elixir_stubs/` -- see `validate_elixir_tools`.
        name if name.starts_with("elixir") => validate_elixir_tools(code),
        // C#: every `csharp-*` backend references a NuGet-only driver
        // (Npgsql, MySqlConnector, Microsoft.Data.Sqlite, Microsoft.Data.
        // SqlClient, Oracle.ManagedDataAccess.Core, Snowflake.Data.Client).
        // `using Npgsql;` with no compiled `Npgsql.dll` on the reference path
        // is a hard `CS0246`, unlike Elixir's soft warning above, so there is
        // no stub-free path through `dotnet build`/`csc` here, and six
        // different driver APIs is too much surface to hand-stub credibly
        // (unlike the two-interface JSR-305 case above). `validate_structural`
        // still covers these backends.
        //
        // Rust: `rust-sqlx` expands `sqlx::query_as!`/`sqlx::query!` at
        // compile time, which needs either a live database connection or an
        // `SQLX_OFFLINE` `.sqlx` query cache -- there is no way to satisfy
        // that from a bare generated file. `rust-tokio-postgres`,
        // `rust-tiberius` and `rust-sibyl` reference their driver crates by
        // fully-qualified path (`tokio_postgres::Row`, etc.) with no `use`
        // to stub around; making `rustc --emit=metadata` resolve them needs
        // `--extern` pointing at real compiled `.rlib`s, i.e. standing up a
        // full Cargo dependency graph per backend -- disproportionate to a
        // single-file lightweight check when every one of these backends'
        // generated code is already syntax-checked by `syn::parse_file` in
        // `crates/scythe-cli/tests/compile_check.rs` and the generated test
        // suite.
        _ => return ToolValidation::Unsupported,
    };

    ToolValidation::Attempted(outcomes)
}

/// A validation result for one backend's generated output, shaped for a
/// caller (the CLI, in particular) to report directly to a user rather than
/// for a test assertion to pick apart.
///
/// Three variants, not two, on purpose: collapsing [`Self::Unsupported`] into
/// [`Self::Passed`] is the exact dishonesty [`ToolValidation`] exists to
/// prevent -- see that type's doc comment. A caller that only distinguished
/// pass from fail would report "OK" for a `csharp-npgsql` file that no
/// compiler ever looked at, which is the bug this whole module was written to
/// close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// No tool-based validator exists for this backend's language at all
    /// (see [`ToolValidation::Unsupported`]). Not a pass and not a failure --
    /// report it as "not checked", never as "checked, no problems".
    Unsupported,
    /// A validator exists and every checker that ran found nothing wrong.
    ///
    /// `tools_run` names the checkers that actually inspected the code;
    /// `tools_missing` names any this validator would also have liked to run
    /// but could not find on `PATH`. Both are carried through rather than
    /// collapsed, because "passed, nothing was installed to check it"
    /// (`tools_run` empty) and "passed, `poly` verified it" are different
    /// claims a caller may want to report differently even though neither is
    /// a build failure under the default, non-strict policy this is built
    /// on (see [`ToolValidation::into_result_with_strictness`]).
    Passed {
        /// Checkers that ran to completion and found nothing wrong.
        tools_run: Vec<&'static str>,
        /// Checkers this validator wanted to run but could not find.
        tools_missing: Vec<&'static str>,
    },
    /// A checker found a problem with the generated code, or the harness
    /// itself could not drive an installed tool. Never produced by a
    /// merely-missing tool -- see [`ToolOutcome::Missing`] vs.
    /// [`ToolOutcome::Failed`], which is exactly the distinction that keeps
    /// an uninstalled linter from reading as a build failure here.
    Failed {
        /// Checkers that ran to completion (regardless of whether they were
        /// the one that reported a problem).
        tools_run: Vec<&'static str>,
        /// Checkers this validator wanted to run but could not find.
        tools_missing: Vec<&'static str>,
        /// Findings, already prefixed with the tool name that reported each
        /// one -- see [`ToolValidation::errors`].
        errors: Vec<String>,
    },
}

impl ValidationOutcome {
    /// Whether this outcome should stop a build: `true` only for
    /// [`Self::Failed`].
    ///
    /// Both [`Self::Unsupported`] and [`Self::Passed`] are "let it through"
    /// outcomes under the default policy -- a caller that wants "no
    /// validator for this language" to also block a build (unusual: it would
    /// stop every C#, Rust, `kotlin-exposed`, and `kotlin-r2dbc` backend from
    /// ever generating) must match on the variant itself rather than use
    /// this helper.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Validate one backend's generated output with the real compilers and
/// linters for its target language, collapsed to the three outcomes a
/// caller needs to decide what to print and whether to fail a build.
///
/// This is [`validate_with_tools`] built on top of, not reimplemented: the
/// pass/fail split follows the same non-strict policy as
/// [`ToolValidation::into_result`] outside [`STRICT_ENV_VAR`] -- a tool that
/// is not installed is tolerated, never a build failure. Keeping the collapse
/// here, delegating to [`ToolValidation::into_result_with_strictness`] rather
/// than re-deriving "missing is fine, a real finding is not" a second time,
/// is what keeps that policy defined in exactly one place; see the doc
/// comment on [`ToolValidation::into_result_with_strictness`] for why
/// [`ValidationOutcome::Unsupported`] is exempt from strictness entirely
/// rather than becoming a fourth, stricter failure case.
///
/// # This shells out to real external tools
///
/// `poly`, `tsc`, `node`, `gofmt`, `javac`, `kotlinc`, `elixirc`, `ruby`, and
/// others, depending on `backend_name` -- see [`validate_with_tools`]'s match
/// arms. When a tool is not installed, probing for it
/// (`Command::new(tool).arg(probe_arg).output()`) fails fast: the OS reports
/// "no such file" as soon as it tries to spawn the process, so an absent
/// tool costs microseconds and never blocks. The cost that matters is the
/// opposite case -- a tool that *is* installed. `javac` and `kotlinc` each
/// pay JVM startup (commonly over a second) on top of a real compile;
/// `tsc --checkJs --strict` runs a full TypeScript typecheck. A caller that
/// invokes this unconditionally on every `scythe generate` would make every
/// run pay that cost for every backend the file targets, on any machine that
/// happens to have the tool installed -- and would silently validate less on
/// a machine that doesn't, with no way for the user to tell which happened
/// short of reading [`ValidationOutcome::Passed`]'s `tools_run`/
/// `tools_missing` fields. **Callers should gate this behind an explicit
/// opt-in (e.g. a `--verify` flag), not run it unconditionally on every
/// generate.**
#[must_use]
pub fn validate_generated_code(code: &str, backend_name: &str) -> ValidationOutcome {
    let validation = validate_with_tools(code, backend_name);
    if matches!(validation, ToolValidation::Unsupported) {
        return ValidationOutcome::Unsupported;
    }

    let tools_run = validation.tools_run();
    let tools_missing = validation.missing_tools();

    match validation.into_result_with_strictness(false) {
        Ok(()) => ValidationOutcome::Passed {
            tools_run,
            tools_missing,
        },
        Err(errors) => ValidationOutcome::Failed {
            tools_run,
            tools_missing,
            errors,
        },
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

/// A directory a checker writes build artifacts into, removed on drop.
///
/// `javac -d`, `kotlinc -d` and `elixirc -o` all require a writable output
/// directory and happily leave `.class`/`.beam` files behind in it -- unlike
/// `poly`, `gofmt`, `ruby -c` and `node --check`, none of which write
/// anything to disk. Without this, running these checkers would litter
/// whatever directory `cargo test` happens to run from. Mirrors
/// [`TempSource`]'s cleanup guarantee, and is deliberately its own per-call
/// directory (not a fixed shared path) so parallel test threads cannot race
/// on each other's output.
struct TempOutDir {
    path: std::path::PathBuf,
}

impl TempOutDir {
    fn new(tool: &'static str) -> Result<Self, ToolOutcome> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("scythe_validate_out_{tool}_{n}"));
        std::fs::create_dir_all(&path).map_err(|error| ToolOutcome::Failed {
            tool,
            reason: format!("{tool}: could not create output directory: {error}"),
        })?;
        Ok(Self { path })
    }

    fn arg(&self, tool: &'static str) -> Result<&str, ToolOutcome> {
        self.path.to_str().ok_or_else(|| ToolOutcome::Failed {
            tool,
            reason: format!("{tool}: temporary output directory path is not valid UTF-8"),
        })
    }
}

impl Drop for TempOutDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Like [`check_with`], but for compilers that must be pointed at a writable
/// output directory. See [`TempOutDir`] for why that can't just be `check_with`
/// with an extra flag baked into `build_args`.
fn check_with_output(
    tool: &'static str,
    probe_arg: &str,
    code: &str,
    ext: &str,
    build_args: impl Fn(&str, &str) -> Vec<String>,
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
    let source_path = match source.arg(tool) {
        Ok(path) => path,
        Err(outcome) => return outcome,
    };

    let out_dir = match TempOutDir::new(tool) {
        Ok(dir) => dir,
        Err(outcome) => return outcome,
    };
    let out_path = match out_dir.arg(tool) {
        Ok(path) => path,
        Err(outcome) => return outcome,
    };

    let args = build_args(source_path, out_path);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match run_tool(tool, &args, stream, max_lines) {
        Ok(errors) => ToolOutcome::Ran { tool, errors },
        Err(outcome) => outcome,
    }
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

/// Paths to the hand-written JSR-305 `@Nonnull`/`@Nullable` stub sources
/// `javac` needs to resolve `java-jdbc`'s nullability annotations. See
/// `tests/java_stubs/javax/annotation/Nonnull.java` for why these are
/// hand-written stand-ins rather than the real `com.google.code.findbugs:
/// jsr305` jar -- same reasoning as [`js_mode_driver_stub_path`], one
/// directory over.
fn java_annotation_stub_paths() -> Result<(String, String), ToolOutcome> {
    const TOOL: &str = "javac";
    let dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/java_stubs/javax/annotation"
    ));
    let nonnull = dir.join("Nonnull.java");
    let nullable = dir.join("Nullable.java");
    match (nonnull.to_str(), nullable.to_str()) {
        (Some(a), Some(b)) => Ok((a.to_string(), b.to_string())),
        _ => Err(ToolOutcome::Failed {
            tool: TOOL,
            reason: format!("{TOOL}: stub annotation path is not valid UTF-8"),
        }),
    }
}

/// A `java-jdbc` source file, written to `<unique dir>/Queries.java`.
///
/// `javac` rejects a `public class Queries { ... }` unless the file it lives
/// in is literally named `Queries.java` -- and `java-jdbc`'s `file_header`
/// (`src/backends/java_jdbc.rs`) always wraps its output in exactly that
/// class, for every engine. The generic [`TempSource`]/`write_temp` naming
/// (`scythe_validate_<n>.java`) would therefore always fail to compile
/// regardless of what the generated code itself says, so this exists
/// alongside it rather than reusing it. A unique per-call directory, not a
/// fixed name in the shared system temp dir, is what keeps this collision-
/// safe under parallel `cargo test` execution.
struct JavaSource {
    dir: std::path::PathBuf,
}

impl JavaSource {
    fn new(code: &str) -> Result<Self, ToolOutcome> {
        const TOOL: &str = "javac";
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("scythe_validate_java_{n}"));
        std::fs::create_dir_all(&dir).map_err(|error| ToolOutcome::Failed {
            tool: TOOL,
            reason: format!("{TOOL}: could not create source directory: {error}"),
        })?;
        let trimmed = format!("{}\n", code.trim_end());
        std::fs::write(dir.join("Queries.java"), trimmed).map_err(|error| ToolOutcome::Failed {
            tool: TOOL,
            reason: format!("{TOOL}: could not write Queries.java: {error}"),
        })?;
        Ok(Self { dir })
    }

    fn source_arg(&self) -> Result<String, ToolOutcome> {
        self.dir
            .join("Queries.java")
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| ToolOutcome::Failed {
                tool: "javac",
                reason: "javac: temporary source path is not valid UTF-8".to_string(),
            })
    }
}

impl Drop for JavaSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Validate generated `java-jdbc` code against the real `javac` compiler.
///
/// `java-jdbc` needs nothing beyond the JDK standard library plus the
/// two-annotation JSR-305 stub from [`java_annotation_stub_paths`], so this
/// compiles real generated code with no network access and no extra classpath
/// setup. `java-r2dbc` goes through [`validate_java_r2dbc_tools`], which adds
/// the R2DBC SPI and Reactor stubs it needs on top.
fn validate_java_tools(code: &str) -> Vec<ToolOutcome> {
    javac_with_stubs(code, &[])
}

/// The R2DBC SPI + Reactive Streams + Reactor stub sources `java-r2dbc` needs
/// on top of the JSR-305 annotations, in `tests/java_stubs/`.
///
/// These exist because `java-r2dbc`'s output is almost entirely *inferred*:
/// `Mono.usingWhen(...).flatMap(result -> Mono.from(result.map((row, meta) ->
/// new GetUserRow(...))))` never spells its own element type, so the row
/// mapping -- the part the reader defects behind #191/#213/#214 lived in -- is
/// checked by nothing at all unless `Mono`, `Flux`, `Result`, and `Row`
/// resolve. Every generic bound in the stubs is copied verbatim from Reactor 3
/// and R2DBC SPI 1.0 rather than loosened, so a file these accept is one the
/// real libraries accept; that is verified by mutation, not assumed (see the
/// header comment on `tests/java_stubs/reactor/core/publisher/Mono.java`).
fn java_r2dbc_stub_paths() -> Result<Vec<String>, ToolOutcome> {
    const TOOL: &str = "javac";
    const RELATIVE: [&str; 9] = [
        "org/reactivestreams/Publisher.java",
        "io/r2dbc/spi/ConnectionFactory.java",
        "io/r2dbc/spi/Connection.java",
        "io/r2dbc/spi/Statement.java",
        "io/r2dbc/spi/Result.java",
        "io/r2dbc/spi/Row.java",
        "io/r2dbc/spi/RowMetadata.java",
        "reactor/core/publisher/Mono.java",
        "reactor/core/publisher/Flux.java",
    ];
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/java_stubs"));
    RELATIVE
        .iter()
        .map(|relative| {
            dir.join(relative)
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| ToolOutcome::Failed {
                    tool: TOOL,
                    reason: format!("{TOOL}: stub source path is not valid UTF-8: {relative}"),
                })
        })
        .collect()
}

/// Validate generated `java-r2dbc` code against the real `javac` compiler,
/// with the R2DBC SPI and Reactor stubs on the source path.
fn validate_java_r2dbc_tools(code: &str) -> Vec<ToolOutcome> {
    let stubs = match java_r2dbc_stub_paths() {
        Ok(paths) => paths,
        Err(outcome) => return vec![outcome],
    };
    javac_with_stubs(code, &stubs)
}

/// Compile `code` as `Queries.java` with `javac`, alongside the JSR-305
/// annotation stubs and any `extra_stubs` the backend needs.
fn javac_with_stubs(code: &str, extra_stubs: &[String]) -> Vec<ToolOutcome> {
    const TOOL: &str = "javac";

    if !tool_present(TOOL, "-version") {
        return vec![ToolOutcome::Missing { tool: TOOL }];
    }

    let source = match JavaSource::new(code) {
        Ok(source) => source,
        Err(outcome) => return vec![outcome],
    };
    let source_arg = match source.source_arg() {
        Ok(path) => path,
        Err(outcome) => return vec![outcome],
    };
    let (nonnull, nullable) = match java_annotation_stub_paths() {
        Ok(paths) => paths,
        Err(outcome) => return vec![outcome],
    };
    let out_dir = match TempOutDir::new(TOOL) {
        Ok(dir) => dir,
        Err(outcome) => return vec![outcome],
    };
    let out_arg = match out_dir.arg(TOOL) {
        Ok(path) => path,
        Err(outcome) => return vec![outcome],
    };

    let mut args = vec![
        "-d".to_string(),
        out_arg.to_string(),
        "-nowarn".to_string(),
        source_arg,
        nonnull,
        nullable,
    ];
    args.extend(extra_stubs.iter().cloned());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    vec![match run_tool(TOOL, &arg_refs, Stream::Both, 20) {
        Ok(errors) => ToolOutcome::Ran { tool: TOOL, errors },
        Err(outcome) => outcome,
    }]
}

/// Validate generated `kotlin-jdbc` code against the real `kotlinc` compiler.
///
/// Only `kotlin-jdbc`, not `kotlin-exposed` or `kotlin-r2dbc` -- see the
/// comment on that match arm in [`validate_with_tools`] for why those two
/// need frameworks this checker cannot cheaply stand up. `kotlin-jdbc`
/// touches only `java.sql`/`java.math`/`java.time`, all part of the JDK
/// `kotlinc` already resolves against, so unlike those two this needs no
/// extra classpath at all -- and unlike `java-jdbc`, Kotlin does not require
/// the source file name to match a public class name, so this can reuse the
/// generic [`check_with_output`] instead of [`JavaSource`]'s bespoke naming.
fn validate_kotlin_tools(code: &str) -> Vec<ToolOutcome> {
    vec![check_with_output(
        "kotlinc",
        "-version",
        code,
        ".kt",
        |source, out| ["-d", out, source].iter().map(|arg| (*arg).to_string()).collect(),
        Stream::Both,
        10,
    )]
}

/// Every file in `tests/elixir_stubs/`: hand-written stand-ins for the external driver structs
/// the `elixir-*` backends construct or pattern-match on (`elixir-myxql`'s `%MyXQL.Result{...}`,
/// `elixir-tds`'s `%Tds.Parameter{...}`, `elixir-jamdb`'s `%Jamdb.Oracle.Query{...}`). See
/// `tests/elixir_stubs/myxql_result.ex` for why a struct reference needs a stub when a plain
/// remote call does not.
///
/// Read from the directory rather than named one by one: a backend that starts referencing a new
/// driver struct then needs only the stub file, and cannot fail here because someone added the
/// file and forgot the corresponding line. Missing or unreadable directory is an error, not an
/// empty list -- silently validating against no stubs at all would turn every struct reference
/// into a compile error attributed to the generated code.
fn elixir_stub_paths() -> Result<Vec<String>, ToolOutcome> {
    const TOOL: &str = "elixirc";
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/elixir_stubs"));
    let entries = std::fs::read_dir(dir).map_err(|e| ToolOutcome::Failed {
        tool: TOOL,
        reason: format!("{TOOL}: cannot read stub directory {}: {e}", dir.display()),
    })?;
    let mut paths: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ex") {
            continue;
        }
        match path.to_str() {
            Some(p) => paths.push(p.to_string()),
            None => {
                return Err(ToolOutcome::Failed {
                    tool: TOOL,
                    reason: format!("{TOOL}: stub struct path is not valid UTF-8"),
                });
            }
        }
    }
    if paths.is_empty() {
        return Err(ToolOutcome::Failed {
            tool: TOOL,
            reason: format!("{TOOL}: no stub modules found in {}", dir.display()),
        });
    }
    // ~keep Deterministic argument order, so a failure reproduces identically run to run.
    paths.sort();
    Ok(paths)
}

/// Validate generated `elixir-*` code against the real `elixirc` compiler.
///
/// Covers every `elixir-*` backend (`elixir-postgrex`, `-ecto`, `-myxql`,
/// `-exqlite`, `-tds`, `-jamdb`). Almost none of them need a stub, which is
/// not laziness -- it is a real asymmetry with Java/Kotlin/C#/Rust. Elixir
/// does not resolve a remote *function* call at compile time:
/// `Postgrex.query/3` when `Postgrex` itself was never compiled is a
/// *warning* ("module is not available or is yet to be defined"), not an
/// error, and `elixirc` still exits `0`. `run_tool` only turns a checker's
/// output into findings when the process itself failed
/// (`output.status.success()` short-circuits otherwise), so these expected
/// warnings about undefined driver modules are silently and correctly
/// ignored -- while a genuine syntax error (verified against `elixirc`
/// directly: a real `MismatchedDelimiterError` exits `1`) still surfaces
/// normally.
///
/// A struct reference is the one place that asymmetry does not hold: `%Mod{
/// ...}` -- construction or pattern match -- expands `Mod.__struct__/1` (or,
/// under this Elixir's set-theoretic type checker, statically checks the
/// pattern) at compile time, and both fail hard when `Mod` was never
/// compiled. Verified directly against `elixirc`, not assumed: `elixir-myxql`
/// and `elixir-tds` are the only two backends that reference an external
/// struct at all (`%MyXQL.Result{...}`, `%Tds.Parameter{...}`), so
/// [`elixir_stub_paths`] supplies both unconditionally -- an unused stub
/// module compiles to a harmless, un-invoked `.beam` file for the other four
/// backends.
fn validate_elixir_tools(code: &str) -> Vec<ToolOutcome> {
    let stubs = match elixir_stub_paths() {
        Ok(paths) => paths,
        Err(outcome) => return vec![outcome],
    };
    vec![check_with_output(
        "elixirc",
        "--version",
        code,
        ".ex",
        move |source, out| {
            let mut args = vec!["-o".to_string(), out.to_string(), source.to_string()];
            args.extend(stubs.iter().cloned());
            args
        },
        Stream::Both,
        8,
    )]
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
    /// unconditionally green. `java-jdbc`, `kotlin-jdbc` and every `elixir-*`
    /// backend have real validators now (see `validate_with_tools`); `csharp-*`
    /// is still in the gap this guards, so it is what this test exercises.
    #[test]
    fn a_backend_with_no_validator_is_unsupported_not_clean() {
        let validation = validate_with_tools("public class Foo {}", "csharp-npgsql");

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
