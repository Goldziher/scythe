pub(crate) mod csharp_microsoft_sqlite;
pub(crate) mod csharp_mysqlconnector;
pub(crate) mod csharp_npgsql;
pub(crate) mod csharp_oracle;
pub(crate) mod csharp_snowflake;
pub(crate) mod csharp_sqlclient;
pub(crate) mod elixir_ecto;
pub(crate) mod elixir_exqlite;
pub(crate) mod elixir_jamdb;
pub(crate) mod elixir_myxql;
pub(crate) mod elixir_postgrex;
pub(crate) mod elixir_tds;
pub(crate) mod go_common;
pub(crate) mod go_database_sql;
pub(crate) mod go_godror;
pub(crate) mod go_gosnowflake;
pub(crate) mod go_pgx;
pub(crate) mod java_jdbc;
pub(crate) mod java_r2dbc;
pub(crate) mod jvm_common;
pub(crate) mod kotlin_exposed;
pub(crate) mod kotlin_jdbc;
pub(crate) mod kotlin_r2dbc;
pub(crate) mod php_amphp;
pub(crate) mod php_common;
pub(crate) mod php_pdo;
pub(crate) mod python_aiomysql;
pub(crate) mod python_aiosqlite;
pub(crate) mod python_asyncpg;
pub(crate) mod python_common;
pub(crate) mod python_duckdb;
pub(crate) mod python_oracledb;
pub(crate) mod python_psycopg3;
pub(crate) mod python_pyodbc;
pub(crate) mod python_snowflake;
pub(crate) mod ruby_mysql2;
pub(crate) mod ruby_oci8;
pub(crate) mod ruby_pg;
pub(crate) mod ruby_rbs;
pub(crate) mod ruby_sqlite3;
pub(crate) mod ruby_tiny_tds;
pub(crate) mod ruby_trilogy;
pub(crate) mod rust_sibyl;
pub(crate) mod rust_tiberius;
pub(crate) mod sqlx;
pub(crate) mod tokio_postgres;
pub(crate) mod typescript_better_sqlite3;
pub(crate) mod typescript_common;
pub(crate) mod typescript_duckdb;
pub(crate) mod typescript_kysely;
pub(crate) mod typescript_mssql;
pub(crate) mod typescript_mysql2;
pub(crate) mod typescript_node_sqlite;
pub(crate) mod typescript_oracledb;
pub(crate) mod typescript_pg;
pub(crate) mod typescript_postgres;
pub(crate) mod typescript_snowflake;
pub(crate) mod typescript_wasm_sqlite;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::NamingConfig;
use scythe_core::analyzer::AnalyzedParam;
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::CodegenBackend;

/// Parse a backend's compiled-in manifest.
///
/// Manifest selection is a pure function of (backend, engine). There is
/// deliberately no filesystem lookup here: generated output must not depend
/// on the process working directory (#82).
pub(crate) fn parse_manifest(manifest_toml: &str) -> Result<BackendManifest, ScytheError> {
    toml::from_str(manifest_toml).map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))
}

/// Validate and apply the `field_case` backend option, writing it into a
/// backend's `NamingConfig`.
///
/// Shared by the five driver-level Java and Kotlin backends (`java-jdbc`,
/// `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `kotlin-exposed`). Unlike the
/// TypeScript backends' `field_case` (see `typescript_common::TsFieldCase`),
/// none of these five need a companion runtime remap: every row they build
/// comes from an explicit `ResultSet`/`Row` getter call keyed by the raw SQL
/// column name (`ResolvedColumn::name`), with `ResolvedColumn::field_name`
/// used only as the emitted record/data-class property name -- see e.g.
/// `java_jdbc::col_rs_expr`, which reads `rs.getX(col.name)` and assigns it
/// positionally into a constructor whose declared parameter is `col.field_name`.
/// Renaming the property therefore never changes what key the driver is
/// asked to look up. `resolve::resolve_columns` is what performs the actual
/// rename (and rejects same-query collisions) once this sets
/// `NamingConfig.field_case`.
pub(crate) fn apply_field_case_option(
    naming: &mut NamingConfig,
    backend_name: &str,
    value: &str,
) -> Result<(), ScytheError> {
    match value {
        "snake_case" | "camelCase" => {
            naming.field_case = value.to_string();
            Ok(())
        }
        other => Err(ScytheError::new(
            ErrorCode::InvalidConfig,
            format!("{backend_name}: invalid field_case '{other}' (expected 'snake_case' or 'camelCase')"),
        )),
    }
}

/// Whether a backend built for `engine` may emit nested-aggregate structs.
///
/// The four opted-in backends (`rust-sqlx`, `rust-tokio-postgres`, `go-pgx`,
/// `python-psycopg3`) all also serve `redshift`, and `rust-sqlx` serves
/// MySQL/MariaDB/SQLite besides. `json_agg` and `row_to_json` are PostgreSQL
/// functions; Redshift has neither. The analyzer already declines to infer a
/// nested type for a Redshift catalog, but the backend cannot rely on that
/// alone — it also selects a *different manifest* per engine, and the
/// Redshift manifests deliberately do not declare the `json_nested`
/// container. Opting in there would resolve the column against a container
/// the manifest never defines.
///
/// Kept as one predicate so all four backends answer the question the same
/// way instead of each spelling out its own engine list.
pub(crate) fn engine_supports_nested_aggregates(engine: &str) -> bool {
    matches!(engine, "postgresql" | "postgres" | "pg")
}

/// The ADO.NET accessor a `csharp-*` backend must use for a column whose
/// neutral type its driver-specific accessor table does not name.
///
/// `DbDataReader.GetValue` is declared to return `object`, so a record field
/// the manifest declared as anything else -- `List<{T}>` for an `array`,
/// `byte[]` for `bytes`, a composite record, `{T}` for `json_typed` -- does
/// not bind to it. The compiler rejects the constructor call outright:
///
/// ```text
/// error CS1503: Argument 2: cannot convert from 'object' to
/// 'System.Collections.Generic.List<string>'
/// ```
///
/// `DbDataReader.GetFieldValue<T>` is the typed path every ADO.NET provider
/// inherits, and it returns `T` by signature. Naming the manifest's own
/// declared type as `T` therefore makes the reader agree with the declaration
/// by construction, rather than by a second hand-maintained table that can
/// drift away from the manifest -- which is precisely how this defect
/// (issue #155) survived in five backends at once.
///
/// The typed path is not merely a compile-time trick: verified against a live
/// PostgreSQL 17 server through Npgsql 8, `GetFieldValue<List<string>>` reads
/// a `text[]`, `GetFieldValue<TimeSpan>` an `interval`,
/// `GetFieldValue<System.Net.IPAddress>` an `inet` and
/// `GetFieldValue<byte[]>` a `bytea`. Degrading the *manifest* to whatever
/// `GetValue`/`GetString` happens to return would throw away types the driver
/// can genuinely produce.
pub(crate) fn csharp_typed_reader_method(lang_type: &str) -> String {
    format!("GetFieldValue<{lang_type}>")
}

