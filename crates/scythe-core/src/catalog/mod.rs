mod fingerprint;
pub(crate) mod type_normalizer;
mod view_resolver;

use ahash::AHashMap;
use sqlparser::ast::{
    AlterColumnOperation, AlterTableOperation, AlterTypeOperation, ColumnOption, DataType, Expr, ObjectName, Statement,
    TableConstraint, UserDefinedTypeRepresentation,
};
use sqlparser::parser::Parser;

use crate::dialect::SqlDialect;
use crate::errors::ScytheError;

use type_normalizer::{bare_name, ident_to_lower, normalize_data_type, object_name_to_key, object_name_to_raw_name};

#[derive(Debug)]
pub struct Catalog {
    tables: AHashMap<String, Table>,
    enums: AHashMap<String, EnumType>,
    composites: AHashMap<String, CompositeType>,
    /// Domain name -> resolved base type (lowercase)
    domains: AHashMap<String, DomainDef>,
    /// SQL dialect this catalog was parsed with. Used downstream to resolve
    /// dialect-specific type semantics (e.g. SQLite's `REAL` is an 8-byte
    /// IEEE float, unlike PostgreSQL's 4-byte `real`).
    dialect: SqlDialect,
    /// Configured engine name (`scythe.toml`'s `[[sql]] engine`) when the
    /// caller knows it, e.g. `"postgresql"`, `"redshift"`, `"duckdb"`.
    ///
    /// `SqlDialect` deliberately collapses every PostgreSQL-compatible
    /// engine onto [`SqlDialect::PostgreSQL`] — `SqlDialect::from_str` maps
    /// `redshift`, `duckdb` and `cockroachdb` all to that one variant —
    /// because for *parsing and type resolution* they behave the same. That
    /// collapse is wrong for capability questions: Redshift has no
    /// `json_agg` at all and DuckDB spells it `json_group_array`, so a
    /// dialect-only gate silently lets both through. Inference that depends
    /// on a server-side function actually existing must consult this, not
    /// the dialect.
    ///
    /// `None` means "not stated" and is treated as the dialect's flagship
    /// engine, which keeps every [`Catalog::from_ddl`] caller (all of the
    /// unit tests, and any embedder that never had an engine string)
    /// behaving exactly as it did before this field existed.
    engine: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DomainDef {
    pub(crate) base_type: String,
    pub(crate) not_null: bool,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub columns: Vec<Column>,
    /// The table (or view) name exactly as the DDL spelled its final
    /// identifier part, independent of the catalog's lowercase lookup key.
    ///
    /// This exists because [`Catalog`]'s tables map is keyed by
    /// [`object_name_to_key`], which lowercases every identifier
    /// unconditionally -- quoted or not -- so that lookups stay
    /// case-insensitive the way every supported SQL dialect treats
    /// unqualified references. That lowercasing is correct for lookup but
    /// destroys the one signal a naming-convention lint needs: whether the
    /// author actually wrote `"UserProfile"`.
    ///
    /// `raw_name` is populated via [`object_name_to_raw_name`], which
    /// mirrors [`ident_to_lower`]'s quote-aware rule: a quoted identifier
    /// keeps its literal casing, an unquoted one is folded to lowercase (the
    /// same case-folding every dialect here applies to unquoted
    /// identifiers). Concretely: for a bare `CREATE TABLE user_profiles`,
    /// `raw_name` is `"user_profiles"` -- identical to the lookup key's bare
    /// name, so a lint keyed on `raw_name` behaves exactly as it would
    /// against the normalised key. It only diverges, and only becomes
    /// interesting, when the DDL quoted a mixed-case or uppercase
    /// identifier, e.g. `CREATE TABLE "UserProfile"` yields
    /// `raw_name == "UserProfile"` while the lookup key is
    /// `"userprofile"`.
    ///
    /// Always the bare (unqualified) name, never schema-qualified, since
    /// every current consumer (SC-N02) only ever checks bare table casing.
    pub raw_name: String,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

#[derive(Debug, Clone)]
pub struct EnumType {
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CompositeType {
    pub fields: Vec<CompositeField>,
}

#[derive(Debug, Clone)]
pub struct CompositeField {
    pub name: String,
    pub sql_type: String,
}

impl Catalog {
    pub fn from_ddl(schema_sql: &[&str]) -> Result<Catalog, ScytheError> {
        Self::from_ddl_with_dialect(schema_sql, &SqlDialect::PostgreSQL)
    }

    pub fn from_ddl_with_dialect(schema_sql: &[&str], dialect: &SqlDialect) -> Result<Catalog, ScytheError> {
        let mut catalog = Catalog {
            tables: AHashMap::new(),
            enums: AHashMap::new(),
            composites: AHashMap::new(),
            domains: AHashMap::new(),
            dialect: *dialect,
            engine: None,
        };

        let parser_dialect = dialect.to_sqlparser_dialect();

        for sql in schema_sql {
            let filtered = Self::strip_psql_meta_commands(sql);
            let cleaned = catalog.extract_unsupported_statements(&filtered, dialect);

            let trimmed = cleaned.trim();
            if trimmed.is_empty() {
                continue;
            }

            let statements =
                Parser::parse_sql(parser_dialect.as_ref(), &cleaned).map_err(|e| ScytheError::syntax(e.to_string()))?;

            for stmt in statements {
                catalog.process_statement(stmt, dialect)?;
            }
        }

        Ok(catalog)
    }

    /// The SQL dialect this catalog was parsed with.
    pub(crate) fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    /// Record the configured engine name for this catalog. See the `engine`
    /// field for why this is tracked separately from [`SqlDialect`].
    ///
    /// Consumed builder-style so a call site that already has the engine
    /// string can attach it in the same expression that builds the catalog.
    #[must_use]
    pub fn with_engine(mut self, engine: &str) -> Self {
        self.engine = Some(engine.to_lowercase());
        self
    }

    /// The configured engine name, or `None` when the caller never stated
    /// one. See the `engine` field.
    pub(crate) fn engine(&self) -> Option<&str> {
        self.engine.as_deref()
    }

    pub fn get_table(&self, name: &str) -> Option<&Table> {
        lookup_qualified(&self.tables, name)
    }

    pub fn get_enum(&self, name: &str) -> Option<&EnumType> {
        lookup_qualified(&self.enums, name)
    }

    /// Iterate over all table names in the catalog.
    pub fn tables(&self) -> impl Iterator<Item = &String> {
        self.tables.keys()
    }

    /// Iterate over all `(name, table)` pairs in the catalog. Useful for
    /// consumers that need to walk every table's schema (e.g. auto-CRUD
    /// generators).
    pub fn tables_iter(&self) -> impl Iterator<Item = (&String, &Table)> {
        self.tables.iter()
    }

    /// Iterate over all enum names in the catalog.
    pub fn enums_iter(&self) -> impl Iterator<Item = (&String, &EnumType)> {
        self.enums.iter()
    }

    /// Iterate over all `(name, composite)` pairs in the catalog. Mirrors
    /// [`Self::enums_iter`] -- added so callers (in particular the
    /// fixture-generated catalog tests, see #161) can assert a *total*
    /// composite count instead of only checking that specific composites
    /// exist, which left extra, unexpected composites invisible.
    pub fn composites_iter(&self) -> impl Iterator<Item = (&String, &CompositeType)> {
        self.composites.iter()
    }

    /// Look up a domain's resolved base type by name.
    pub fn get_domain_base_type(&self, name: &str) -> Option<&str> {
        lookup_qualified(&self.domains, name).map(|d| d.base_type.as_str())
    }

    pub fn get_composite(&self, name: &str) -> Option<&CompositeType> {
        lookup_qualified(&self.composites, name)
    }
}

impl Catalog {
    /// Pre-process a SQL string to extract statements that sqlparser cannot handle
    /// (CREATE DOMAIN, CREATE SCHEMA, Oracle CREATE SEQUENCE / CREATE TRIGGER).
    /// Processes them internally and returns the remaining SQL with those
    /// statements removed.
    fn extract_unsupported_statements(&mut self, sql: &str, dialect: &SqlDialect) -> String {
        let mut result = String::with_capacity(sql.len());
        let is_oracle_slash_script = *dialect == SqlDialect::Oracle && sql.lines().any(|line| line.trim() == "/");
        let raw_statements = if is_oracle_slash_script {
            Self::split_oracle_slash_statements(sql)
        } else {
            Self::split_top_level_statements(sql)
        };
        for raw_stmt in raw_statements {
            let trimmed = raw_stmt.trim();
            if trimmed.is_empty() || trimmed.starts_with("--") && !trimmed.contains('\n') {
                result.push_str(raw_stmt);
                continue;
            }
            let no_comments = Self::strip_leading_comments(trimmed);
            let upper = no_comments.to_uppercase();
            if upper.starts_with("CREATE DOMAIN") {
                self.try_parse_create_domain(no_comments, dialect);
            } else if upper.starts_with("CREATE SCHEMA") {
            } else if *dialect == SqlDialect::Oracle
                && (upper.starts_with("CREATE SEQUENCE") || Self::is_oracle_create_trigger(&upper))
            {
                // Sequences contribute no columns or types to the catalog, and
                // Oracle allows their START WITH / INCREMENT BY options in any
                // order, which sqlparser's positional option parser rejects.
                // Trigger bodies use PL/SQL (`:NEW.col`, BEGIN/END blocks) that
                // sqlparser does not parse at all. Both are dropped precisely
                // here rather than handed to the parser.
            } else {
                let stmt_to_add = if matches!(dialect, SqlDialect::PostgreSQL | SqlDialect::MsSql) {
                    Self::strip_identity_patterns(raw_stmt)
                } else {
                    raw_stmt.to_string()
                };
                result.push_str(&stmt_to_add);
                if !stmt_to_add.ends_with(';') {
                    result.push(';');
                }
            }
        }
        result
    }

    /// Strip IDENTITY(seed,step) patterns from SQL for Redshift/MSSQL compatibility.
    /// Redshift uses IDENTITY(1,1) syntax which PostgreSQL parser doesn't recognize.
    /// This removes those patterns, converting columns to plain type WITHOUT the IDENTITY clause.
    ///
    /// Operates on `char_indices()` rather than raw bytes: pushing a raw
    /// UTF-8 byte as if it were a Latin-1 code point (`bytes[i] as char`)
    /// reinterprets every multi-byte character one byte at a time, mojibaking
    /// any non-ASCII identifier, comment, or string literal the statement
    /// contains. It also tracks single-quoted string, double-quoted
    /// identifier, and comment state so an `IDENTITY(` spelled inside a
    /// literal (e.g. a default value `'identity(x)'`) is copied through
    /// verbatim rather than matched as the keyword. See issue #181.
    fn strip_identity_patterns(sql: &str) -> String {
        let chars: Vec<(usize, char)> = sql.char_indices().collect();
        let len = chars.len();
        let mut result = String::with_capacity(sql.len());
        let mut i = 0usize;
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut in_line_comment = false;
        let mut in_block_comment = false;

        while i < len {
            let ch = chars[i].1;

            if in_line_comment {
                result.push(ch);
                if ch == '\n' {
                    in_line_comment = false;
                }
                i += 1;
                continue;
            }
            if in_block_comment {
                result.push(ch);
                if ch == '*' && i + 1 < len && chars[i + 1].1 == '/' {
                    result.push('/');
                    in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if in_single_quote {
                result.push(ch);
                if ch == '\'' {
                    if i + 1 < len && chars[i + 1].1 == '\'' {
                        result.push('\'');
                        i += 2;
                    } else {
                        in_single_quote = false;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
                continue;
            }
            if in_double_quote {
                result.push(ch);
                if ch == '"' {
                    in_double_quote = false;
                }
                i += 1;
                continue;
            }

            match ch {
                '\'' => {
                    in_single_quote = true;
                    result.push(ch);
                    i += 1;
                    continue;
                }
                '"' => {
                    in_double_quote = true;
                    result.push(ch);
                    i += 1;
                    continue;
                }
                '-' if i + 1 < len && chars[i + 1].1 == '-' => {
                    in_line_comment = true;
                    result.push_str("--");
                    i += 2;
                    continue;
                }
                '/' if i + 1 < len && chars[i + 1].1 == '*' => {
                    in_block_comment = true;
                    result.push_str("/*");
                    i += 2;
                    continue;
                }
                _ => {}
            }

            if ch.is_ascii_alphabetic() && Self::matches_identity_keyword(&chars, i) {
                let is_start_boundary = i == 0 || {
                    let prev = chars[i - 1].1;
                    !(prev.is_ascii_alphanumeric() || prev == '_')
                };
                if is_start_boundary {
                    let mut j = i + 8;
                    while j < len && chars[j].1.is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < len && chars[j].1 == '(' {
                        let mut k = j + 1;
                        let mut found_valid_pattern = false;

                        while k < len && chars[k].1.is_ascii_whitespace() {
                            k += 1;
                        }
                        let num_start = k;
                        while k < len && chars[k].1.is_ascii_digit() {
                            k += 1;
                        }
                        if k > num_start {
                            while k < len && chars[k].1.is_ascii_whitespace() {
                                k += 1;
                            }
                            if k < len && chars[k].1 == ',' {
                                k += 1;
                                while k < len && chars[k].1.is_ascii_whitespace() {
                                    k += 1;
                                }
                                let num_start2 = k;
                                while k < len && chars[k].1.is_ascii_digit() {
                                    k += 1;
                                }
                                if k > num_start2 {
                                    while k < len && chars[k].1.is_ascii_whitespace() {
                                        k += 1;
                                    }
                                    if k < len && chars[k].1 == ')' {
                                        i = k + 1;
                                        found_valid_pattern = true;
                                    }
                                }
                            }
                        }

                        if !found_valid_pattern {
                            result.push_str("IDENTITY(");
                            // `j` is the position of the `(` we just emitted;
                            // advance one char past it. The previous version
                            // advanced 9 bytes from this same position, which
                            // deleted up to 8 bytes of legitimate source past
                            // the `(` -- see #181.
                            i = j + 1;
                        }
                    } else {
                        for &(_, keyword_ch) in &chars[i..i + 8] {
                            result.push(keyword_ch);
                        }
                        i += 8;
                    }
                    continue;
                }
            }

            result.push(ch);
            i += 1;
        }

        result
    }

    /// Check if chars at position i match the IDENTITY keyword (case-insensitive, ASCII only)
    fn matches_identity_keyword(chars: &[(usize, char)], i: usize) -> bool {
        const IDENTITY: &str = "IDENTITY";
        if i + 8 > chars.len() {
            return false;
        }
        chars[i..i + 8]
            .iter()
            .zip(IDENTITY.chars())
            .all(|(&(_, c), k)| c.to_ascii_uppercase() == k)
    }

    /// Split SQL text into top-level statements by semicolons, preserving
    /// the semicolons and whitespace in the returned fragments.
    fn split_top_level_statements(sql: &str) -> Vec<&str> {
        let mut statements = Vec::new();
        let mut start = 0;
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        let bytes = sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if in_line_comment {
                if bytes[i] == b'\n' {
                    in_line_comment = false;
                }
                i += 1;
                continue;
            }
            if in_block_comment {
                if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if in_single_quote {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        i += 2;
                    } else {
                        in_single_quote = false;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
                continue;
            }
            if in_double_quote {
                if bytes[i] == b'"' {
                    in_double_quote = false;
                }
                i += 1;
                continue;
            }
            match bytes[i] {
                b'\'' => {
                    in_single_quote = true;
                    i += 1;
                }
                b'"' => {
                    in_double_quote = true;
                    i += 1;
                }
                b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                    in_line_comment = true;
                    i += 2;
                }
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                    in_block_comment = true;
                    i += 2;
                }
                b';' => {
                    statements.push(&sql[start..=i]);
                    start = i + 1;
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
        if start < sql.len() {
            let remainder = &sql[start..];
            if !remainder.trim().is_empty() {
                statements.push(remainder);
            }
        }
        statements
    }

    /// Split an Oracle SQL*Plus script into top-level statements delimited by
    /// a line containing only `/` (the conventional SQL*Plus statement
    /// terminator). Unlike [`split_top_level_statements`], this does not treat
    /// `;` as a boundary: PL/SQL blocks (e.g. `CREATE TRIGGER ... BEGIN ...
    /// END;`) contain semicolons internally, and only the standalone `/` line
    /// marks the end of the statement.
    fn split_oracle_slash_statements(sql: &str) -> Vec<&str> {
        let mut statements = Vec::new();
        let mut block_start = 0usize;
        let mut offset = 0usize;
        for line in sql.split_inclusive('\n') {
            if line.trim() == "/" {
                let block = &sql[block_start..offset];
                if !block.trim().is_empty() {
                    statements.push(block);
                }
                block_start = offset + line.len();
            }
            offset += line.len();
        }
        if block_start < sql.len() {
            let remainder = &sql[block_start..];
            if !remainder.trim().is_empty() {
                statements.push(remainder);
            }
        }
        statements
    }

    /// True if `upper` (an already-uppercased, comment-stripped statement) is
    /// a `CREATE TRIGGER` or `CREATE OR REPLACE TRIGGER` statement.
    fn is_oracle_create_trigger(upper: &str) -> bool {
        upper.starts_with("CREATE TRIGGER") || upper.starts_with("CREATE OR REPLACE TRIGGER")
    }

    /// Remove psql client meta-command lines from a SQL string.
    ///
    /// `pg_dump 18+` and tools such as `dbmate` emit lines like
    /// `\restrict <token>` and `\unrestrict <token>` that are psql client
    /// directives, not SQL.  `sqlparser` rejects any token starting with `\`,
    /// so we strip those lines before handing the text to the parser.
    ///
    /// Only lines whose **first non-whitespace character** is `\` are removed.
    /// Each dropped line is replaced with an empty line so that error
    /// line-number offsets remain meaningful.  No `\connect`, `\i`, `\copy`,
    /// or `\set` semantics are interpreted — the lines are simply discarded.
    fn strip_psql_meta_commands(sql: &str) -> String {
        let mut out = String::with_capacity(sql.len());
        for line in sql.split('\n') {
            if line.trim_start().starts_with('\\') {
                out.push('\n');
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        if !sql.ends_with('\n') && out.ends_with('\n') {
            out.pop();
        }
        out
    }

    /// Strip leading SQL comments (-- and /* */) from a string.
    fn strip_leading_comments(s: &str) -> &str {
        let mut rest = s;
        loop {
            rest = rest.trim_start();
            if rest.starts_with("--") {
                if let Some(nl) = rest.find('\n') {
                    rest = &rest[nl + 1..];
                } else {
                    return "";
                }
            } else if rest.starts_with("/*") {
                if let Some(end) = rest.find("*/") {
                    rest = &rest[end + 2..];
                } else {
                    return "";
                }
            } else {
                return rest;
            }
        }
    }

    /// Try to parse `CREATE DOMAIN <name> AS <type> [NOT NULL] [CHECK ...]`.
    /// Returns true if the SQL was a CREATE DOMAIN statement (even if parsing
    /// was only partial).
    fn try_parse_create_domain(&mut self, sql: &str, dialect: &SqlDialect) -> bool {
        let trimmed = sql.trim();
        if !trimmed.to_ascii_uppercase().starts_with("CREATE DOMAIN") {
            return false;
        }
        let trimmed = trimmed.trim_end_matches(';').trim();
        // `to_ascii_uppercase` (never `to_uppercase`) is required here: it
        // only rewrites the ASCII a-z range, one byte for one byte, so
        // `upper` is guaranteed to have exactly `trimmed`'s byte length and
        // char boundaries. Every byte offset found below is computed on
        // `upper` and then used to slice `trimmed` directly.
        // `str::to_uppercase()` cannot make that guarantee -- full Unicode
        // case folding can change a character's byte length (e.g. U+FB01
        // "ﬁ" becomes the two-byte-longer "FI"), which silently produced
        // offsets that landed inside a multi-byte character and panicked.
        // See issue #184.
        let upper = trimmed.to_ascii_uppercase();

        let name_start = skip_ws(&upper, "CREATE DOMAIN".len());
        let name_end = find_ws(&upper, name_start);
        if name_end <= name_start {
            return true;
        }
        let domain_name = trimmed[name_start..name_end].trim().to_lowercase();

        let after_name = skip_ws(&upper, name_end);
        // PostgreSQL's `AS` between the domain name and its base type is
        // optional (`CREATE DOMAIN x TEXT NOT NULL;` is valid) -- see #166
        // and #184.
        let has_as = upper[after_name..].starts_with("AS")
            && upper
                .as_bytes()
                .get(after_name + 2)
                .is_none_or(|b| b.is_ascii_whitespace());
        let type_start = if has_as {
            skip_ws(&upper, after_name + 2)
        } else {
            after_name
        };

        let rest = trimmed[type_start..].trim_end();
        let rest_upper = upper[type_start..].trim_end();

        // ` CONSTRAINT ` also terminates the base type: PostgreSQL allows a
        // named `CONSTRAINT <name> CHECK (...)` between the type and the
        // check body, and without this the constraint name and keyword were
        // folded into the reported base type. See #166.
        let end_pos = [" NOT NULL", " CHECK", " DEFAULT", " CONSTRAINT "]
            .iter()
            .filter_map(|kw| rest_upper.find(kw))
            .min()
            .unwrap_or(rest.len());
        let base_type_raw = rest[..end_pos].trim();
        if base_type_raw.is_empty() {
            return true;
        }

        // Only a `NOT NULL` appearing outside of any parenthesized group
        // (and outside of any single-quoted string literal) is the `NOT
        // NULL` constraint keyword -- a `CHECK` body's payload may itself
        // contain the literal text `NOT NULL`, e.g.
        // `CHECK (VALUE <> 'NOT NULL')`, which must not be mistaken for the
        // keyword. See #166.
        let not_null = top_level_contains_keyword(rest_upper, "NOT NULL");

        let parser_dialect = dialect.to_sqlparser_dialect();
        let normalized = match Parser::parse_sql(
            parser_dialect.as_ref(),
            &format!("CREATE TABLE _domain_tmp_ (_col_ {})", base_type_raw),
        ) {
            Ok(stmts) => {
                if let Some(Statement::CreateTable(ct)) = stmts.into_iter().next() {
                    if let Some(col) = ct.columns.first() {
                        let (t, _) = normalize_data_type(&col.data_type, &self.domains, *dialect);
                        t
                    } else {
                        base_type_raw.to_lowercase()
                    }
                } else {
                    base_type_raw.to_lowercase()
                }
            }
            Err(_) => base_type_raw.to_lowercase(),
        };

        self.domains.insert(
            domain_name,
            DomainDef {
                base_type: normalized,
                not_null,
            },
        );
        true
    }
}

impl Catalog {
    fn process_statement(&mut self, stmt: Statement, dialect: &SqlDialect) -> Result<(), ScytheError> {
        match stmt {
            Statement::CreateTable(ct) => self.process_create_table(ct, dialect),
            Statement::AlterTable(alter_table) => {
                self.process_alter_table(alter_table.name, alter_table.operations, dialect)
            }
            Statement::CreateType { name, representation } => {
                if let Some(repr) = representation {
                    self.process_create_type(name, repr, dialect)
                } else {
                    Ok(())
                }
            }
            Statement::AlterType(alter_type) => self.process_alter_type(alter_type.name, alter_type.operation),
            Statement::CreateView(cv) => {
                self.process_create_view(cv.name, cv.columns, *cv.query, cv.materialized, dialect)
            }
            _ => Ok(()),
        }
    }

    fn process_create_table(
        &mut self,
        ct: sqlparser::ast::CreateTable,
        dialect: &SqlDialect,
    ) -> Result<(), ScytheError> {
        let table_name = object_name_to_key(&ct.name);
        let raw_name = object_name_to_raw_name(&ct.name);

        let columns: Vec<Column> = if ct.columns.is_empty() {
            match ct.query {
                // `CREATE TABLE ... AS SELECT`: the schema comes entirely
                // from the query's projected columns. Resolve it through
                // the same analyzer path an ordinary annotated query uses,
                // rather than silently registering a zero-column table. See
                // issue #183.
                Some(query) => self.resolve_select_columns(*query)?,
                None => Vec::new(),
            }
        } else {
            let mut columns: Vec<Column> = Vec::new();

            for col_def in &ct.columns {
                let col_name = ident_to_lower(&col_def.name);
                let (sql_type, is_serial) = normalize_data_type(&col_def.data_type, &self.domains, *dialect);

                let sql_type = if let DataType::Enum(variants, _bits) = &col_def.data_type {
                    if matches!(dialect, SqlDialect::MySQL | SqlDialect::SQLite) && !variants.is_empty() {
                        let enum_key = format!("{}_{}", table_name.replace('.', "_"), col_name);
                        let values: Vec<String> = variants
                            .iter()
                            .map(|v| match v {
                                sqlparser::ast::EnumMember::Name(name) => name.trim_matches('\'').to_string(),
                                sqlparser::ast::EnumMember::NamedValue(name, _) => name.trim_matches('\'').to_string(),
                            })
                            .collect();
                        self.enums.insert(enum_key.clone(), EnumType { values });
                        enum_key
                    } else {
                        sql_type
                    }
                } else {
                    sql_type
                };

                let mut nullable = !is_serial;
                let mut default: Option<String> = None;
                let mut primary_key = false;
                let mut is_auto_increment = false;

                for opt_def in &col_def.options {
                    match &opt_def.option {
                        ColumnOption::Null => {
                            nullable = true;
                        }
                        ColumnOption::NotNull => {
                            nullable = false;
                        }
                        ColumnOption::Default(expr) => {
                            default = Some(expr.to_string());
                        }
                        ColumnOption::PrimaryKey(_) => {
                            primary_key = true;
                            let is_integer_rowid_alias = matches!(col_def.data_type, DataType::Integer(_));
                            if sqlite_primary_key_forces_not_null(
                                dialect,
                                ct.without_rowid,
                                ct.strict,
                                is_integer_rowid_alias,
                            ) {
                                nullable = false;
                            }
                        }
                        ColumnOption::Unique(_) => {}
                        ColumnOption::Generated {
                            generation_expr: Some(expr),
                            ..
                        } => {
                            default = Some(format!("GENERATED ALWAYS AS ({})", expr));
                        }
                        ColumnOption::DialectSpecific(tokens) => {
                            let joined: String = tokens
                                .iter()
                                .map(|t| t.to_string().to_uppercase())
                                .collect::<Vec<_>>()
                                .join("");
                            if joined.contains("AUTO_INCREMENT") || joined.contains("AUTOINCREMENT") {
                                is_auto_increment = true;
                                nullable = false;
                            }
                        }
                        _ => {}
                    }
                }

                if is_auto_increment {
                    nullable = false;
                }

                columns.push(Column {
                    name: col_name,
                    sql_type,
                    nullable,
                    default,
                    primary_key,
                });
            }

            for constraint in &ct.constraints {
                if let TableConstraint::PrimaryKey(pk_constraint) = constraint {
                    // The rowid-alias escape hatch only exists for a
                    // *single*-column primary key (SQLite has no composite
                    // rowid alias), so look up its raw declared type from
                    // `ct.columns` -- `columns` (built above) only carries
                    // `normalize_data_type`'s output, which collapses `INT`
                    // and `INTEGER` onto the same value and can no longer
                    // answer this question.
                    let is_integer_rowid_alias = match pk_constraint.columns.as_slice() {
                        [single] => {
                            let pk_name = pk_column_name(&single.column.expr);
                            ct.columns.iter().any(|c| {
                                ident_to_lower(&c.name) == pk_name && matches!(c.data_type, DataType::Integer(_))
                            })
                        }
                        _ => false,
                    };
                    let force_not_null = sqlite_primary_key_forces_not_null(
                        dialect,
                        ct.without_rowid,
                        ct.strict,
                        is_integer_rowid_alias,
                    );

                    for idx_col in &pk_constraint.columns {
                        let pk_name = pk_column_name(&idx_col.column.expr);
                        if let Some(col) = columns.iter_mut().find(|c| c.name == pk_name) {
                            col.primary_key = true;
                            if force_not_null {
                                col.nullable = false;
                            }
                        }
                    }
                }
            }

            columns
        };

        // `CREATE TABLE IF NOT EXISTS` on a table that is already
        // registered is a no-op in PostgreSQL -- it must not silently
        // replace the existing definition, which is exactly the idiom used
        // in idempotent migration files. See issue #183.
        if ct.if_not_exists && self.tables.contains_key(&table_name) {
            return Ok(());
        }

        self.tables.insert(table_name, Table { columns, raw_name });
        Ok(())
    }

    fn process_alter_table(
        &mut self,
        name: ObjectName,
        operations: Vec<AlterTableOperation>,
        dialect: &SqlDialect,
    ) -> Result<(), ScytheError> {
        let table_key = object_name_to_key(&name);

        for op in operations {
            match op {
                AlterTableOperation::AddColumn { column_def, .. } => {
                    let Some(table) = get_table_mut(&mut self.tables, &table_key) else {
                        return Err(ScytheError::unknown_table(&table_key));
                    };
                    let col_name = ident_to_lower(&column_def.name);
                    let (sql_type, is_serial) = normalize_data_type(&column_def.data_type, &self.domains, *dialect);
                    let mut nullable = !is_serial;
                    let mut default = None;
                    let mut primary_key = false;

                    for opt_def in &column_def.options {
                        match &opt_def.option {
                            ColumnOption::Null => nullable = true,
                            ColumnOption::NotNull => nullable = false,
                            ColumnOption::Default(expr) => {
                                default = Some(expr.to_string());
                            }
                            ColumnOption::PrimaryKey(_) => {
                                primary_key = true;
                                // SQLite supports neither adding a
                                // primary-key column nor `WITHOUT
                                // ROWID`/`STRICT` on an existing table, so
                                // this branch is unreachable against real
                                // SQLite; kept consistent with
                                // `process_create_table` for other
                                // dialects, using the same exact-`INTEGER`
                                // rowid-alias rule rather than a blanket
                                // `nullable = false`. See #108.
                                let is_integer_rowid_alias = matches!(column_def.data_type, DataType::Integer(_));
                                if sqlite_primary_key_forces_not_null(dialect, false, false, is_integer_rowid_alias) {
                                    nullable = false;
                                }
                            }
                            _ => {}
                        }
                    }

                    table.columns.push(Column {
                        name: col_name,
                        sql_type,
                        nullable,
                        default,
                        primary_key,
                    });
                }
                AlterTableOperation::DropColumn { column_names, .. } => {
                    let Some(table) = get_table_mut(&mut self.tables, &table_key) else {
                        return Err(ScytheError::unknown_table(&table_key));
                    };
                    for column_name in &column_names {
                        let col_lower = ident_to_lower(column_name);
                        table.columns.retain(|c| c.name != col_lower);
                    }
                }
                AlterTableOperation::RenameColumn {
                    old_column_name,
                    new_column_name,
                } => {
                    let Some(table) = get_table_mut(&mut self.tables, &table_key) else {
                        return Err(ScytheError::unknown_table(&table_key));
                    };
                    let old_name = ident_to_lower(&old_column_name);
                    let new_name = ident_to_lower(&new_column_name);
                    if let Some(col) = table.columns.iter_mut().find(|c| c.name == old_name) {
                        col.name = new_name;
                    }
                }
                AlterTableOperation::RenameTable { table_name } => {
                    let (new_key, new_raw_name) = match &table_name {
                        sqlparser::ast::RenameTableNameKind::To(name)
                        | sqlparser::ast::RenameTableNameKind::As(name) => {
                            (object_name_to_key(name), object_name_to_raw_name(name))
                        }
                    };
                    let removed = match self.tables.remove(&table_key) {
                        Some(table) => Some(table),
                        None => {
                            let bare = bare_name(&table_key).to_string();
                            self.tables.remove(&bare)
                        }
                    };
                    // Every other operation in this match (AddColumn,
                    // DropColumn, RenameColumn, AlterColumn, AddConstraint)
                    // rejects an unknown target table with
                    // `ScytheError::unknown_table` rather than silently doing
                    // nothing. `RENAME TO` was the one exception: a typo'd
                    // table name in a migration no-opped without any signal,
                    // making it indistinguishable from a correct rename.
                    // Follow the same precedent here.
                    let Some(mut table) = removed else {
                        return Err(ScytheError::unknown_table(&table_key));
                    };
                    // `raw_name` must track the rename too, not just the
                    // lookup key -- otherwise a table renamed to CamelCase
                    // would keep reporting its pre-rename (possibly
                    // snake_case) spelling to SC-N02 forever.
                    table.raw_name = new_raw_name;
                    self.tables.insert(new_key, table);
                }
                AlterTableOperation::AlterColumn { column_name, op } => {
                    let Some(table) = get_table_mut(&mut self.tables, &table_key) else {
                        return Err(ScytheError::unknown_table(&table_key));
                    };
                    let col_lower = ident_to_lower(&column_name);
                    if let Some(col) = table.columns.iter_mut().find(|c| c.name == col_lower) {
                        match op {
                            AlterColumnOperation::SetNotNull => {
                                col.nullable = false;
                            }
                            AlterColumnOperation::DropNotNull => {
                                col.nullable = true;
                            }
                            AlterColumnOperation::SetDataType { data_type, .. } => {
                                let (new_type, _) = normalize_data_type(&data_type, &self.domains, *dialect);
                                col.sql_type = new_type;
                            }
                            AlterColumnOperation::SetDefault { value } => {
                                col.default = Some(value.to_string());
                            }
                            AlterColumnOperation::DropDefault => {
                                col.default = None;
                            }
                            _ => {}
                        }
                    }
                }
                AlterTableOperation::AddConstraint { constraint, .. } => {
                    let Some(table) = get_table_mut(&mut self.tables, &table_key) else {
                        return Err(ScytheError::unknown_table(&table_key));
                    };
                    if let TableConstraint::PrimaryKey(pk_constraint) = &constraint {
                        // SQLite supports neither `ADD CONSTRAINT` nor
                        // adding a primary key to an existing table, so
                        // this branch is unreachable against real SQLite.
                        // By the time an `ALTER TABLE` reaches here,
                        // `table.columns` only carries
                        // `normalize_data_type`'s output, which collapses
                        // `INT` and `INTEGER` onto the same string -- the
                        // rowid-alias exact-type check from
                        // `process_create_table` cannot be reproduced
                        // here. Kept sound rather than precise: never
                        // force `NOT NULL` on SQLite through this path,
                        // exactly like a composite SQLite primary key. See
                        // #108.
                        let force_not_null = sqlite_primary_key_forces_not_null(dialect, false, false, false);
                        for idx_col in &pk_constraint.columns {
                            let pk_name = pk_column_name(&idx_col.column.expr);
                            if let Some(col) = table.columns.iter_mut().find(|c| c.name == pk_name) {
                                col.primary_key = true;
                                if force_not_null {
                                    col.nullable = false;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn process_create_type(
        &mut self,
        name: ObjectName,
        repr: UserDefinedTypeRepresentation,
        dialect: &SqlDialect,
    ) -> Result<(), ScytheError> {
        let type_key = object_name_to_key(&name);

        match repr {
            UserDefinedTypeRepresentation::Enum { labels } => {
                let values: Vec<String> = labels.iter().map(|l| l.value.clone()).collect();
                self.enums.insert(type_key, EnumType { values });
            }
            UserDefinedTypeRepresentation::Composite { attributes } => {
                let fields: Vec<CompositeField> = attributes
                    .iter()
                    .map(|attr| {
                        let (ft, _) = normalize_data_type(&attr.data_type, &self.domains, *dialect);
                        CompositeField {
                            name: ident_to_lower(&attr.name),
                            sql_type: ft,
                        }
                    })
                    .collect();
                self.composites.insert(type_key, CompositeType { fields });
            }
            _ => {}
        }

        Ok(())
    }

    fn process_alter_type(&mut self, name: ObjectName, operation: AlterTypeOperation) -> Result<(), ScytheError> {
        let type_key = object_name_to_key(&name);

        match operation {
            AlterTypeOperation::AddValue(add_val) => {
                if let Some(enum_type) = self.enums.get_mut(&type_key) {
                    enum_type.values.push(add_val.value.value.clone());
                }
            }
            AlterTypeOperation::RenameValue(rename_val) => {
                if let Some(enum_type) = self.enums.get_mut(&type_key) {
                    let from = &rename_val.from.value;
                    if let Some(v) = enum_type.values.iter_mut().find(|v| v == &from) {
                        *v = rename_val.to.value.clone();
                    }
                }
            }
            AlterTypeOperation::Rename(rename) => {
                let new_key = rename.new_name.value.to_lowercase();
                if let Some(e) = self.enums.remove(&type_key) {
                    self.enums.insert(new_key.clone(), e);
                }
                if let Some(c) = self.composites.remove(&type_key) {
                    self.composites.insert(new_key, c);
                }
            }
        }

        Ok(())
    }
}

fn get_table_mut<'a>(tables: &'a mut AHashMap<String, Table>, key: &str) -> Option<&'a mut Table> {
    if tables.contains_key(key) {
        return tables.get_mut(key);
    }
    if key.contains('.') {
        // A schema-qualified key must match a real, equally-qualified
        // registration -- do not silently strip the qualifier and resolve
        // a bare-registered table. Mirrors `lookup_qualified`; see #185.
        return None;
    }
    let suffix = format!(".{key}");
    let found_key = {
        let mut candidates: Vec<&String> = tables.keys().filter(|k| k.ends_with(&suffix)).collect();
        // Deterministic regardless of `AHashMap`'s per-process-random
        // iteration order -- see #177.
        candidates.sort();
        candidates.first().map(|k| (*k).clone())
    };
    found_key.and_then(move |k| tables.get_mut(&k))
}

/// Look up `name` in `map`, tolerating a *single* schema qualifier on
/// either side of the lookup/registration boundary:
///
/// - An unqualified lookup (`"users"`) also matches an entry registered
///   under any single leading qualifier (`"public.users"`), deterministically
///   picking the lexicographically smallest matching key when more than one
///   qualifier registers the same bare name. `AHashMap`'s iteration order is
///   randomized per process, so relying on "whichever the iterator finds
///   first" made the same input resolve to a different entry from one run
///   to the next -- see #177.
/// - A qualified lookup (`"wrong_schema.users"`) matches *only* an exact,
///   equally-qualified entry. It must never fall back to a bare-registered
///   entry under a different qualifier than the one asked for: doing so
///   silently accepted any schema qualifier for a bare-registered table --
///   see #185.
///
/// Shared by [`Catalog::get_table`], [`Catalog::get_enum`],
/// [`Catalog::get_composite`], [`Catalog::get_domain_base_type`], and (via
/// `super::lookup_qualified`) `type_normalizer::normalize_data_type`'s own
/// domain lookup -- keeping every domain-name resolution on one path is
/// exactly what #184 (item 3) required.
fn lookup_qualified<'a, T>(map: &'a AHashMap<String, T>, name: &str) -> Option<&'a T> {
    let lower = name.to_lowercase();
    if let Some(value) = map.get(&lower) {
        return Some(value);
    }
    if lower.contains('.') {
        return None;
    }
    let suffix = format!(".{lower}");
    map.iter()
        .filter(|(k, _)| k.ends_with(&suffix))
        .min_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, v)| v)
}

/// Whether a `PRIMARY KEY` on this column/constraint implies `NOT NULL`.
///
/// On every dialect other than SQLite, `PRIMARY KEY` always implies `NOT
/// NULL`. SQLite is the outlier: a bare `PRIMARY KEY` does *not* imply `NOT
/// NULL` at all -- SQLite 3.50.6 happily stores a `NULL` in `k INT PRIMARY
/// KEY` or `k TEXT PRIMARY KEY`. The only cases where SQLite does enforce
/// `NOT NULL` are:
///
/// - a single-column primary key whose *raw declared type name* is exactly
///   `INTEGER` (case-insensitive) -- the "rowid alias" -- passed here as
///   `is_integer_rowid_alias`; `INT`, `INTEGER(11)`, `INT4` and `BIGINT` do
///   not qualify, only the exact spelling does,
/// - a table declared `WITHOUT ROWID`,
/// - a table declared `STRICT`.
///
/// `is_integer_rowid_alias` must be computed from the *raw* `DataType`
/// before it passes through [`normalize_data_type`], which collapses `INT`
/// and `INTEGER` onto the same normalized string and would make the two
/// indistinguishable here. See issue #108.
fn sqlite_primary_key_forces_not_null(
    dialect: &SqlDialect,
    without_rowid: bool,
    strict: bool,
    is_integer_rowid_alias: bool,
) -> bool {
    if !matches!(dialect, SqlDialect::SQLite) {
        return true;
    }
    without_rowid || strict || is_integer_rowid_alias
}

/// Extract the column name referenced by a `PRIMARY KEY (...)` constraint
/// entry, normalized the same way column names are registered
/// ([`ident_to_lower`]): an unquoted identifier is lowercased, a quoted
/// identifier keeps its exact case and loses its quote characters.
/// `expr.to_string()` alone retains the quote characters themselves, which
/// then never matches a registered column name. See issue #178.
fn pk_column_name(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(ident) => ident_to_lower(ident),
        other => other.to_string().to_lowercase(),
    }
}

/// Byte offset of the first non-whitespace character in `s` at or after
/// `from`, or `s.len()` if none. Used by [`Catalog::try_parse_create_domain`].
fn skip_ws(s: &str, from: usize) -> usize {
    s[from..]
        .find(|c: char| !c.is_whitespace())
        .map_or(s.len(), |p| from + p)
}

/// Byte offset of the first whitespace character in `s` at or after
/// `from`, or `s.len()` if none. Used by [`Catalog::try_parse_create_domain`].
fn find_ws(s: &str, from: usize) -> usize {
    s[from..].find(char::is_whitespace).map_or(s.len(), |p| from + p)
}

/// Whether `keyword` (already ASCII-uppercase) appears in `haystack`
/// (already ASCII-uppercase) outside of any parenthesized group and outside
/// of any single-quoted string literal -- so a `CHECK (...)` payload
/// containing the literal text `NOT NULL` (e.g. `CHECK (VALUE <> 'NOT
/// NULL')`) is never mistaken for the `NOT NULL` constraint keyword. See
/// issue #166.
fn top_level_contains_keyword(haystack: &str, keyword: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut depth: i32 = 0;
    let mut in_quote = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_quote {
            if b == b'\'' {
                in_quote = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' => {
                in_quote = true;
                i += 1;
                continue;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth <= 0 && haystack[i..].starts_with(keyword) {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_create_table() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE users (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                email VARCHAR(255),
                age INTEGER DEFAULT 0,
                active BOOLEAN NOT NULL DEFAULT true
            );"])
        .unwrap();

        let table = catalog.get_table("users").unwrap();
        assert_eq!(table.columns.len(), 5);

        let id = &table.columns[0];
        assert_eq!(id.name, "id");
        assert_eq!(id.sql_type, "integer");
        assert!(!id.nullable);
        assert!(id.primary_key);

        let name_col = &table.columns[1];
        assert_eq!(name_col.name, "name");
        assert_eq!(name_col.sql_type, "text");
        assert!(!name_col.nullable);

        let email = &table.columns[2];
        assert_eq!(email.name, "email");
        assert_eq!(email.sql_type, "varchar(255)");
        assert!(email.nullable);

        let age = &table.columns[3];
        assert_eq!(age.sql_type, "integer");
        assert!(age.default.is_some());

        let active = &table.columns[4];
        assert_eq!(active.sql_type, "boolean");
        assert!(!active.nullable);
    }

    #[test]
    fn test_enum_type() {
        let catalog = Catalog::from_ddl(&["CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');"]).unwrap();

        let mood = catalog.get_enum("mood").unwrap();
        assert_eq!(mood.values, vec!["sad", "ok", "happy"]);
    }

    #[test]
    fn test_composite_type() {
        let catalog = Catalog::from_ddl(&["CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);"]).unwrap();

        let addr = catalog.get_composite("address").unwrap();
        assert_eq!(addr.fields.len(), 3);
        assert_eq!(addr.fields[0].name, "street");
        assert_eq!(addr.fields[0].sql_type, "text");
    }

    #[test]
    fn test_alter_table_add_column() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE t (id INTEGER);",
            "ALTER TABLE t ADD COLUMN name TEXT NOT NULL;",
        ])
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[1].name, "name");
        assert!(!table.columns[1].nullable);
    }

    #[test]
    fn test_alter_type_add_value() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TYPE mood AS ENUM ('sad', 'happy');",
            "ALTER TYPE mood ADD VALUE 'ok';",
        ])
        .unwrap();

        let mood = catalog.get_enum("mood").unwrap();
        assert_eq!(mood.values, vec!["sad", "happy", "ok"]);
    }

    #[test]
    fn test_serial_types() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (
                a SERIAL,
                b BIGSERIAL,
                c SMALLSERIAL
            );"])
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].sql_type, "integer");
        assert!(!table.columns[0].nullable);
        assert_eq!(table.columns[1].sql_type, "bigint");
        assert!(!table.columns[1].nullable);
        assert_eq!(table.columns[2].sql_type, "smallint");
        assert!(!table.columns[2].nullable);
    }

    #[test]
    fn test_table_level_primary_key() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (
                a INTEGER,
                b TEXT,
                PRIMARY KEY (a)
            );"])
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert!(table.columns[0].primary_key);
        assert!(!table.columns[0].nullable);
        assert!(!table.columns[1].primary_key);
    }

    #[test]
    fn test_schema_qualified_name() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE public.users (id INTEGER);"]).unwrap();

        assert!(catalog.get_table("public.users").is_some());
        assert!(catalog.get_table("users").is_some());
    }

    #[test]
    fn test_array_type() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (tags TEXT[], scores INTEGER[]);"]).unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].sql_type, "text[]");
        assert_eq!(table.columns[1].sql_type, "int[]");
    }

