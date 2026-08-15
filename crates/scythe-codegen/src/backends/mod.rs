pub(crate) mod csharp_microsoft_sqlite;
pub(crate) mod csharp_mysqlconnector;
pub(crate) mod csharp_npgsql;
pub(crate) mod csharp_oracle;
pub(crate) mod csharp_snowflake;
pub(crate) mod csharp_sqlclient;
pub(crate) mod elixir_common;
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
use scythe_core::SqlDialect;
use scythe_core::analyzer::AnalyzedParam;
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::{CodegenBackend, ResolvedParam};

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
/// -- see #186 and board #148, where two independent character scanners each got
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
    /// A MySQL/MariaDB `` `...` `` quoted identifier, with `` `` `` as the
    /// escape for an embedded backtick. Only ever produced when the caller's
    /// dialect is [`SqlDialect::MySQL`] (board #148 item 2).
    Backtick,
    /// An MSSQL `[...]` delimited identifier, with `]]` as the escape for an
    /// embedded `]`. Only ever produced when the caller's dialect is
    /// [`SqlDialect::MsSql`] (board #148 item 4) -- gating on dialect is what keeps
    /// this from misreading PostgreSQL array-subscript syntax (`arr[1]`).
    Bracketed,
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
/// string/identifier/comment, or is it live SQL". `dialect` gates the lexical
/// forms that cannot be told apart from the text alone without knowing the
/// target engine: MySQL/MariaDB backtick identifiers and `#` line comments
/// (only under [`SqlDialect::MySQL`]), and MSSQL `[bracketed]` identifiers
/// (only under [`SqlDialect::MsSql`]) -- `#` in particular collides with
/// PostgreSQL's `#>`/`#>>` JSON operators, and `[`/`]` collide with
/// PostgreSQL array-subscript syntax, so neither can be recognized
/// dialect-blind (board #148).
fn tokenize_sql(sql: &str, dialect: SqlDialect) -> Vec<(SqlSpanKind, String)> {
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

        if c == '`' && dialect == SqlDialect::MySQL {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            i += 1;
            while i < len {
                if chars[i] == '`' {
                    if i + 1 < len && chars[i + 1] == '`' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push((SqlSpanKind::Backtick, chars[start..i].iter().collect()));
            continue;
        }

        if c == '#' && dialect == SqlDialect::MySQL {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            spans.push((SqlSpanKind::LineComment, chars[start..i].iter().collect()));
            continue;
        }

        if c == '[' && dialect == SqlDialect::MsSql {
            flush_code_buf(&mut code_buf, &mut spans);
            let start = i;
            i += 1;
            while i < len {
                if chars[i] == ']' {
                    if i + 1 < len && chars[i + 1] == ']' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push((SqlSpanKind::Bracketed, chars[start..i].iter().collect()));
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
///
/// Dialect-blind: kept for the call sites this multi-agent change did not
/// reach (see [`clean_sql_dialect`] for the dialect-aware replacement, which
/// every owned call site now uses). Passing `SqlDialect::PostgreSQL` into
/// [`tokenize_sql`] here gates off the MySQL backtick/`#` and MSSQL bracket
/// handling, reproducing this function's pre-board #148 behaviour exactly.
pub(crate) fn clean_sql(sql: &str) -> String {
    clean_sql_lines(sql, SqlDialect::PostgreSQL)
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Like clean_sql but joins lines with spaces (for languages that embed SQL inline).
/// Dialect-blind; see [`clean_sql`]'s doc comment.
pub(crate) fn clean_sql_oneline(sql: &str) -> String {
    clean_sql_lines(sql, SqlDialect::PostgreSQL)
        .join(" ")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Dialect-aware [`clean_sql`]: recognizes MySQL backtick identifiers/`#`
/// comments and MSSQL `[bracketed]` identifiers per `dialect` (board #148).
///
/// ~keep No production caller yet: the four backends migrated to the
/// dialect-aware pipeline all embed SQL as a single line and so reach for
/// [`clean_sql_oneline_dialect`]. The multiline form is what the ~44
/// still-unmigrated backends will need, and its tests below pin the MySQL
/// backtick / `#`-comment and MSSQL bracket behaviour that only this
/// signature exercises. `expect`, not `allow`, deliberately: it becomes a
/// hard error the moment a caller appears, so the attribute cannot outlive
/// the reason for it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "staged board #148 migration; see doc comment")
)]
pub(crate) fn clean_sql_dialect(sql: &str, dialect: SqlDialect) -> String {
    clean_sql_lines(sql, dialect)
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Dialect-aware [`clean_sql_oneline`]; see [`clean_sql_dialect`].
pub(crate) fn clean_sql_oneline_dialect(sql: &str, dialect: SqlDialect) -> String {
    clean_sql_lines(sql, dialect)
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
/// trailing comment (board #148) be stripped without deleting the rest of the
/// query, while a `-- @name Foo` header line still disappears instead of
/// leaving a blank line behind (every generated fixture depends on that).
fn clean_sql_lines(sql: &str, dialect: SqlDialect) -> Vec<String> {
    let normalized = sql.replace("\r\n", "\n");
    let spans = tokenize_sql(&normalized, dialect);

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
/// Dialect-blind; see [`clean_sql`]'s doc comment.
pub(crate) fn clean_sql_with_optional(sql: &str, optional_params: &[String], params: &[AnalyzedParam]) -> String {
    let cleaned = clean_sql(sql);
    rewrite_optional_params(&cleaned, optional_params, params)
}

/// Clean SQL (oneline) and apply optional parameter rewriting.
/// Dialect-blind; see [`clean_sql`]'s doc comment.
pub(crate) fn clean_sql_oneline_with_optional(
    sql: &str,
    optional_params: &[String],
    params: &[AnalyzedParam],
) -> String {
    let cleaned = clean_sql_oneline(sql);
    rewrite_optional_params(&cleaned, optional_params, params)
}

/// Dialect-aware [`clean_sql_oneline_with_optional`]; see [`clean_sql_dialect`].
pub(crate) fn clean_sql_oneline_with_optional_dialect(
    sql: &str,
    dialect: SqlDialect,
    optional_params: &[String],
    params: &[AnalyzedParam],
) -> String {
    let cleaned = clean_sql_oneline_dialect(sql, dialect);
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
/// Dialect-blind, kept for the call sites this multi-agent change did not
/// reach: whether a bare `?` is a placeholder is decided by scanning the code
/// spans for a `$`-followed-by-digit occurrence, and treating a bare `?` as a
/// placeholder only when none exists. That heuristic has a known gap --
/// [`rewrite_placeholders_indexed`]'s doc comment explains why, and is what
/// every owned call site now uses instead.
///
/// `?|`, `?&`, `?-`, `?-|`, `?||`, and `@?` are multi-character PostgreSQL
/// operators (JSONB key/path existence, geometric comparisons) that are
/// **never** valid placeholder syntax in any dialect this crate targets, so
/// [`rewrite_code_span_placeholders`] recognizes and skips them unconditionally
/// -- independent of the `$N` heuristic above, and correct even for a
/// zero-placeholder query that has no `$N` to anchor that heuristic on.
pub(crate) fn rewrite_pg_placeholders(sql: &str, formatter: impl Fn(u32) -> String) -> String {
    let spans = tokenize_sql(sql, SqlDialect::PostgreSQL);
    let uses_dollar_placeholders = spans
        .iter()
        .any(|(kind, text)| *kind == SqlSpanKind::Code && code_span_has_dollar_number(text));

    let mut result = String::with_capacity(sql.len());
    let mut positional_counter: u32 = 0;
    let mut occurrences: Vec<u32> = Vec::new();
    for (kind, text) in &spans {
        if *kind == SqlSpanKind::Code {
            rewrite_code_span_placeholders(
                text,
                !uses_dollar_placeholders,
                &formatter,
                &mut positional_counter,
                &mut result,
                &mut occurrences,
            );
        } else {
            result.push_str(text);
        }
    }
    result
}

/// Whether a bare `?` can ever be a placeholder under `dialect`, decided from
/// the dialect alone -- replacing the `$N`-occurrence heuristic in
/// [`rewrite_pg_placeholders`] (board #148 item 1, #186).
///
/// Verified against sqlparser 0.62's own tokenizer (`src/tokenizer.rs`,
/// the `'?' if self.dialect.supports_geometric_types()` arm, with the
/// source comment "Postgres uses ? for jsonb operators, not prepared
/// statements"): `PostgreSqlDialect` is the only builtin dialect that
/// overrides `supports_geometric_types()` to `true` (`dialect/postgresql.rs`),
/// so it is the only dialect whose tokenizer never emits `Token::Placeholder`
/// for a lone `?` -- it always emits `Token::Question` or one of the
/// JSONB/geometric operator tokens instead. A PostgreSQL query containing
/// `data ? 'active'` therefore never held a placeholder in the first place,
/// with or without another `$N` elsewhere in the query, and no
/// parameter-count heuristic is needed to tell the two apart. Every other
/// dialect this crate targets (MySQL, SQLite, MsSql, Oracle, Snowflake) uses
/// bare `?` as its native positional placeholder syntax and has no
/// JSONB-style operator that collides with it.
fn dialect_allows_bare_question_mark_placeholder(dialect: SqlDialect) -> bool {
    !matches!(dialect, SqlDialect::PostgreSQL)
}

/// Dialect-aware rewrite of SQL placeholders (`$N` or bare `?`), returning
/// both the rewritten SQL and, for each placeholder occurrence rewritten (in
/// textual order, including repeats), the `$N`/`?N` position number it
/// resolved to.
///
/// Unlike [`rewrite_pg_placeholders`], whether a bare `?` is a placeholder is
/// decided from `dialect` alone via
/// [`dialect_allows_bare_question_mark_placeholder`], never from whether the
/// query happens to also contain a `$N`.
///
/// The returned position list is what lets a caller bind per SQL-text
/// occurrence instead of per declared parameter (GH #149): `$2 ... $1`
/// binds out of declaration order, a repeated `$1` produces two occurrences
/// of position `1`, and an `@optional` rewrite (`($1 IS NULL OR col = $1)`)
/// likewise produces two occurrences of the same position -- all three need
/// one bind per occurrence, not one bind per unique parameter, or the
/// generated code either swaps arguments or throws a parameter-count
/// mismatch at runtime.
pub(crate) fn rewrite_placeholders_indexed(
    sql: &str,
    dialect: SqlDialect,
    formatter: impl Fn(u32) -> String,
) -> (String, Vec<u32>) {
    let spans = tokenize_sql(sql, dialect);
    let bare_question_is_placeholder = dialect_allows_bare_question_mark_placeholder(dialect);

    let mut result = String::with_capacity(sql.len());
    let mut positional_counter: u32 = 0;
    let mut occurrences: Vec<u32> = Vec::new();
    for (kind, text) in &spans {
        if *kind == SqlSpanKind::Code {
            rewrite_code_span_placeholders(
                text,
                bare_question_is_placeholder,
                &formatter,
                &mut positional_counter,
                &mut result,
                &mut occurrences,
            );
        } else {
            result.push_str(text);
        }
    }
    (result, occurrences)
}

/// Resolve the [`ResolvedParam`] that a [`rewrite_placeholders_indexed`]
/// occurrence position refers to.
///
/// `resolve::resolve_params` builds `resolved` from `analyzed_params` in the
/// same order (one output per input, positionally), so the index into one
/// slice is the index into the other; `analyzed_params[i].position` is the
/// actual `$N` the analyzer resolved parameter `i` to. This is what lets a
/// backend bind an occurrence's own position back to a concrete parameter
/// instead of assuming SQL-declaration order matches bind order (#149): a
/// repeated `$1`, an out-of-order `$2 ... $1`, and an `@optional` rewrite all
/// resolve correctly through this lookup because it is driven by the
/// occurrence's own position, never by loop index.
///
/// Panics if `position` is not among `analyzed_params` -- every position
/// [`rewrite_placeholders_indexed`] returns came from the same SQL text the
/// analyzer walked to build `analyzed_params`, so a miss here means the two
/// disagree about what that SQL contains, which is a scythe bug, not
/// something a caller can recover from.
pub(crate) fn resolved_param_for_position<'a>(
    analyzed_params: &[AnalyzedParam],
    resolved: &'a [ResolvedParam],
    position: u32,
) -> &'a ResolvedParam {
    let idx = analyzed_params
        .iter()
        .position(|p| p.position == i64::from(position))
        .unwrap_or_else(|| panic!("no analyzed parameter at position {position}; analyzed_params={analyzed_params:?}"));
    &resolved[idx]
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

/// Rewrite `$N` and (when `bare_question_is_placeholder` is true) bare `?`
/// placeholders within a single `Code` span, appending the result to `out`,
/// advancing `counter` for each `?` consumed, and recording each rewritten
/// occurrence's resolved position (in output order) into `occurrences`.
///
/// Shared by [`rewrite_pg_placeholders`] (which computes
/// `bare_question_is_placeholder` from the `$N`-occurrence heuristic and
/// discards `occurrences`) and [`rewrite_placeholders_indexed`] (which
/// computes it from the dialect and returns `occurrences`) -- the actual
/// rewriting is one implementation; only how the two callers decide whether a
/// bare `?` counts differs. The multi-character operators in
/// [`UNAMBIGUOUS_OPERATORS`] have no such ambiguity and are always passed
/// through, regardless of `bare_question_is_placeholder`.
fn rewrite_code_span_placeholders(
    text: &str,
    bare_question_is_placeholder: bool,
    formatter: &impl Fn(u32) -> String,
    counter: &mut u32,
    out: &mut String,
    occurrences: &mut Vec<u32>,
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
            occurrences.push(num);
            i = j;
        } else if ch == '?' && bare_question_is_placeholder && !chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
            *counter += 1;
            out.push_str(&formatter(*counter));
            occurrences.push(*counter);
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
        "javascript-node-sqlite" => Box::new(typescript_node_sqlite::TypescriptNodeSqliteBackend::new_js(
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
            source_relation: None,
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
    // SQL lexer / rewriting tests (board #148, #186, #149-adjacent).
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

    // -----------------------------------------------------------------
    // Dialect-aware tokenizer / placeholder tests (board #148, GH #149).
    //
    // These exercise `rewrite_placeholders_indexed` and `clean_sql_dialect`/
    // `clean_sql_oneline_dialect` directly -- the dialect-driven replacements
    // every owned backend (java-jdbc, kotlin-jdbc, kotlin-exposed, php-amphp)
    // now calls instead of the heuristic-based `rewrite_pg_placeholders`/
    // `clean_sql`/`clean_sql_oneline`. Each test states, in its doc comment or
    // inline, what the OLD (heuristic-based, dialect-blind) code produced for
    // the same input.
    // -----------------------------------------------------------------

    /// The headline board #148 defect, reproduced against both the OLD heuristic
    /// entry point and the NEW dialect-driven one on the exact same input.
    /// `rewrite_pg_placeholders` (unchanged, kept for callers this change did
    /// not reach) has no `$N` to anchor its heuristic on, so it guesses "no
    /// dollar placeholders exist -> bare `?` is positional" and corrupts the
    /// JSONB key-existence operator into a placeholder reference:
    /// `WHERE data $1 'active'`. `rewrite_placeholders_indexed` under
    /// `SqlDialect::PostgreSQL` is told the dialect directly and never
    /// rewrites it, with zero occurrences recorded.
    #[test]
    fn test_bare_question_mark_is_never_a_placeholder_under_postgresql_even_with_zero_dollar_placeholders() {
        let sql = "SELECT * FROM docs WHERE data ? 'active'";

        let old_result = rewrite_pg_placeholders(sql, |n| format!("${n}"));
        assert_eq!(
            old_result, "SELECT * FROM docs WHERE data $1 'active'",
            "pins the OLD heuristic's corruption of the JSONB operator"
        );

        let (new_result, occurrences) = rewrite_placeholders_indexed(sql, SqlDialect::PostgreSQL, |n| format!("${n}"));
        assert_eq!(
            new_result, sql,
            "dialect-driven rewrite must leave the JSONB operator untouched"
        );
        assert!(
            occurrences.is_empty(),
            "no placeholder was ever present; got: {occurrences:?}"
        );
    }

    /// Same bare `?`, but under a dialect that genuinely uses it as a
    /// positional placeholder -- proving the decision is dialect-driven, not
    /// "always leave `?` alone now".
    #[test]
    fn test_bare_question_mark_is_a_placeholder_under_mysql() {
        let sql = "SELECT * FROM docs WHERE data ? 'active'";
        let (result, occurrences) = rewrite_placeholders_indexed(sql, SqlDialect::MySQL, |n| format!("${n}"));
        assert_eq!(result, "SELECT * FROM docs WHERE data $1 'active'");
        assert_eq!(occurrences, vec![1]);
    }

    /// GH #149: `$2` declared before `$1` in the SQL text must report its
    /// own position at each occurrence, in textual order -- this is what lets
    /// a backend bind per SQL-text occurrence instead of assuming declaration
    /// order.
    #[test]
    fn test_rewrite_placeholders_indexed_returns_positions_in_textual_order_out_of_order() {
        let (result, occurrences) =
            rewrite_placeholders_indexed("WHERE b = $2 AND a = $1", SqlDialect::PostgreSQL, |_| "?".to_string());
        assert_eq!(result, "WHERE b = ? AND a = ?");
        assert_eq!(occurrences, vec![2, 1]);
    }

    /// GH #149: a repeated `$1` must report position `1` twice, once per
    /// occurrence -- not collapse to a single entry the way a per-declared-
    /// parameter bind loop would.
    #[test]
    fn test_rewrite_placeholders_indexed_repeated_placeholder_reports_one_occurrence_per_use() {
        let (result, occurrences) =
            rewrite_placeholders_indexed("WHERE a = $1 OR b = $1", SqlDialect::PostgreSQL, |_| "?".to_string());
        assert_eq!(result, "WHERE a = ? OR b = ?");
        assert_eq!(occurrences, vec![1, 1]);
    }

    /// board #148 item 2/#186: a MySQL backtick-quoted identifier containing a bare
    /// `?` must not have the `?` inside it rewritten, and the identifier's own
    /// characters must not desynchronize the *later* placeholder's number.
    /// Matches the `WRONG:`-documented expectation in
    /// `sql_text_characterization_tests.rs`'s
    /// `question_mark_inside_a_mysql_backtick_identifier_is_rewritten_as_a_placeholder`.
    #[test]
    fn test_mysql_backtick_identifier_is_not_corrupted_by_bare_question_mark() {
        let sql = "SELECT `a?b` FROM t WHERE c = ?";
        let (result, occurrences) = rewrite_placeholders_indexed(sql, SqlDialect::MySQL, |n| format!("[P{n}]"));
        assert_eq!(result, "SELECT `a?b` FROM t WHERE c = [P1]");
        assert_eq!(occurrences, vec![1]);
    }

    /// board #148 item 4: same as above for an MSSQL `[bracketed]` identifier.
    #[test]
    fn test_mssql_bracket_identifier_is_not_corrupted_by_bare_question_mark() {
        let sql = "SELECT [a?b] FROM t WHERE c = ?";
        let (result, occurrences) = rewrite_placeholders_indexed(sql, SqlDialect::MsSql, |n| format!("[P{n}]"));
        assert_eq!(result, "SELECT [a?b] FROM t WHERE c = [P1]");
        assert_eq!(occurrences, vec![1]);
    }

    /// board #148 item 2: a MySQL backtick identifier containing `-- ` must not be
    /// read as opening a line comment. Matches the `clean_sql_dialect`
    /// counterpart of the `WRONG:`-documented
    /// `clean_sql_treats_a_double_dash_inside_a_mysql_backtick_identifier_as_a_comment`
    /// characterization test (which pins the OLD, dialect-blind `clean_sql`'s
    /// corruption of the same input to `"SELECT \`a"`).
    #[test]
    fn test_clean_sql_dialect_mysql_backtick_preserves_double_dash_inside() {
        let sql = "SELECT `a -- b` FROM t";
        assert_eq!(
            clean_sql(sql),
            "SELECT `a",
            "pins the OLD dialect-blind clean_sql's corruption"
        );
        assert_eq!(clean_sql_dialect(sql, SqlDialect::MySQL), sql);
    }

    /// board #148 item 4: same as above for an MSSQL `[bracketed]` identifier.
    #[test]
    fn test_clean_sql_dialect_mssql_bracket_preserves_double_dash_inside() {
        let sql = "SELECT [a -- b] FROM t";
        assert_eq!(
            clean_sql(sql),
            "SELECT [a",
            "pins the OLD dialect-blind clean_sql's corruption"
        );
        assert_eq!(clean_sql_dialect(sql, SqlDialect::MsSql), sql);
    }

    /// board #148 item 3: MySQL `#` line comments were deliberately unrecognized
    /// dialect-blind (`#` collides with PostgreSQL's `#>`/`#>>` JSON
    /// operators), so the OLD `clean_sql` leaves the whole comment in the
    /// output. Under `SqlDialect::MySQL` it must be stripped like `--`.
    #[test]
    fn test_clean_sql_dialect_mysql_hash_comment_is_stripped() {
        let sql = "SELECT 1 # note\nFROM t";
        assert_eq!(
            clean_sql(sql),
            "SELECT 1 # note\nFROM t",
            "pins the OLD dialect-blind clean_sql leaving the # comment in place"
        );
        // WRONG-adjacent (matches the pre-existing `--` wart pinned by
        // `clean_sql_strips_a_trailing_line_comment_but_leaves_the_space_before_it`
        // in sql_text_characterization_tests.rs): stripping a trailing
        // comment does not also strip the space that preceded it.
        assert_eq!(clean_sql_dialect(sql, SqlDialect::MySQL), "SELECT 1 \nFROM t");
    }

    /// The oneline variant of the `#` test above, since every owned backend
    /// calls `clean_sql_oneline_dialect` (or its `_with_optional` wrapper),
    /// never `clean_sql_dialect`, to flatten SQL into one embedded literal.
    #[test]
    fn test_clean_sql_oneline_dialect_mysql_hash_comment_is_stripped() {
        let sql = "SELECT 1 # note\nFROM t";
        assert_eq!(clean_sql_oneline_dialect(sql, SqlDialect::MySQL), "SELECT 1  FROM t");
    }

    /// PostgreSQL's `#>`/`#>>` JSON operators are exactly why `#` cannot be
    /// treated as a comment dialect-blind -- confirm they still survive
    /// untouched under `SqlDialect::PostgreSQL`.
    #[test]
    fn test_clean_sql_dialect_postgresql_hash_is_not_a_comment() {
        let sql = "SELECT data #> '{a,b}' AS x FROM t";
        assert_eq!(clean_sql_dialect(sql, SqlDialect::PostgreSQL), sql);
    }
}