/// A classified span of SQL text produced by [`tokenize_sql`].
///
/// One lexer pass drives both comment-stripping (`clean_sql`,
/// `clean_sql_oneline`) and placeholder-rewriting (`rewrite_pg_placeholders`)
/// -- see #186 and #148, where two independent character scanners each got
/// string/comment handling wrong in a different way. `Code` spans are the
/// only spans either consumer may rewrite; every other span is opaque SQL
/// text that must be reproduced byte-for-byte (module the caller's own
/// whitespace joining).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlSpanKind {
    /// Ordinary SQL: identifiers, keywords, operators, placeholders.
    Code,
    /// A `'...'` string literal, with `''` as the escape for an embedded quote.
    SingleQuoted,
    /// A `"..."` quoted identifier, with `""` as the escape for an embedded quote.
    DoubleQuoted,
    /// A PostgreSQL `$$...$$` or `$tag$...$tag$` dollar-quoted string.
    DollarQuoted,
    /// A `-- ...` comment, running to end of line (the newline itself is not
    /// part of the span).
    LineComment,
    /// A `/* ... */` comment. PostgreSQL nests these, so this tracks nesting
    /// depth; an unterminated comment consumes to end of input.
    BlockComment,
}

/// Tokenize `sql` into a sequence of classified spans whose text
/// concatenates back to exactly `sql`.
///
/// This is the single source of truth for "is this character part of a
/// string/identifier/comment, or is it live SQL". Dialect-specific lexical
/// forms that would require knowing the target engine (MySQL backtick
/// identifiers and `#` comments, MSSQL `[bracketed]` identifiers) are
/// deliberately not handled here: none of these functions are told which
/// engine produced the SQL, and guessing from the text alone is unsafe --
/// `#` in particular collides with PostgreSQL's `#>`/`#>>` JSON operators.
/// Handling those dialects needs the caller to pass an explicit engine/
/// dialect value through to this module; until that plumbing exists, this
/// lexer only recognizes the ANSI-ish subset (`'...'`, `"..."`, `$$...$$`,
/// `--`, `/* */`) that is safe across every engine this crate targets.
fn tokenize_sql(sql: &str) -> Vec<(SqlSpanKind, String)> {
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut spans: Vec<(SqlSpanKind, String)> = Vec::new();
    let mut code_buf = String::new();
    let mut i = 0;

    while i < len {
        let c = chars[i];

        if c == '-' && i + 1 < len && chars[i + 1] == '-' {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            i += 2;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            spans.push((SqlSpanKind::LineComment, chars[start..i].iter().collect()));
            continue;
        }

        if c == '/' && i + 1 < len && chars[i + 1] == '*' {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            i += 2;
            let mut depth = 1u32;
            while i < len && depth > 0 {
                if chars[i] == '/' && i + 1 < len && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && i + 1 < len && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            spans.push((SqlSpanKind::BlockComment, chars[start..i].iter().collect()));
            continue;
        }

        if c == '\'' {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            i += 1;
            while i < len {
                if chars[i] == '\'' {
                    if i + 1 < len && chars[i + 1] == '\'' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push((SqlSpanKind::SingleQuoted, chars[start..i].iter().collect()));
            continue;
        }

        if c == '"' {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            i += 1;
            while i < len {
                if chars[i] == '"' {
                    if i + 1 < len && chars[i + 1] == '"' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push((SqlSpanKind::DoubleQuoted, chars[start..i].iter().collect()));
            continue;
        }

        if c == '$'
            && let Some(open_end) = match_dollar_quote_open(&chars, i)
        {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            let delim: Vec<char> = chars[i..open_end].to_vec();
            let mut j = open_end;
            let mut found = false;
            while j < len {
                if chars[j] == '$' && j + delim.len() <= len && chars[j..j + delim.len()] == delim[..] {
                    j += delim.len();
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                j = len;
            }
            spans.push((SqlSpanKind::DollarQuoted, chars[start..j].iter().collect()));
            i = j;
            continue;
        }

        code_buf.push(c);
        i += 1;
    }

    flush_code_buf(&mut code_buf, &mut spans);
    spans
}

fn flush_code_buf(code_buf: &mut String, spans: &mut Vec<(SqlSpanKind, String)>) {
    if !code_buf.is_empty() {
        spans.push((SqlSpanKind::Code, std::mem::take(code_buf)));
    }
}

/// If `chars[i]` (a `$`) opens a dollar-quote delimiter (`$$` or `$tag$`),
/// return the index just past the opening delimiter. A tag follows SQL
/// identifier rules (must not start with a digit), which is exactly what
/// keeps this from misfiring on a `$1`-style placeholder.
fn match_dollar_quote_open(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j < chars.len() && chars[j] == '$' {
        return Some(j + 1);
    }
    let tag_start = j;
    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
        j += 1;
    }
    if j > tag_start && j < chars.len() && chars[j] == '$' && !chars[tag_start].is_ascii_digit() {
        return Some(j + 1);
    }
    None
}

/// Strip SQL comments, trailing semicolons, and excess whitespace.
/// Preserves newlines between lines.
pub(crate) fn clean_sql(sql: &str) -> String {
    clean_sql_lines(sql)
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Like clean_sql but joins lines with spaces (for languages that embed SQL inline).
pub(crate) fn clean_sql_oneline(sql: &str) -> String {
    clean_sql_lines(sql)
        .join(" ")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Split `sql` into lines with every comment (`--` and `/* */`, string- and
/// dollar-quote-aware) removed, dropping any line whose content was entirely
/// a comment.
///
/// A line is dropped only when a comment is what emptied it -- a line that
/// was already blank in the source (no comment token touched it) is kept,
/// matching the previous behaviour of the naive `starts_with("--")` filter
/// for every case that filter got right. This is what lets a mid-line
/// trailing comment (#148) be stripped without deleting the rest of the
/// query, while a `-- @name Foo` header line still disappears instead of
/// leaving a blank line behind (every generated fixture depends on that).
fn clean_sql_lines(sql: &str) -> Vec<String> {
    let normalized = sql.replace("\r\n", "\n");
    let spans = tokenize_sql(&normalized);

    let mut lines: Vec<String> = vec![String::new()];
    let mut touched_by_comment: Vec<bool> = vec![false];

    for (kind, text) in &spans {
        let is_comment = matches!(kind, SqlSpanKind::LineComment | SqlSpanKind::BlockComment);
        for ch in text.chars() {
            if ch == '\n' {
                if is_comment {
                    *touched_by_comment.last_mut().expect("at least one line") = true;
                }
                lines.push(String::new());
                touched_by_comment.push(false);
            } else if is_comment {
                *touched_by_comment.last_mut().expect("at least one line") = true;
            } else {
                lines.last_mut().expect("at least one line").push(ch);
            }
        }
    }

    lines
        .into_iter()
        .zip(touched_by_comment)
        .filter(|(kept, touched)| !(*touched && kept.trim().is_empty()))
        .map(|(kept, _)| kept)
        .collect()
}

/// Rewrite SQL for optional parameters.
///
/// For each optional param, finds `column = $N` (or `column <> $N`, `column != $N`)
/// and rewrites to `($N IS NULL OR column = $N)`. This allows callers to pass NULL
/// to skip a filter condition at runtime.
///
/// This operates on the raw SQL before any backend-specific placeholder rewriting.
pub(crate) fn rewrite_optional_params(sql: &str, optional_params: &[String], params: &[AnalyzedParam]) -> String {
    if optional_params.is_empty() {
        return sql.to_string();
    }

    let mut result = sql.to_string();

    for opt_name in optional_params {
        let Some(param) = params.iter().find(|p| p.name == *opt_name) else {
            continue;
        };
        let placeholder = format!("${}", param.position);

        for op in &[
            ">=",
            "<=",
            "<>",
            "!=",
            ">",
            "<",
            "=",
            "NOT ILIKE",
            "not ilike",
            "NOT LIKE",
            "not like",
            "ILIKE",
            "ilike",
            "LIKE",
            "like",
        ] {
            result = rewrite_comparison(&result, &placeholder, op);
        }
    }

    result
}

/// Rewrite a single `column <op> $N` pattern to `($N IS NULL OR column <op> $N)`.
/// Handles both `column <op> $N` and `$N <op> column` orderings.
fn rewrite_comparison(sql: &str, placeholder: &str, op: &str) -> String {
    let mut result = String::with_capacity(sql.len() + 32);
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if let Some((_start, col, end)) = try_match_col_op_ph(&chars, i, op, placeholder) {
            result.push_str(&format!("({placeholder} IS NULL OR {col} {op} {placeholder})"));
            i = end;
            continue;
        }

        if let Some((end, col)) = try_match_ph_op_col(&chars, i, op, placeholder) {
            result.push_str(&format!("({placeholder} IS NULL OR {col} {op} {placeholder})"));
            i = end;
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Keywords that `is_ident_char` would happily scan as a column name but
/// that can never actually be one in this position. Without this check, a
/// compound operator like `NOT LIKE` gets misread: the scanner captures
/// `NOT` as "the column" and matches `LIKE` as "the operator", producing
/// `col NOT ($N IS NULL OR LIKE $N)` -- invalid SQL that silently drops the
/// real column (#186 item 3). This also guards against a second rewrite
/// pass (e.g. plain `LIKE` running after `NOT LIKE` already matched) seeing
/// the literal word `NOT` it just emitted and matching on that.
fn is_reserved_non_column_keyword(ident: &str) -> bool {
    matches!(
        ident.to_ascii_uppercase().as_str(),
        "NOT" | "AND" | "OR" | "IS" | "NULL"
    )
}

/// Match `op`, split on whitespace, against `chars` starting at `j`,
/// tolerating any amount of whitespace between words of a compound operator
/// (`NOT LIKE`, `NOT ILIKE`). Single-word operators degenerate to an exact
/// literal match, same as before. Returns the index just past the match.
fn match_op_tokens(chars: &[char], mut j: usize, op: &str) -> Option<usize> {
    for (idx, word) in op.split_whitespace().enumerate() {
        if idx > 0 {
            let ws_start = j;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j == ws_start {
                return None;
            }
        }
        let word_chars: Vec<char> = word.chars().collect();
        if j + word_chars.len() > chars.len() {
            return None;
        }
        for (k, wc) in word_chars.iter().enumerate() {
            if chars[j + k] != *wc {
                return None;
            }
        }
        j += word_chars.len();
    }
    Some(j)
}

/// Try to match `identifier <ws>* <op> <ws>* placeholder` starting at position `i`.
/// Returns `(match_start, column_name, match_end)` if found.
fn try_match_col_op_ph(chars: &[char], i: usize, op: &str, placeholder: &str) -> Option<(usize, String, usize)> {
    if !is_ident_char(chars[i]) {
        return None;
    }
    if i > 0 && is_ident_char(chars[i - 1]) {
        return None;
    }

    let ident_start = i;
    let mut j = i;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    let ident: String = chars[ident_start..j].iter().collect();
    if is_reserved_non_column_keyword(&ident) {
        return None;
    }

    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    j = match_op_tokens(chars, j, op)?;

    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    let ph_chars: Vec<char> = placeholder.chars().collect();
    if j + ph_chars.len() > chars.len() {
        return None;
    }
    for (k, pc) in ph_chars.iter().enumerate() {
        if chars[j + k] != *pc {
            return None;
        }
    }
    j += ph_chars.len();

    if j < chars.len() && chars[j].is_ascii_digit() {
        return None;
    }

    Some((i, ident, j))
}

/// Try to match `placeholder <ws>* <op> <ws>* identifier` starting at position `i`.
/// Returns `(match_end, column_name)` if found.
fn try_match_ph_op_col(chars: &[char], i: usize, op: &str, placeholder: &str) -> Option<(usize, String)> {
    let ph_chars: Vec<char> = placeholder.chars().collect();
    if i + ph_chars.len() > chars.len() {
        return None;
    }

    if i > 0 && (chars[i - 1] == '$' || chars[i - 1].is_ascii_digit()) {
        return None;
    }

    for (k, pc) in ph_chars.iter().enumerate() {
        if chars[i + k] != *pc {
            return None;
        }
    }
    let mut j = i + ph_chars.len();

    if j < chars.len() && chars[j].is_ascii_digit() {
        return None;
    }

    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    j = match_op_tokens(chars, j, op)?;

    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    if j >= chars.len() || !is_ident_char(chars[j]) {
        return None;
    }
    let ident_start = j;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    let ident: String = chars[ident_start..j].iter().collect();

    if is_reserved_non_column_keyword(&ident) {
        return None;
    }

    Some((j, ident))
}

/// Clean SQL and apply optional parameter rewriting.
pub(crate) fn clean_sql_with_optional(sql: &str, optional_params: &[String], params: &[AnalyzedParam]) -> String {
    let cleaned = clean_sql(sql);
    rewrite_optional_params(&cleaned, optional_params, params)
}

/// Clean SQL (oneline) and apply optional parameter rewriting.
pub(crate) fn clean_sql_oneline_with_optional(
    sql: &str,
    optional_params: &[String],
    params: &[AnalyzedParam],
) -> String {
    let cleaned = clean_sql_oneline(sql);
    rewrite_optional_params(&cleaned, optional_params, params)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// Rewrite SQL placeholders (`$N` or `?`) to a target format.
///
/// Skips placeholders inside single-/double-quoted and dollar-quoted spans,
/// and inside comments (see [`tokenize_sql`]). The `formatter` closure
/// receives the parameter number (1-based) and returns the replacement.
///
/// A query uses exactly one placeholder style: PostgreSQL/Redshift/
/// CockroachDB SQL always uses `$N` (the core parser normalizes MySQL's
/// bare `?`, and rewrites Oracle `:N`/MSSQL `@pN` to bare `?`, before this
/// function ever sees the SQL). Whether the *current* query is `$N`-style is
/// therefore determined by scanning the code spans for a `$`-followed-by-
/// digit occurrence: if one exists, a bare `?` is left untouched, because on
/// a `$N`-style (PostgreSQL-family) query a bare `?` can only be the JSONB
/// `?` key-existence operator, never a placeholder (#186). Otherwise a bare
/// `?` not immediately followed by a digit is treated as a sequential
/// positional placeholder, matching every non-PostgreSQL caller's existing
/// behaviour.
///
/// `?|`, `?&`, `?-`, `?-|`, `?||`, and `@?` are multi-character PostgreSQL
/// operators (JSONB key/path existence, geometric comparisons) that are
/// **never** valid placeholder syntax in any dialect this crate targets, so
/// [`rewrite_code_span_placeholders`] recognizes and skips them unconditionally
/// -- independent of the `$N` heuristic above, and correct even for a
/// zero-placeholder query that has no `$N` to anchor that heuristic on.
pub(crate) fn rewrite_pg_placeholders(sql: &str, formatter: impl Fn(u32) -> String) -> String {
    let spans = tokenize_sql(sql);
    let uses_dollar_placeholders = spans
        .iter()
        .any(|(kind, text)| *kind == SqlSpanKind::Code && code_span_has_dollar_number(text));

    let mut result = String::with_capacity(sql.len());
    let mut positional_counter: u32 = 0;
    for (kind, text) in &spans {
        if *kind == SqlSpanKind::Code {
            rewrite_code_span_placeholders(
                text,
                uses_dollar_placeholders,
                &formatter,
                &mut positional_counter,
                &mut result,
            );
        } else {
            result.push_str(text);
        }
    }
    result
}

/// Whether `text` (a `Code`-span substring, guaranteed free of strings and
/// comments) contains a `$` immediately followed by an ASCII digit.
fn code_span_has_dollar_number(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len()).any(|i| chars[i] == '$' && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()))
}

/// Multi-character PostgreSQL operators built on `?` or `@?` that are never
/// placeholder syntax in any dialect this crate targets -- JSONB key-array
/// existence (`?|`, `?&`) and jsonpath existence (`@?`), plus the geometric
/// comparison operators (`?-`, `?-|`, `?||`). Longer operators are listed
/// before the shorter operators they prefix (`?-|` before `?-`, `?||` before
/// `?|`) so the greedy match in [`match_unambiguous_operator`] picks the
/// longest one, not "the first two characters".
const UNAMBIGUOUS_OPERATORS: [&str; 6] = ["?-|", "?||", "?|", "?&", "?-", "@?"];

/// If `chars[i..]` starts with one of [`UNAMBIGUOUS_OPERATORS`], return its
/// length so the caller can copy it through untouched.
fn match_unambiguous_operator(chars: &[char], i: usize) -> Option<usize> {
    UNAMBIGUOUS_OPERATORS.iter().find_map(|op| {
        let op_chars: Vec<char> = op.chars().collect();
        let end = i + op_chars.len();
        (end <= chars.len() && chars[i..end] == op_chars[..]).then_some(op_chars.len())
    })
}

/// Rewrite `$N` and (when `uses_dollar_placeholders` is false) bare `?`
/// placeholders within a single `Code` span, appending the result to `out`
/// and advancing `counter` for each `?` consumed.
///
/// A lone `?` between two expressions is genuinely ambiguous without dialect
/// information -- it is a positional placeholder on `?`-style engines and
/// PostgreSQL's JSONB key-existence operator on `$N`-style engines -- so it
/// stays governed by the `uses_dollar_placeholders` heuristic and can still
/// misfire on a zero-placeholder PostgreSQL query that uses bare `?` (no
/// `$N` present to detect the dialect from). The multi-character operators
/// in [`UNAMBIGUOUS_OPERATORS`] have no such ambiguity and are always passed
/// through, regardless of that heuristic.
fn rewrite_code_span_placeholders(
    text: &str,
    uses_dollar_placeholders: bool,
    formatter: &impl Fn(u32) -> String,
    counter: &mut u32,
    out: &mut String,
) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if let Some(op_len) = match_unambiguous_operator(&chars, i) {
            out.extend(&chars[i..i + op_len]);
            i += op_len;
            continue;
        }

        let ch = chars[i];
        if ch == '$' && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
            let mut j = i + 1;
            let mut num_str = String::new();
            while j < len && chars[j].is_ascii_digit() {
                num_str.push(chars[j]);
                j += 1;
            }
            let num: u32 = num_str.parse().unwrap_or(0);
            out.push_str(&formatter(num));
            i = j;
        } else if ch == '?' && !uses_dollar_placeholders && !chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
            *counter += 1;
            out.push_str(&formatter(*counter));
            i += 1;
        } else {
            out.push(ch);
            i += 1;
        }
    }
}

/// Get a backend by name and database engine.
///
/// The `engine` parameter (e.g., "postgresql", "mysql", "sqlite") determines
/// which manifest is loaded for type mappings. PG-only backends reject non-PG engines.
pub fn get_backend(name: &str, engine: &str) -> Result<Box<dyn CodegenBackend>, ScytheError> {
    let canonical_engine = normalize_engine(engine);
    let backend: Box<dyn CodegenBackend> = match name {
        "rust-sqlx" | "sqlx" | "rust" => Box::new(sqlx::SqlxBackend::new(canonical_engine)?),
        "rust-tokio-postgres" | "tokio-postgres" => {
            Box::new(tokio_postgres::TokioPostgresBackend::new(canonical_engine)?)
        }
        "python-psycopg3" | "python" => Box::new(python_psycopg3::PythonPsycopg3Backend::new(canonical_engine)?),
        "python-asyncpg" => Box::new(python_asyncpg::PythonAsyncpgBackend::new(canonical_engine)?),
        "python-aiomysql" => Box::new(python_aiomysql::PythonAiomysqlBackend::new(canonical_engine)?),
        "python-aiosqlite" => Box::new(python_aiosqlite::PythonAiosqliteBackend::new(canonical_engine)?),
        "python-duckdb" => Box::new(python_duckdb::PythonDuckdbBackend::new(canonical_engine)?),
        "typescript-postgres" | "ts" | "typescript" => {
            Box::new(typescript_postgres::TypescriptPostgresBackend::new(canonical_engine)?)
        }
        "javascript-postgres" => Box::new(typescript_postgres::TypescriptPostgresBackend::new_js(
            canonical_engine,
        )?),
        "typescript-pg" => Box::new(typescript_pg::TypescriptPgBackend::new(canonical_engine)?),
        "javascript-pg" => Box::new(typescript_pg::TypescriptPgBackend::new_js(canonical_engine)?),
        "typescript-mysql2" => Box::new(typescript_mysql2::TypescriptMysql2Backend::new(canonical_engine)?),
        "javascript-mysql2" => Box::new(typescript_mysql2::TypescriptMysql2Backend::new_js(canonical_engine)?),
        "typescript-better-sqlite3" => Box::new(typescript_better_sqlite3::TypescriptBetterSqlite3Backend::new(
            canonical_engine,
        )?),
        "javascript-better-sqlite3" => Box::new(typescript_better_sqlite3::TypescriptBetterSqlite3Backend::new_js(
            canonical_engine,
        )?),
        "typescript-duckdb" => Box::new(typescript_duckdb::TypescriptDuckdbBackend::new(canonical_engine)?),
        "typescript-node-sqlite" => Box::new(typescript_node_sqlite::TypescriptNodeSqliteBackend::new(
            canonical_engine,
        )?),
        "typescript-wasm-sqlite" => Box::new(typescript_wasm_sqlite::TypescriptWasmSqliteBackend::new(
            canonical_engine,
        )?),
        "typescript-kysely" | "kysely" => Box::new(typescript_kysely::TypescriptKyselyBackend::new(canonical_engine)?),
        "go-database-sql" => Box::new(go_database_sql::GoDatabaseSqlBackend::new(canonical_engine)?),
        "go-pgx" | "go" => Box::new(go_pgx::GoPgxBackend::new(canonical_engine)?),
        "java-jdbc" | "java" => Box::new(java_jdbc::JavaJdbcBackend::new(canonical_engine)?),
        "java-r2dbc" | "r2dbc-java" => Box::new(java_r2dbc::JavaR2dbcBackend::new(canonical_engine)?),
        "kotlin-exposed" | "exposed" => Box::new(kotlin_exposed::KotlinExposedBackend::new(canonical_engine)?),
        "kotlin-jdbc" | "kotlin" | "kt" => Box::new(kotlin_jdbc::KotlinJdbcBackend::new(canonical_engine)?),
        "kotlin-r2dbc" | "r2dbc-kotlin" => Box::new(kotlin_r2dbc::KotlinR2dbcBackend::new(canonical_engine)?),
        "csharp-npgsql" | "csharp" | "c#" | "dotnet" => {
            Box::new(csharp_npgsql::CsharpNpgsqlBackend::new(canonical_engine)?)
        }
        "csharp-mysqlconnector" => Box::new(csharp_mysqlconnector::CsharpMysqlConnectorBackend::new(
            canonical_engine,
        )?),
        "csharp-microsoft-sqlite" => Box::new(csharp_microsoft_sqlite::CsharpMicrosoftSqliteBackend::new(
            canonical_engine,
        )?),
        "elixir-postgrex" | "elixir" | "ex" => Box::new(elixir_postgrex::ElixirPostgrexBackend::new(canonical_engine)?),
        "elixir-ecto" | "ecto" => Box::new(elixir_ecto::ElixirEctoBackend::new(canonical_engine)?),
        "elixir-myxql" => Box::new(elixir_myxql::ElixirMyxqlBackend::new(canonical_engine)?),
        "elixir-exqlite" => Box::new(elixir_exqlite::ElixirExqliteBackend::new(canonical_engine)?),
        "ruby-pg" | "ruby" | "rb" => Box::new(ruby_pg::RubyPgBackend::new(canonical_engine)?),
        "ruby-mysql2" => Box::new(ruby_mysql2::RubyMysql2Backend::new(canonical_engine)?),
        "ruby-sqlite3" => Box::new(ruby_sqlite3::RubySqlite3Backend::new(canonical_engine)?),
        "ruby-trilogy" | "trilogy" => Box::new(ruby_trilogy::RubyTrilogyBackend::new(canonical_engine)?),
        "php-pdo" | "php" => Box::new(php_pdo::PhpPdoBackend::new(canonical_engine)?),
        "php-amphp" | "amphp" => Box::new(php_amphp::PhpAmphpBackend::new(canonical_engine)?),
        "rust-tiberius" | "tiberius" => Box::new(rust_tiberius::RustTiberiusBackend::new(canonical_engine)?),
        "python-pyodbc" | "pyodbc" => Box::new(python_pyodbc::PythonPyodbcBackend::new(canonical_engine)?),
        "typescript-mssql" | "tedious" => Box::new(typescript_mssql::TypescriptMssqlBackend::new(canonical_engine)?),
        "csharp-sqlclient" => Box::new(csharp_sqlclient::CsharpSqlClientBackend::new(canonical_engine)?),
        "ruby-tiny-tds" | "tiny-tds" | "tiny_tds" => {
            Box::new(ruby_tiny_tds::RubyTinyTdsBackend::new(canonical_engine)?)
        }
        "elixir-tds" | "tds" => Box::new(elixir_tds::ElixirTdsBackend::new(canonical_engine)?),
        "rust-sibyl" | "sibyl" => Box::new(rust_sibyl::RustSibylBackend::new(canonical_engine)?),
        "python-oracledb" | "oracledb" => Box::new(python_oracledb::PythonOracledbBackend::new(canonical_engine)?),
        "typescript-oracledb" => Box::new(typescript_oracledb::TypescriptOracledbBackend::new(canonical_engine)?),
        "go-godror" | "godror" => Box::new(go_godror::GoGodrorBackend::new(canonical_engine)?),
        "csharp-oracle" => Box::new(csharp_oracle::CsharpOracleBackend::new(canonical_engine)?),
        "ruby-oci8" | "oci8" => Box::new(ruby_oci8::RubyOci8Backend::new(canonical_engine)?),
        "elixir-jamdb" | "jamdb" => Box::new(elixir_jamdb::ElixirJamdbBackend::new(canonical_engine)?),
        "python-snowflake" => Box::new(python_snowflake::PythonSnowflakeBackend::new(canonical_engine)?),
        "typescript-snowflake" => Box::new(typescript_snowflake::TypescriptSnowflakeBackend::new(canonical_engine)?),
        "go-gosnowflake" | "gosnowflake" => Box::new(go_gosnowflake::GoGosnowflakeBackend::new(canonical_engine)?),
        "csharp-snowflake" => Box::new(csharp_snowflake::CsharpSnowflakeBackend::new(canonical_engine)?),
        _ => {
            return Err(ScytheError::new(
                ErrorCode::InvalidConfig,
                format!("unknown backend: {}", name),
            ));
        }
    };

    if !backend
        .supported_engines()
        .iter()
        .any(|e| normalize_engine(e) == canonical_engine)
    {
        return Err(ScytheError::new(
            ErrorCode::InvalidConfig,
            format!(
                "backend '{}' does not support engine '{}'. Supported: {:?}",
                name,
                engine,
                backend.supported_engines()
            ),
        ));
    }

    Ok(backend)
}

/// Normalize engine name to canonical form.
///
/// Public so callers outside this crate (e.g. `scythe-cli`) can classify a
/// `[[sql]]` block's configured engine without duplicating the alias table —
/// for example, to decide whether a block is PostgreSQL-wire-compatible
/// before attempting live-database verification.
pub fn normalize_engine(engine: &str) -> &str {
    match engine {
        "postgresql" | "postgres" | "pg" | "cockroachdb" | "crdb" => "postgresql",
        "mysql" => "mysql",
        "mariadb" => "mariadb",
        "sqlite" | "sqlite3" => "sqlite",
        "duckdb" => "duckdb",
        "mssql" | "sqlserver" | "tsql" => "mssql",
        "oracle" => "oracle",
        "snowflake" => "snowflake",
        "redshift" => "redshift",
        other => other,
    }
}

/// Exact-value characterization tests for `clean_sql`, `clean_sql_oneline`,
/// and `rewrite_pg_placeholders`, kept in their own file so the pinned
/// behaviour is readable as a specification for the planned dialect-aware
/// tokenizer.
#[cfg(test)]
mod sql_text_characterization_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, position: i64) -> AnalyzedParam {
        AnalyzedParam {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            nullable: true,
            position,
        }
    }

    /// Structural guard for #82: no source file under `src/backends` may
    /// reference a manifest path relative to the process working directory,
    /// however that reference is spelled. This catches a reintroduction of
    /// the filesystem-lookup pattern written differently from the original
    /// (different variable names, a helper function, etc.) that a simple
    /// function-name grep for the old `load_or_default_manifest` helper would
    /// miss.
    ///
    /// The forbidden needle is built at runtime, not written as a literal
    /// here, so this check does not trip over its own source describing what
    /// it looks for.
    #[test]
    fn test_no_backends_relative_manifest_path_literals() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends");
        let word: String = format!("backend{}", "s/");
        let needle: String = format!("{}{}", '"', word);

        let mut offenders = Vec::new();
        collect_rs_files(std::path::Path::new(root), &mut offenders);

        let offenders: Vec<_> = offenders
            .into_iter()
            .filter(|path| {
                let contents = std::fs::read_to_string(path).expect("failed to read source file");
                contents.contains(&needle)
            })
            .collect();

        assert!(
            offenders.is_empty(),
            "found a working-directory-relative manifest path literal in: {offenders:?} (#82 regression \
             -- manifest selection must be a pure function of (backend, engine), no filesystem lookup)"
        );
    }

    /// Recursively collect every `.rs` file path under `dir`.
    fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir).expect("failed to read backends dir");
        for entry in entries {
            let entry = entry.expect("failed to read dir entry");
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    /// Documents the engine-blind collision the deleted CWD-relative lookup
    /// would have caused: two backends sharing a backend-name-only manifest
    /// path (`backends/<name>/manifest.toml`) but supporting different
    /// engines would have silently shared one manifest file on disk. Pure
    /// `(backend, engine)` selection must not exhibit this collision.
    #[test]
    fn test_get_backend_rust_sqlx_mysql_has_no_postgresql_scalars() {
        let backend = get_backend("rust-sqlx", "mysql").expect("rust-sqlx should support mysql");
        let scalars = &backend.manifest().types.scalars;
        assert_ne!(
            scalars.get("uuid").map(String::as_str),
            Some("uuid::Uuid"),
            "rust-sqlx/mysql manifest should not carry the PostgreSQL-specific 'uuid' mapping"
        );
        assert_ne!(
            scalars.get("inet").map(String::as_str),
            Some("ipnetwork::IpNetwork"),
            "rust-sqlx/mysql manifest should not carry the PostgreSQL-specific 'inet' mapping"
        );
    }

    #[test]
    fn test_get_backend_java_jdbc_sqlite_has_no_postgresql_scalars() {
        let backend = get_backend("java-jdbc", "sqlite").expect("java-jdbc should support sqlite");
        let scalars = &backend.manifest().types.scalars;
        assert_ne!(
            scalars.get("uuid").map(String::as_str),
            Some("java.util.UUID"),
            "java-jdbc/sqlite manifest should not carry the PostgreSQL-specific 'uuid' mapping"
        );
        assert_ne!(
            scalars.get("decimal").map(String::as_str),
            Some("java.math.BigDecimal"),
            "java-jdbc/sqlite manifest should not carry the PostgreSQL-specific 'decimal' mapping"
        );
    }

    #[test]
    fn test_normalize_engine_cockroachdb() {
        assert_eq!(normalize_engine("cockroachdb"), "postgresql");
        assert_eq!(normalize_engine("crdb"), "postgresql");
    }

    #[test]
    fn test_get_backend_cockroachdb_with_pg_backends() {
        let pg_backends = [
            "rust-sqlx",
            "rust-tokio-postgres",
            "python-psycopg3",
            "python-asyncpg",
            "typescript-postgres",
            "typescript-pg",
            "typescript-kysely",
            "go-pgx",
            "ruby-pg",
            "elixir-postgrex",
            "csharp-npgsql",
            "php-pdo",
            "php-amphp",
        ];
        for backend_name in &pg_backends {
            let result = get_backend(backend_name, "cockroachdb");
            assert!(
                result.is_ok(),
                "backend '{}' should accept cockroachdb engine, got: {:?}",
                backend_name,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_backend_crdb_alias() {
        let result = get_backend("rust-sqlx", "crdb");
        assert!(result.is_ok(), "rust-sqlx should accept 'crdb' engine alias");
    }

    #[test]
    fn test_normalize_engine_duckdb() {
        assert_eq!(normalize_engine("duckdb"), "duckdb");
    }

    #[test]
    fn test_get_backend_duckdb_with_compatible_backends() {
        let duckdb_backends = [
            "python-duckdb",
            "typescript-duckdb",
            "go-database-sql",
            "java-jdbc",
            "kotlin-jdbc",
        ];
        for backend_name in &duckdb_backends {
            let result = get_backend(backend_name, "duckdb");
            assert!(
                result.is_ok(),
                "backend '{}' should accept duckdb engine, got: {:?}",
                backend_name,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_backend_duckdb_rejected_by_pg_only() {
        let result = get_backend("rust-sqlx", "duckdb");
        assert!(result.is_err(), "rust-sqlx should reject duckdb engine");
    }

    #[test]
    fn test_rewrite_simple_equality() {
        let sql = "SELECT * FROM users WHERE status = $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users WHERE ($1 IS NULL OR status = $1)");
    }

    #[test]
    fn test_rewrite_qualified_column() {
        let sql = "SELECT * FROM users u WHERE u.status = $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users u WHERE ($1 IS NULL OR u.status = $1)");
    }

    #[test]
    fn test_rewrite_multiple_optional() {
        let sql = "SELECT * FROM users WHERE status = $1 AND name = $2";
        let params = vec![param("status", 1), param("name", 2)];
        let result = rewrite_optional_params(sql, &["status".to_string(), "name".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE ($1 IS NULL OR status = $1) AND ($2 IS NULL OR name = $2)"
        );
    }

    #[test]
    fn test_rewrite_mixed_optional_required() {
        let sql = "SELECT * FROM users WHERE id = $1 AND status = $2";
        let params = vec![param("id", 1), param("status", 2)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE id = $1 AND ($2 IS NULL OR status = $2)"
        );
    }

    #[test]
    fn test_rewrite_like_operator() {
        let sql = "SELECT * FROM users WHERE name LIKE $1";
        let params = vec![param("name", 1)];
        let result = rewrite_optional_params(sql, &["name".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users WHERE ($1 IS NULL OR name LIKE $1)");
    }

    #[test]
    fn test_rewrite_ilike_operator() {
        let sql = "SELECT * FROM users WHERE name ILIKE $1";
        let params = vec![param("name", 1)];
        let result = rewrite_optional_params(sql, &["name".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users WHERE ($1 IS NULL OR name ILIKE $1)");
    }

    #[test]
    fn test_rewrite_comparison_operators() {
        let sql = "SELECT * FROM users WHERE age >= $1";
        let params = vec![param("age", 1)];
        let result = rewrite_optional_params(sql, &["age".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users WHERE ($1 IS NULL OR age >= $1)");
    }

    #[test]
    fn test_rewrite_less_than() {
        let sql = "SELECT * FROM users WHERE age < $1";
        let params = vec![param("age", 1)];
        let result = rewrite_optional_params(sql, &["age".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users WHERE ($1 IS NULL OR age < $1)");
    }

    #[test]
    fn test_no_rewrite_without_optional() {
        let sql = "SELECT * FROM users WHERE status = $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &[], &params);
        assert_eq!(result, sql);
    }

    #[test]
    fn test_rewrite_not_equal() {
        let sql = "SELECT * FROM users WHERE status <> $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users WHERE ($1 IS NULL OR status <> $1)");
    }

    #[test]
    fn test_rewrite_does_not_match_similar_placeholder() {
        let sql = "SELECT * FROM users WHERE status = $10";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(result, sql);
    }

    #[test]
    fn test_normalize_engine_mariadb() {
        assert_eq!(normalize_engine("mariadb"), "mariadb");
    }

    #[test]
    fn test_get_backend_mariadb_with_mysql_backends() {
        let mariadb_backends = [
            "rust-sqlx",
            "python-aiomysql",
            "typescript-mysql2",
            "go-database-sql",
            "java-jdbc",
            "java-r2dbc",
            "kotlin-jdbc",
            "kotlin-r2dbc",
            "csharp-mysqlconnector",
            "elixir-myxql",
            "ruby-mysql2",
            "ruby-trilogy",
            "php-pdo",
            "php-amphp",
        ];
        for backend_name in &mariadb_backends {
            let result = get_backend(backend_name, "mariadb");
            assert!(
                result.is_ok(),
                "backend '{}' should accept mariadb engine, got: {:?}",
                backend_name,
                result.err()
            );
        }
    }

    // -----------------------------------------------------------------
    // SQL lexer / rewriting tests (#148, #186, #149-adjacent).
    //
    // These pin down the behaviour of `clean_sql`, `clean_sql_oneline`, and
    // `rewrite_pg_placeholders` against a real SQL lexer instead of the
    // character-scanning implementation these functions previously had.
    // Written before the lexer existed, to prove each defect is real.
    // -----------------------------------------------------------------

    #[test]
    fn test_clean_sql_oneline_strips_mid_line_comment_but_keeps_rest_of_query() {
        let sql = "SELECT id, name\nFROM users\nWHERE id > $1 -- skip low ids\n  AND name IS NOT NULL;";
        let result = clean_sql_oneline(sql);
        assert!(
            result.contains("AND name IS NOT NULL"),
            "mid-line comment must not swallow the rest of the query, got: {result:?}"
        );
        assert!(
            !result.contains("--"),
            "comment marker must be stripped, got: {result:?}"
        );
    }

    #[test]
    fn test_clean_sql_strips_mid_line_comment_but_keeps_rest_of_query() {
        let sql = "SELECT id\nFROM users\nWHERE id > $1 -- skip low ids\nAND active = true;";
        let result = clean_sql(sql);
        assert!(result.contains("AND active = true"), "got: {result:?}");
        assert!(!result.contains("--"), "got: {result:?}");
    }

    #[test]
    fn test_clean_sql_preserves_double_dash_inside_single_quoted_string() {
        let sql = "SELECT * FROM t WHERE label = 'a -- not a comment' AND id = $1;";
        let result = clean_sql_oneline(sql);
        assert!(result.contains("'a -- not a comment'"), "got: {result:?}");
        assert!(result.contains("id = $1"), "got: {result:?}");
    }

    #[test]
    fn test_clean_sql_preserves_double_dash_inside_double_quoted_identifier() {
        let sql = "SELECT \"weird--col\" FROM t WHERE id = $1;";
        let result = clean_sql_oneline(sql);
        assert!(result.contains("\"weird--col\""), "got: {result:?}");
    }

    #[test]
    fn test_clean_sql_preserves_double_dash_inside_dollar_quoted_string() {
        let sql = "SELECT $$has -- inside$$ AS x FROM t WHERE id = $1;";
        let result = clean_sql_oneline(sql);
        assert!(result.contains("$$has -- inside$$"), "got: {result:?}");
    }

    #[test]
    fn test_clean_sql_strips_nested_block_comments() {
        let sql = "SELECT id /* outer /* inner */ still outer */ FROM t WHERE id = $1;";
        let result = clean_sql_oneline(sql);
        assert!(!result.contains("outer"), "got: {result:?}");
        assert!(!result.contains("inner"), "got: {result:?}");
        assert!(result.contains("SELECT id"), "got: {result:?}");
        assert!(result.contains("FROM t WHERE id = $1"), "got: {result:?}");
    }

    #[test]
    fn test_clean_sql_unterminated_block_comment_consumes_to_end_without_panicking() {
        let sql = "SELECT id FROM t /* oops forgot to close";
        let result = clean_sql_oneline(sql);
        assert_eq!(result, "SELECT id FROM t");
    }

    #[test]
    fn test_clean_sql_drops_lines_that_are_only_a_header_comment() {
        // Regression guard: header comment lines (`-- @name ...`) must be
        // deleted entirely, not replaced by a blank line -- every generated
        // fixture depends on this.
        let sql = "-- @name Foo\n-- @returns :many\nSELECT id FROM t WHERE id = $1;";
        let result = clean_sql(sql);
        assert_eq!(result, "SELECT id FROM t WHERE id = $1");
    }

    #[test]
    fn test_rewrite_pg_placeholders_preserves_dollar_quoted_body() {
        let sql =
            "SELECT id FROM t WHERE body = $$literal -- not comment, ' quote, $1 not a placeholder$$ AND id = $2;";
        let result = rewrite_pg_placeholders(sql, |n| format!("@p{n}"));
        assert!(
            result.contains("$$literal -- not comment, ' quote, $1 not a placeholder$$"),
            "dollar-quoted body must survive untouched, got: {result:?}"
        );
        assert!(result.contains("id = @p2"), "got: {result:?}");
    }

    #[test]
    fn test_rewrite_pg_placeholders_handles_escaped_single_quotes() {
        let sql = "SELECT id FROM t WHERE label = 'it''s $1 not a placeholder' AND id = $2;";
        let result = rewrite_pg_placeholders(sql, |n| format!("@p{n}"));
        assert!(result.contains("'it''s $1 not a placeholder'"), "got: {result:?}");
        assert!(result.contains("id = @p2"), "got: {result:?}");
    }

    #[test]
    fn test_rewrite_pg_placeholders_preserves_jsonb_operators() {
        let sql = "SELECT id FROM docs WHERE data ? 'mykey' AND tags ?| array['a','b'] AND tags ?& array['a','b'] AND id = $1;";
        let result = rewrite_pg_placeholders(sql, |n| format!("@p{n}"));
        assert!(result.contains("data ? 'mykey'"), "got: {result:?}");
        assert!(result.contains("tags ?| array"), "got: {result:?}");
        assert!(result.contains("tags ?& array"), "got: {result:?}");
        assert!(result.contains("id = @p1"), "got: {result:?}");
    }

    /// Coordinator repro: a JSONB query with *zero* `$N` placeholders, so the
    /// `uses_dollar_placeholders` heuristic has nothing to anchor on. `?|`
    /// and `?&` are unambiguous -- never placeholder syntax in any dialect
    /// this crate targets -- so they must survive regardless of that
    /// heuristic. The bare `?` (`data ? 'active'`) is genuinely ambiguous
    /// without dialect information and is intentionally still rewritten
    /// here; see the residual-limitation comment on
    /// `rewrite_code_span_placeholders`.
    #[test]
    fn test_rewrite_pg_placeholders_preserves_jsonb_multi_char_operators_with_zero_dollar_placeholders() {
        let sql = "SELECT * FROM docs WHERE data ? 'active' AND tags ?| ARRAY['a'] AND meta ?& ARRAY['b']";
        let result = rewrite_pg_placeholders(sql, |n| format!("${n}"));
        assert!(result.contains("tags ?| ARRAY['a']"), "got: {result:?}");
        assert!(result.contains("meta ?& ARRAY['b']"), "got: {result:?}");
    }

    #[test]
    fn test_rewrite_pg_placeholders_preserves_geometry_operators() {
        let sql = "SELECT id FROM shapes WHERE line1 ?- line2 AND line1 ?-| line2 AND line1 ?|| line2 AND id = $1;";
        let result = rewrite_pg_placeholders(sql, |n| format!("@p{n}"));
        assert!(result.contains("line1 ?- line2"), "got: {result:?}");
        assert!(result.contains("line1 ?-| line2"), "got: {result:?}");
        assert!(result.contains("line1 ?|| line2"), "got: {result:?}");
        assert!(result.contains("id = @p1"), "got: {result:?}");
    }

    #[test]
    fn test_rewrite_pg_placeholders_preserves_operators_mixed_with_dollar_placeholder() {
        let sql = "SELECT id FROM docs WHERE tags ?| ARRAY['a'] AND doc @? '$.a' AND id = $1;";
        let result = rewrite_pg_placeholders(sql, |n| format!("@p{n}"));
        assert!(result.contains("tags ?| ARRAY['a']"), "got: {result:?}");
        assert!(result.contains("doc @? '$.a'"), "got: {result:?}");
        assert!(result.contains("id = @p1"), "got: {result:?}");
    }

    #[test]
    fn test_rewrite_pg_placeholders_number_boundary() {
        let sql = "SELECT $1, $10, $1x";
        let result = rewrite_pg_placeholders(sql, |n| format!("P{n}"));
        assert_eq!(result, "SELECT P1, P10, P1x");
    }

    #[test]
    fn test_rewrite_pg_placeholders_ignores_question_mark_inside_string_literal() {
        let sql = "SELECT id FROM t WHERE note = 'is this a question?' AND age > ?;";
        let result = rewrite_pg_placeholders(sql, |n| format!("${n}"));
        assert!(result.contains("'is this a question?'"), "got: {result:?}");
        assert!(result.contains("age > $1"), "got: {result:?}");
    }

    #[test]
    fn test_clean_sql_handles_non_ascii_in_comment_and_string() {
        let sql = "SELECT name -- 名前を取得 comment\nFROM t WHERE label = 'héllo wörld' AND id = $1;";
        let result = clean_sql_oneline(sql);
        assert!(!result.contains('名'), "got: {result:?}");
        assert!(result.contains("'héllo wörld'"), "got: {result:?}");
        assert!(result.contains("id = $1"), "got: {result:?}");
    }

    #[test]
    fn test_clean_sql_handles_crlf_line_endings() {
        let sql = "SELECT id\r\nFROM users\r\n-- a comment\r\nWHERE id = $1;";
        let result = clean_sql(sql);
        assert!(!result.contains('\r'), "got: {result:?}");
        assert!(!result.contains("-- a comment"), "got: {result:?}");
        assert_eq!(result, "SELECT id\nFROM users\nWHERE id = $1");
    }

    #[test]
    fn test_rewrite_optional_params_handles_not_like() {
        let sql = "SELECT id FROM t WHERE name NOT LIKE $1";
        let params = vec![param("name", 1)];
        let result = rewrite_optional_params(sql, &["name".to_string()], &params);
        assert_eq!(result, "SELECT id FROM t WHERE ($1 IS NULL OR name NOT LIKE $1)");
    }
}