    #[test]
    fn test_timestamp_types() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (
                a TIMESTAMP,
                b TIMESTAMP WITH TIME ZONE,
                c TIMESTAMPTZ
            );"])
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].sql_type, "timestamp");
        assert_eq!(table.columns[1].sql_type, "timestamptz");
        assert_eq!(table.columns[2].sql_type, "timestamptz");
    }

    #[test]
    fn test_mysql_basic_create_table() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE users (
                id INT PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                email TEXT,
                active BOOLEAN NOT NULL DEFAULT true
            );"],
            &crate::dialect::SqlDialect::MySQL,
        )
        .unwrap();

        let table = catalog.get_table("users").unwrap();
        assert_eq!(table.columns.len(), 4);

        let id = &table.columns[0];
        assert_eq!(id.name, "id");
        assert!(id.primary_key);
        assert!(!id.nullable);

        let name_col = &table.columns[1];
        assert_eq!(name_col.name, "name");
        assert!(!name_col.nullable);

        let email = &table.columns[2];
        assert_eq!(email.name, "email");
        assert!(email.nullable);
    }

    #[test]
    fn test_mysql_auto_increment() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (
                id INT AUTO_INCREMENT PRIMARY KEY,
                name VARCHAR(100)
            );"],
            &crate::dialect::SqlDialect::MySQL,
        )
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].name, "id");
        assert!(!table.columns[0].nullable);
        assert!(table.columns[0].primary_key);
    }

    #[test]
    fn test_mysql_inline_enum() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (
                status ENUM('active', 'inactive', 'pending') NOT NULL
            );"],
            &crate::dialect::SqlDialect::MySQL,
        )
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].name, "status");
        assert!(!table.columns[0].nullable);
        let enum_type = catalog.get_enum("t_status").unwrap();
        assert_eq!(enum_type.values, vec!["active", "inactive", "pending"]);
    }

    #[test]
    fn test_mysql_types() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (
                a TINYINT,
                b MEDIUMINT,
                c BIGINT,
                d DOUBLE,
                e DATETIME,
                f BLOB,
                g JSON
            );"],
            &crate::dialect::SqlDialect::MySQL,
        )
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns.len(), 7);
    }

    #[test]
    fn test_sqlite_basic_create_table() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT,
                score REAL
            );"],
            &crate::dialect::SqlDialect::SQLite,
        )
        .unwrap();

        let table = catalog.get_table("users").unwrap();
        assert_eq!(table.columns.len(), 4);

        let id = &table.columns[0];
        assert_eq!(id.name, "id");
        assert!(id.primary_key);
        assert!(!id.nullable);

        let score = &table.columns[3];
        assert_eq!(score.name, "score");
        assert!(score.nullable);
    }

    #[test]
    fn test_sqlite_types() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (
                a INTEGER,
                b REAL,
                c TEXT,
                d BLOB,
                e NUMERIC,
                f BOOLEAN
            );"],
            &crate::dialect::SqlDialect::SQLite,
        )
        .unwrap();

        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns.len(), 6);
    }

    #[test]
    fn test_from_ddl_backward_compat() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER);"]).unwrap();
        assert!(catalog.get_table("t").is_some());
    }

    #[test]
    fn test_redshift_identity_stripping() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE users (
                id INTEGER IDENTITY(1,1) PRIMARY KEY,
                name VARCHAR(100) NOT NULL
            );"],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .unwrap();

        let table = catalog.get_table("users").unwrap();
        assert_eq!(table.columns.len(), 2);

        let id = &table.columns[0];
        assert_eq!(id.name, "id");
        assert!(id.primary_key);
        assert!(!id.nullable);

        let name = &table.columns[1];
        assert_eq!(name.name, "name");
        assert!(!name.nullable);
    }

    #[test]
    fn test_mssql_identity_stripping() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE products (
                id INT IDENTITY(100, 5) PRIMARY KEY,
                product_name VARCHAR(255)
            );"],
            &crate::dialect::SqlDialect::MsSql,
        )
        .unwrap();

        let table = catalog.get_table("products").unwrap();
        assert_eq!(table.columns.len(), 2);

        let id = &table.columns[0];
        assert_eq!(id.name, "id");
        assert!(id.primary_key);
    }

    #[test]
    fn test_identity_with_whitespace() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE test (
                id INTEGER IDENTITY  (  1  ,  1  ) NOT NULL
            );"],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .unwrap();

        let table = catalog.get_table("test").unwrap();
        assert_eq!(table.columns.len(), 1);
        assert_eq!(table.columns[0].name, "id");
    }

    #[test]
    fn test_postgresql_generated_identity_is_preserved() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE test (
                id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                name TEXT NOT NULL
            );"],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .unwrap();

        let table = catalog.get_table("test").unwrap();
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].name, "id");
        assert_eq!(table.columns[0].sql_type, "bigint");
        assert!(table.columns[0].primary_key);
    }

    #[test]
    fn test_identity_column_name_is_preserved() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE test (
                id BIGINT PRIMARY KEY,
                identity TEXT NOT NULL,
                other INTEGER
            );"],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .unwrap();

        let table = catalog.get_table("test").unwrap();
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.columns[1].name, "identity");
        assert_eq!(table.columns[1].sql_type, "text");
        assert!(!table.columns[1].nullable);
    }

    #[test]
    fn test_redshift_full_schema() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE users (
                    id INTEGER IDENTITY(1,1) NOT NULL,
                    name VARCHAR(255) NOT NULL,
                    email VARCHAR(255),
                    status VARCHAR(50) NOT NULL DEFAULT 'active',
                    created_at TIMESTAMPTZ NOT NULL DEFAULT GETDATE()
                );

                CREATE TABLE orders (
                    id INTEGER IDENTITY(1,1) NOT NULL,
                    user_id INTEGER NOT NULL,
                    total DECIMAL(10, 2) NOT NULL,
                    notes VARCHAR(4000),
                    created_at TIMESTAMPTZ NOT NULL DEFAULT GETDATE()
                );

                CREATE TABLE tags (
                    id INTEGER IDENTITY(1,1) NOT NULL,
                    name VARCHAR(255) NOT NULL
                );

                CREATE TABLE user_tags (
                    user_id INTEGER NOT NULL,
                    tag_id INTEGER NOT NULL
                );"],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .unwrap();

        assert!(catalog.get_table("users").is_some());
        assert!(catalog.get_table("orders").is_some());
        assert!(catalog.get_table("tags").is_some());
        assert!(catalog.get_table("user_tags").is_some());

        let users = catalog.get_table("users").unwrap();
        assert_eq!(users.columns.len(), 5);
        assert_eq!(users.columns[0].name, "id");
        assert!(!users.columns[0].nullable);
        assert_eq!(users.columns[1].name, "name");
        assert!(!users.columns[1].nullable);
        assert_eq!(users.columns[2].name, "email");
        assert!(users.columns[2].nullable);

        let orders = catalog.get_table("orders").unwrap();
        assert_eq!(orders.columns.len(), 5);
        assert_eq!(orders.columns[0].name, "id");
        assert!(!orders.columns[0].nullable);
    }

    #[test]
    fn test_identity_case_insensitive() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE test (
                id INT Identity(1,1) NOT NULL
            );"],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .unwrap();

        let table = catalog.get_table("test").unwrap();
        assert_eq!(table.columns.len(), 1);
        assert_eq!(table.columns[0].name, "id");
    }

    #[test]
    fn test_skips_psql_restrict_meta_command() {
        let schema = "\
-- PostgreSQL database dump\n\
-- Dumped from database version 18.0\n\
\n\
\\restrict pq7iUOIh6kaSGp222hdriGzvRgqMRbZgU76Lw2XJsigT6TAJ0gcLqz6yTyHGDMO\n\
\n\
SET statement_timeout = 0;\n\
SET lock_timeout = 0;\n\
SET standard_conforming_strings = on;\n\
\n\
CREATE TABLE public.t (\n\
    id uuid NOT NULL,\n\
    meta jsonb\n\
);\n\
\n\
ALTER TABLE ONLY public.t\n\
    ADD CONSTRAINT t_pkey PRIMARY KEY (id);\n\
\n\
\\unrestrict pq7iUOIh6kaSGp222hdriGzvRgqMRbZgU76Lw2XJsigT6TAJ0gcLqz6yTyHGDMO\n\
";
        let catalog = Catalog::from_ddl(&[schema]).expect("parse must succeed");

        let table = catalog.get_table("t").expect("table t must exist");
        assert_eq!(table.columns.len(), 2);

        let id_col = &table.columns[0];
        assert_eq!(id_col.name, "id");
        assert_eq!(id_col.sql_type, "uuid");
        assert!(!id_col.nullable);
        assert!(id_col.primary_key);

        let meta_col = &table.columns[1];
        assert_eq!(meta_col.name, "meta");
        assert_eq!(meta_col.sql_type, "jsonb");
        assert!(meta_col.nullable);
    }

    #[test]
    fn test_skips_leading_backslash_line() {
        let schema = "\\restrict dbmate\nCREATE TABLE items (id SERIAL PRIMARY KEY, name TEXT NOT NULL);";
        let catalog = Catalog::from_ddl(&[schema]).expect("parse must succeed");

        let table = catalog.get_table("items").expect("table items must exist");
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].name, "id");
        assert_eq!(table.columns[0].sql_type, "integer");
        assert!(!table.columns[0].nullable);
        assert!(table.columns[0].primary_key);
        assert_eq!(table.columns[1].name, "name");
        assert!(!table.columns[1].nullable);
    }

    #[test]
    fn test_normal_ddl_without_backslash_unaffected() {
        let schema = "CREATE TABLE products (id INTEGER PRIMARY KEY, price NUMERIC(10,2) NOT NULL);";
        let catalog = Catalog::from_ddl(&[schema]).expect("parse must succeed");

        let table = catalog.get_table("products").expect("table products must exist");
        assert_eq!(table.columns.len(), 2);

        let id_col = &table.columns[0];
        assert_eq!(id_col.name, "id");
        assert_eq!(id_col.sql_type, "integer");
        assert!(!id_col.nullable);
        assert!(id_col.primary_key);

        let price_col = &table.columns[1];
        assert_eq!(price_col.name, "price");
        assert_eq!(price_col.sql_type, "numeric(10,2)");
        assert!(!price_col.nullable);
    }

    #[test]
    fn test_oracle_slash_script_skips_sequences_and_triggers() {
        // Mirrors integration_tests/sql/oracle/schema_full.sql: SQL*Plus `/`
        // terminators, a CREATE SEQUENCE whose START WITH precedes INCREMENT
        // BY (an ordering sqlparser's positional option parser rejects), and
        // a CREATE OR REPLACE TRIGGER with a PL/SQL body sqlparser cannot
        // parse at all. None of the three should reach the parser as-is; the
        // sequence and trigger must be skipped, and the table must still be
        // extracted correctly.
        let schema = "\
-- Full Oracle schema including sequences and triggers.\n\
\n\
CREATE SEQUENCE users_seq START WITH 1 INCREMENT BY 1\n\
/\n\
\n\
CREATE TABLE users (\n\
    id NUMBER NOT NULL PRIMARY KEY,\n\
    name VARCHAR2(255) NOT NULL,\n\
    email VARCHAR2(255)\n\
)\n\
/\n\
\n\
CREATE OR REPLACE TRIGGER users_bi\n\
BEFORE INSERT ON users\n\
FOR EACH ROW\n\
BEGIN\n\
    IF :NEW.id IS NULL THEN\n\
        :NEW.id := users_seq.NEXTVAL;\n\
    END IF;\n\
END;\n\
/\n\
";
        let catalog = Catalog::from_ddl_with_dialect(&[schema], &crate::dialect::SqlDialect::Oracle)
            .expect("schema with sequences and triggers must parse");

        assert_eq!(catalog.tables_iter().count(), 1, "only the table should be cataloged");

        let table = catalog.get_table("users").expect("table users must exist");
        assert_eq!(table.columns.len(), 3);

        let id = &table.columns[0];
        assert_eq!(id.name, "id");
        assert!(id.primary_key);
        assert!(!id.nullable);

        let name = &table.columns[1];
        assert_eq!(name.name, "name");
        assert!(!name.nullable);

        let email = &table.columns[2];
        assert_eq!(email.name, "email");
        assert!(email.nullable);
    }

    #[test]
    fn test_split_oracle_slash_statements_basic() {
        let sql = "CREATE SEQUENCE s START WITH 1\n/\n\nCREATE TABLE t (id NUMBER)\n/\n";
        let blocks = Catalog::split_oracle_slash_statements(sql);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].trim(), "CREATE SEQUENCE s START WITH 1");
        assert_eq!(blocks[1].trim(), "CREATE TABLE t (id NUMBER)");
    }

    #[test]
    fn test_split_oracle_slash_statements_no_trailing_slash() {
        let sql = "CREATE TABLE t (id NUMBER)\n/\nCREATE TABLE u (id NUMBER)";
        let blocks = Catalog::split_oracle_slash_statements(sql);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].trim(), "CREATE TABLE t (id NUMBER)");
        assert_eq!(blocks[1].trim(), "CREATE TABLE u (id NUMBER)");
    }

    // -- #178: quoted PRIMARY KEY column names ------------------------------

    #[test]
    fn test_quoted_primary_key_name_applies_not_null() {
        // A quoted PRIMARY KEY column name retained its quotes when
        // lowercased for comparison, so it never matched the registered
        // column and the PK constraint (and the NOT NULL it implies) was
        // silently dropped.
        let catalog = Catalog::from_ddl(&[r#"CREATE TABLE t ("Id" INTEGER, note TEXT, PRIMARY KEY ("Id"));"#]).unwrap();
        let table = catalog.get_table("t").unwrap();
        let id_col = table
            .columns
            .iter()
            .find(|c| c.name == "Id")
            .expect("quoted PK column must be registered as \"Id\"");
        assert!(
            id_col.primary_key,
            "quoted PRIMARY KEY (\"Id\") must mark the column primary_key"
        );
        assert!(!id_col.nullable, "a quoted PRIMARY KEY column must still be NOT NULL");
    }

    #[test]
    fn test_quoted_primary_key_via_alter_table_add_constraint() {
        // The pg_dump `ALTER TABLE ... ADD CONSTRAINT ... PRIMARY KEY` form
        // of the same bug.
        let catalog = Catalog::from_ddl(&[
            r#"CREATE TABLE t ("UserId" INTEGER, note TEXT);"#,
            r#"ALTER TABLE t ADD CONSTRAINT t_pkey PRIMARY KEY ("UserId");"#,
        ])
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let user_id = table.columns.iter().find(|c| c.name == "UserId").unwrap();
        assert!(user_id.primary_key);
        assert!(!user_id.nullable);
    }

    // -- #181: strip_identity_patterns byte-index-as-char-index -------------

    #[test]
    fn test_strip_identity_patterns_preserves_non_ascii_text() {
        let catalog = Catalog::from_ddl_with_dialect(
            &[r#"CREATE TABLE t (id INTEGER NOT NULL, "naïve" TEXT NOT NULL, "café" TEXT NOT NULL);"#],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let names: Vec<&String> = table.columns.iter().map(|c| &c.name).collect();
        assert!(
            table.columns.iter().any(|c| c.name == "naïve"),
            "non-ASCII quoted identifier must survive identity-pattern stripping intact, got: {names:?}"
        );
        assert!(table.columns.iter().any(|c| c.name == "café"));
    }

    #[test]
    fn test_strip_identity_patterns_skips_string_literal() {
        // A `default` value textually containing `identity(` inside a
        // string literal must not be treated as the IDENTITY(seed,step)
        // keyword at all.
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (id INTEGER NOT NULL, kind TEXT NOT NULL DEFAULT 'identity(x)', extra INTEGER NOT NULL);"],
            &crate::dialect::SqlDialect::PostgreSQL,
        )
        .expect("legal DDL with an `identity(x)` string literal must parse");
        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns.len(), 3, "no columns must be dropped/corrupted");
        assert_eq!(table.columns[2].name, "extra");
    }

    #[test]
    fn test_strip_identity_patterns_preserves_text_after_non_numeric_identity_paren() {
        let result = Catalog::strip_identity_patterns("IDENTITY(abc) more_text_here");
        assert!(
            result.contains("more_text_here"),
            "text following a non-numeric IDENTITY(...) payload must not be deleted, got: {result:?}"
        );
    }

    #[test]
    fn test_strip_identity_patterns_preserves_multibyte_chars_byte_for_byte() {
        let input = "café naïve 日本語 ﬁ IDENTITY(1,1)";
        let result = Catalog::strip_identity_patterns(input);
        assert!(
            result.starts_with("café naïve 日本語 ﬁ "),
            "non-ASCII text before the IDENTITY(...) match must be byte-for-byte unchanged, got: {result:?}"
        );
        assert!(
            !result.contains("IDENTITY(1,1)"),
            "a valid numeric,numeric IDENTITY(...) pattern must still be stripped"
        );
    }

    // -- #183: CTAS, IF NOT EXISTS, ALTER on an unknown table ----------------

    #[test]
    fn test_create_table_as_select_registers_columns() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE base (id INTEGER NOT NULL, name TEXT NOT NULL);",
            "CREATE TABLE derived AS SELECT id, name FROM base;",
        ])
        .unwrap();
        let derived = catalog.get_table("derived").expect("CTAS must register a table");
        assert_eq!(
            derived.columns.len(),
            2,
            "CTAS must carry the query's projected columns, not zero"
        );
        assert_eq!(derived.columns[0].name, "id");
        assert_eq!(derived.columns[0].sql_type, "integer");
        assert_eq!(derived.columns[1].name, "name");
        assert_eq!(derived.columns[1].sql_type, "text");
    }

    #[test]
    fn test_create_table_if_not_exists_does_not_replace_existing_definition() {
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE u (id INTEGER NOT NULL, a TEXT NOT NULL);",
            "CREATE TABLE IF NOT EXISTS u (id INTEGER NOT NULL);",
        ])
        .unwrap();
        let table = catalog.get_table("u").unwrap();
        assert_eq!(
            table.columns.len(),
            2,
            "IF NOT EXISTS on an already-registered table must be a no-op"
        );
        assert!(
            table.columns.iter().any(|c| c.name == "a"),
            "column `a` from the original definition must survive"
        );
    }

    #[test]
    fn test_alter_table_add_column_on_unknown_table_errors() {
        let result = Catalog::from_ddl(&[
            "CREATE TABLE users (id INTEGER NOT NULL);",
            "ALTER TABLE userz ADD COLUMN email TEXT NOT NULL;",
        ]);
        assert!(
            result.is_err(),
            "ALTER TABLE against an unknown table must error, not silently no-op"
        );
        assert_eq!(result.unwrap_err().code, crate::errors::ErrorCode::UnknownTable);
    }

    #[test]
    fn test_alter_table_drop_column_on_unknown_table_errors() {
        let result = Catalog::from_ddl(&[
            "CREATE TABLE users (id INTEGER NOT NULL);",
            "ALTER TABLE userz DROP COLUMN id;",
        ]);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, crate::errors::ErrorCode::UnknownTable);
    }

    #[test]
    fn test_alter_table_rename_table_on_unknown_table_errors() {
        // A typo'd table name in a migration's `RENAME TO` must not
        // disappear silently -- every other ALTER TABLE operation here
        // already rejects an unknown target (see the two tests above); a
        // rename that quietly no-ops is indistinguishable from a correct
        // one and leaves the catalog with neither the old nor the new name
        // registered under a real definition.
        let result = Catalog::from_ddl(&[
            "CREATE TABLE users (id INTEGER NOT NULL);",
            "ALTER TABLE userz RENAME TO people;",
        ]);
        assert!(
            result.is_err(),
            "RENAME TO against an unknown table must error, not silently no-op"
        );
        assert_eq!(result.unwrap_err().code, crate::errors::ErrorCode::UnknownTable);
    }

    #[test]
    fn test_alter_table_rename_table_on_known_table_still_renames() {
        // Regression guard alongside the error-path test above: fixing the
        // unknown-table case must not break the ordinary successful rename,
        // including keeping `raw_name` in sync with the new name.
        let catalog = Catalog::from_ddl(&[
            "CREATE TABLE users (id INTEGER NOT NULL);",
            "ALTER TABLE users RENAME TO people;",
        ])
        .unwrap();
        assert!(
            catalog.get_table("users").is_none(),
            "the old name must no longer resolve"
        );
        let renamed = catalog.get_table("people").expect("the new name must resolve");
        assert_eq!(renamed.raw_name, "people");
        assert_eq!(renamed.columns.len(), 1);
    }

    // -- #184: CREATE DOMAIN panic, schema-qualified NOT NULL ----------------

    #[test]
    fn test_create_domain_with_non_ascii_name_does_not_panic() {
        // A domain name whose uppercase form has a different byte length
        // than its original (U+FB01 "ﬁ" uppercases to the two-byte-longer
        // "FI") panicked when that offset was sliced against the original.
        let catalog = Catalog::from_ddl(&[
            "CREATE DOMAIN \u{fb01}\u{fb01} AS TEXT NOT NULL;",
            "CREATE TABLE t (id INTEGER NOT NULL);",
        ])
        .unwrap();
        assert!(catalog.get_table("t").is_some());
    }

    #[test]
    fn test_schema_qualified_domain_not_null_matches_bare_reference() {
        let catalog = Catalog::from_ddl(&[
            "CREATE DOMAIN app.nn AS TEXT NOT NULL;",
            "CREATE DOMAIN plain_nn AS TEXT NOT NULL;",
            "CREATE TABLE t (a nn, b plain_nn, c app.nn);",
        ])
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let a = table.columns.iter().find(|c| c.name == "a").unwrap();
        let b = table.columns.iter().find(|c| c.name == "b").unwrap();
        let c = table.columns.iter().find(|c| c.name == "c").unwrap();
        assert_eq!(a.sql_type, "text");
        assert!(
            !a.nullable,
            "a bare reference to a schema-qualified domain must resolve its NOT NULL the same as its type"
        );
        assert!(!b.nullable);
        assert!(!c.nullable);
    }

    // -- #166: CHECK-body substring, AS-optional, CONSTRAINT termination ----

    #[test]
    fn test_domain_check_body_containing_not_null_text_is_still_nullable() {
        let catalog = Catalog::from_ddl(&[
            "CREATE DOMAIN nickname AS TEXT CHECK (VALUE <> 'NOT NULL');",
            "CREATE DOMAIN plainname AS TEXT CHECK (VALUE <> 'x');",
            "CREATE TABLE users (id INTEGER NOT NULL, nick nickname, plain plainname);",
        ])
        .unwrap();
        let table = catalog.get_table("users").unwrap();
        let nick = table.columns.iter().find(|c| c.name == "nick").unwrap();
        let plain = table.columns.iter().find(|c| c.name == "plain").unwrap();
        assert!(
            nick.nullable,
            "a CHECK body merely containing the literal text NOT NULL must not be mistaken for the NOT NULL keyword"
        );
        assert!(plain.nullable);
    }

    #[test]
    fn test_create_domain_without_as_keyword() {
        let catalog = Catalog::from_ddl(&[
            "CREATE DOMAIN email_no_as TEXT NOT NULL;",
            "CREATE TABLE t (b email_no_as);",
        ])
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].sql_type, "text");
        assert!(!table.columns[0].nullable);
    }

    #[test]
    fn test_create_domain_with_named_constraint_check() {
        let catalog = Catalog::from_ddl(&[
            "CREATE DOMAIN money_amount AS NUMERIC(10,2) CONSTRAINT positive_amount CHECK (VALUE > 0);",
            "CREATE TABLE t (amt money_amount);",
        ])
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].sql_type, "numeric(10,2)");
    }

    // -- #185: bare-registered table must reject an arbitrary schema qualifier

    #[test]
    fn test_bare_table_rejects_wrong_schema_qualifier() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE users (id INTEGER NOT NULL, name TEXT NOT NULL);"]).unwrap();
        assert!(catalog.get_table("users").is_some());
        assert!(
            catalog.get_table("totally_wrong_schema.users").is_none(),
            "a bare-registered table must not accept an arbitrary schema qualifier"
        );
    }

    // -- #177: deterministic ambiguous-table resolution ----------------------

    #[test]
    fn test_ambiguous_table_lookup_is_deterministic_across_instances() {
        // `get_table`'s fallback iterated `AHashMap::iter().find(...)` --
        // whose order is randomized per `AHashMap` instance -- so an
        // unqualified lookup that matched more than one schema-qualified
        // table could resolve to a different table across otherwise
        // identical catalogs.
        let ddl = [
            "CREATE TABLE a.t (acol INTEGER NOT NULL);",
            "CREATE TABLE b.t (bcol TEXT NOT NULL);",
        ];
        let first = Catalog::from_ddl(&ddl).unwrap();
        let first_col = first.get_table("t").unwrap().columns[0].name.clone();

        for _ in 0..50 {
            let catalog = Catalog::from_ddl(&ddl).unwrap();
            let col = &catalog.get_table("t").unwrap().columns[0].name;
            assert_eq!(
                *col, first_col,
                "ambiguous unqualified lookup must resolve the same way across independently constructed catalogs"
            );
        }
    }

    // -- #108: SQLite PRIMARY KEY is not implicitly NOT NULL ----------------
    //
    // On SQLite, `PRIMARY KEY` does NOT imply `NOT NULL`, except for the
    // single-column `INTEGER PRIMARY KEY` rowid alias, or inside a
    // `WITHOUT ROWID` / `STRICT` table. Verified against SQLite 3.50.6.

    #[test]
    fn test_sqlite_integer_primary_key_is_not_null() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k INTEGER PRIMARY KEY);"], &SqlDialect::SQLite).unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(
            !k.nullable,
            "INTEGER PRIMARY KEY is the rowid alias and must be NOT NULL"
        );
    }

    #[test]
    fn test_sqlite_int_primary_key_is_nullable() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k INT PRIMARY KEY);"], &SqlDialect::SQLite).unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(
            k.nullable,
            "INT PRIMARY KEY is not the rowid alias and must stay nullable"
        );
    }

    #[test]
    fn test_sqlite_text_primary_key_is_nullable() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k TEXT PRIMARY KEY);"], &SqlDialect::SQLite).unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(k.nullable, "TEXT PRIMARY KEY must stay nullable on SQLite");
    }

    #[test]
    fn test_sqlite_integer_primary_key_autoincrement_is_not_null() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (k INTEGER PRIMARY KEY AUTOINCREMENT);"],
            &SqlDialect::SQLite,
        )
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(!k.nullable);
    }

    #[test]
    fn test_sqlite_without_rowid_forces_not_null() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (k TEXT PRIMARY KEY) WITHOUT ROWID;"],
            &SqlDialect::SQLite,
        )
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(!k.nullable, "WITHOUT ROWID tables enforce NOT NULL on the primary key");
    }

    #[test]
    fn test_sqlite_strict_forces_not_null() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k TEXT PRIMARY KEY) STRICT;"], &SqlDialect::SQLite)
                .unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(!k.nullable, "STRICT tables enforce NOT NULL on the primary key");
    }

    #[test]
    fn test_sqlite_composite_primary_key_is_nullable() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (a INTEGER, b TEXT, PRIMARY KEY (a, b));"],
            &SqlDialect::SQLite,
        )
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let a = table.columns.iter().find(|c| c.name == "a").unwrap();
        let b = table.columns.iter().find(|c| c.name == "b").unwrap();
        assert!(a.primary_key && b.primary_key);
        assert!(a.nullable, "composite primary key columns must stay nullable on SQLite");
        assert!(b.nullable, "composite primary key columns must stay nullable on SQLite");
    }

    #[test]
    fn test_sqlite_composite_primary_key_without_rowid_is_not_null() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (a INTEGER, b TEXT, PRIMARY KEY (a, b)) WITHOUT ROWID;"],
            &SqlDialect::SQLite,
        )
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let a = table.columns.iter().find(|c| c.name == "a").unwrap();
        let b = table.columns.iter().find(|c| c.name == "b").unwrap();
        assert!(!a.nullable);
        assert!(!b.nullable);
    }

    #[test]
    fn test_sqlite_single_column_table_constraint_integer_primary_key_is_not_null() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k INTEGER, PRIMARY KEY (k));"], &SqlDialect::SQLite)
                .unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(!k.nullable);
    }

    #[test]
    fn test_sqlite_explicit_not_null_with_text_primary_key_is_not_null() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k TEXT PRIMARY KEY NOT NULL);"], &SqlDialect::SQLite)
                .unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(!k.nullable);
    }

    #[test]
    fn test_sqlite_explicit_not_null_before_text_primary_key_is_not_null() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k TEXT NOT NULL PRIMARY KEY);"], &SqlDialect::SQLite)
                .unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(!k.nullable);
    }

    #[test]
    fn test_postgresql_primary_key_still_implies_not_null() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k INT PRIMARY KEY);"], &SqlDialect::PostgreSQL).unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(
            !k.nullable,
            "PostgreSQL PRIMARY KEY must remain unconditionally NOT NULL"
        );
    }

    #[test]
    fn test_mysql_primary_key_still_implies_not_null() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (k INT PRIMARY KEY);"], &SqlDialect::MySQL).unwrap();
        let table = catalog.get_table("t").unwrap();
        let k = &table.columns[0];
        assert!(k.primary_key);
        assert!(!k.nullable, "MySQL PRIMARY KEY must remain unconditionally NOT NULL");
    }

    #[test]
    fn test_postgresql_composite_primary_key_still_implies_not_null() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (a INT, b INT, PRIMARY KEY (a, b));"],
            &SqlDialect::PostgreSQL,
        )
        .unwrap();
        let table = catalog.get_table("t").unwrap();
        let a = table.columns.iter().find(|c| c.name == "a").unwrap();
        let b = table.columns.iter().find(|c| c.name == "b").unwrap();
        assert!(!a.nullable);
        assert!(!b.nullable);
    }

    #[test]
    fn test_sqlite_without_rowid_then_strict_is_not_null() {
        // sqlparser 0.62 parses `WITHOUT ROWID` immediately after the
        // column/constraint list and `STRICT` much later (after ORDER BY /
        // ON COMMIT / etc.) with no comma expected between them, so only
        // this order (and no comma) round-trips.
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (k TEXT PRIMARY KEY) WITHOUT ROWID STRICT;"],
            &SqlDialect::SQLite,
        );
        match catalog {
            Ok(catalog) => {
                let table = catalog.get_table("t").unwrap();
                assert!(!table.columns[0].nullable);
            }
            Err(err) => {
                panic!("expected WITHOUT ROWID STRICT to parse, got: {err}");
            }
        }
    }

    #[test]
    fn test_sqlite_reverse_table_options_order() {
        // issue #108 flagged that sqlparser reads `WITHOUT ROWID` and
        // `STRICT` at different points in `parse_create_table`, so the
        // reverse order (`STRICT` before `WITHOUT ROWID`) does not
        // round-trip. This documents the actual behavior rather than
        // silently masking it: if a future sqlparser upgrade changes this,
        // this test should fail and be updated rather than deleted.
        let result = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE t (k TEXT PRIMARY KEY) STRICT, WITHOUT ROWID;"],
            &SqlDialect::SQLite,
        );
        assert!(
            result.is_err(),
            "sqlparser 0.62 parses STRICT well before WITHOUT ROWID's position and does not expect a comma \
             between them, so `STRICT, WITHOUT ROWID` (STRICT first) is expected to fail to parse -- if this \
             now succeeds, sqlparser's grammar changed and this test should be updated to assert the new \
             behavior instead of just deleted"
        );
    }
}
