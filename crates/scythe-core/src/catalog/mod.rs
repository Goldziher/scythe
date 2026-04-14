pub(crate) mod type_normalizer;
mod view_resolver;

use ahash::AHashMap;
use sqlparser::ast::{
    AlterColumnOperation, AlterTableOperation, AlterTypeOperation, ColumnOption, ObjectName,
    Statement, TableConstraint, UserDefinedTypeRepresentation,
};
use sqlparser::parser::Parser;

use crate::dialect::SqlDialect;
use crate::errors::ScytheError;

use type_normalizer::{bare_name, ident_to_lower, normalize_data_type, object_name_to_key};

// ---------------------------------------------------------------------------
// Public data structures
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Catalog {
    tables: AHashMap<String, Table>,
    enums: AHashMap<String, EnumType>,
    composites: AHashMap<String, CompositeType>,
    /// Domain name -> resolved base type (lowercase)
    domains: AHashMap<String, DomainDef>,
}

#[derive(Debug, Clone)]
pub(crate) struct DomainDef {
    pub(crate) base_type: String,
    pub(crate) not_null: bool,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub columns: Vec<Column>,
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

// ---------------------------------------------------------------------------
// Constructor & accessors
// ---------------------------------------------------------------------------

impl Catalog {
    pub fn from_ddl(schema_sql: &[&str]) -> Result<Catalog, ScytheError> {
        Self::from_ddl_with_dialect(schema_sql, &SqlDialect::PostgreSQL)
    }

    pub fn from_ddl_with_dialect(
        schema_sql: &[&str],
        dialect: &SqlDialect,
    ) -> Result<Catalog, ScytheError> {
        let mut catalog = Catalog {
            tables: AHashMap::new(),
            enums: AHashMap::new(),
            composites: AHashMap::new(),
            domains: AHashMap::new(),
        };

        let parser_dialect = dialect.to_sqlparser_dialect();

        for sql in schema_sql {
            // Pre-process: extract statements that sqlparser cannot handle,
            // then feed the remainder to sqlparser.
            let cleaned = catalog.extract_unsupported_statements(sql, dialect);

            let trimmed = cleaned.trim();
            if trimmed.is_empty() {
                continue;
            }

            let statements = Parser::parse_sql(parser_dialect.as_ref(), &cleaned)
                .map_err(|e| ScytheError::syntax(e.to_string()))?;

            for stmt in statements {
                catalog.process_statement(stmt, dialect)?;
            }
        }

        Ok(catalog)
    }

    pub fn get_table(&self, name: &str) -> Option<&Table> {
        let lower = name.to_lowercase();
        self.tables.get(&lower).or_else(|| {
            // Try stripping schema prefix for lookup
            if let Some((_schema, table)) = lower.split_once('.') {
                self.tables.get(table)
            } else {
                // Try finding with any schema prefix
                self.tables
                    .iter()
                    .find(|(k, _)| k.ends_with(&format!(".{}", lower)) || k.as_str() == lower)
                    .map(|(_, v)| v)
            }
        })
    }

    pub fn get_enum(&self, name: &str) -> Option<&EnumType> {
        let lower = name.to_lowercase();
        self.enums.get(&lower).or_else(|| {
            if let Some((_schema, type_name)) = lower.split_once('.') {
                self.enums.get(type_name)
            } else {
                self.enums
                    .iter()
                    .find(|(k, _)| k.ends_with(&format!(".{}", lower)))
                    .map(|(_, v)| v)
            }
        })
    }

    /// Iterate over all table names in the catalog.
    pub fn tables(&self) -> impl Iterator<Item = &String> {
        self.tables.keys()
    }

    /// Iterate over all enum names in the catalog.
    pub fn enums_iter(&self) -> impl Iterator<Item = (&String, &EnumType)> {
        self.enums.iter()
    }

    /// Look up a domain's resolved base type by name.
    pub fn get_domain_base_type(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        self.domains
            .get(&lower)
            .map(|d| d.base_type.as_str())
            .or_else(|| {
                if let Some((_schema, type_name)) = lower.split_once('.') {
                    self.domains.get(type_name).map(|d| d.base_type.as_str())
                } else {
                    self.domains
                        .iter()
                        .find(|(k, _)| k.ends_with(&format!(".{}", lower)))
                        .map(|(_, d)| d.base_type.as_str())
                }
            })
    }

    pub fn get_composite(&self, name: &str) -> Option<&CompositeType> {
        let lower = name.to_lowercase();
        self.composites.get(&lower).or_else(|| {
            if let Some((_schema, type_name)) = lower.split_once('.') {
                self.composites.get(type_name)
            } else {
                self.composites
                    .iter()
                    .find(|(k, _)| k.ends_with(&format!(".{}", lower)))
                    .map(|(_, v)| v)
            }
        })
    }
}

// ---------------------------------------------------------------------------
// CREATE DOMAIN manual parsing (sqlparser 0.55 doesn't support it)
// ---------------------------------------------------------------------------

impl Catalog {
    /// Pre-process a SQL string to extract statements that sqlparser cannot handle
    /// (CREATE DOMAIN, CREATE SCHEMA). Processes them internally and returns the
    /// remaining SQL with those statements removed.
    fn extract_unsupported_statements(&mut self, sql: &str, dialect: &SqlDialect) -> String {
        let mut result = String::with_capacity(sql.len());
        // Split on semicolons, but be careful about content within parentheses and quotes
        for raw_stmt in Self::split_top_level_statements(sql) {
            let trimmed = raw_stmt.trim();
            if trimmed.is_empty() || trimmed.starts_with("--") && !trimmed.contains('\n') {
                result.push_str(raw_stmt);
                continue;
            }
            // Strip leading comments to check the actual statement
            let no_comments = Self::strip_leading_comments(trimmed);
            let upper = no_comments.to_uppercase();
            if upper.starts_with("CREATE DOMAIN") {
                self.try_parse_create_domain(no_comments, dialect);
                // Replace with empty to remove from output
            } else if upper.starts_with("CREATE SCHEMA") {
                // Silently ignore
            } else {
                // For Redshift/MSSQL, strip IDENTITY(seed,step) patterns that PostgreSQL parser doesn't support
                let stmt_to_add = if matches!(dialect, SqlDialect::PostgreSQL | SqlDialect::MsSql) {
                    Self::strip_identity_patterns(raw_stmt)
                } else {
                    raw_stmt.to_string()
                };
                result.push_str(&stmt_to_add);
                // Ensure there's a semicolon separator if the original had one
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
    fn strip_identity_patterns(sql: &str) -> String {
        let mut result = String::with_capacity(sql.len());
        let bytes = sql.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            // Check for "IDENTITY" keyword (case-insensitive) at word boundary
            if i + 8 <= bytes.len() && Self::matches_identity_keyword(bytes, i) {
                // Verify this is a word boundary (not preceded by alphanumeric/underscore)
                let is_start_boundary =
                    i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
                if !is_start_boundary {
                    result.push(bytes[i] as char);
                    i += 1;
                    continue;
                }

                // Skip past "IDENTITY"
                i += 8;
                // Skip whitespace
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                // Check for opening parenthesis
                if i < bytes.len() && bytes[i] == b'(' {
                    // Try to parse IDENTITY(N,N)
                    let mut j = i + 1;
                    let mut found_valid_pattern = false;

                    // Skip whitespace after opening paren
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    // Try to parse first number
                    let num_start = j;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j > num_start {
                        // Found first number, now look for comma
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if j < bytes.len() && bytes[j] == b',' {
                            j += 1;
                            // Skip whitespace after comma
                            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                                j += 1;
                            }
                            // Try to parse second number
                            let num_start2 = j;
                            while j < bytes.len() && bytes[j].is_ascii_digit() {
                                j += 1;
                            }
                            if j > num_start2 {
                                // Found second number, now look for closing paren
                                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                                    j += 1;
                                }
                                if j < bytes.len() && bytes[j] == b')' {
                                    // Valid IDENTITY(N,N) pattern found - skip it
                                    i = j + 1;
                                    found_valid_pattern = true;
                                }
                            }
                        }
                    }

                    if !found_valid_pattern {
                        // Not a valid IDENTITY pattern, output the opening paren
                        result.push_str("IDENTITY");
                        result.push('(');
                        i += 9;
                    }
                } else {
                    // No opening paren after IDENTITY, just output it
                    result.push_str("IDENTITY");
                }
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }

        result
    }

    /// Check if bytes at position i match the IDENTITY keyword (case-insensitive)
    fn matches_identity_keyword(bytes: &[u8], i: usize) -> bool {
        if i + 8 > bytes.len() {
            return false;
        }

        const IDENTITY_UPPER: &[u8; 8] = b"IDENTITY";
        const IDENTITY_LOWER: &[u8; 8] = b"identity";

        if bytes[i..i + 8] == *IDENTITY_UPPER {
            return true;
        }
        if bytes[i..i + 8] == *IDENTITY_LOWER {
            return true;
        }

        // Check for mixed case
        bytes[i..i + 8]
            .iter()
            .zip(IDENTITY_UPPER.iter())
            .all(|(b, ub)| b.to_ascii_uppercase() == *ub)
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
                        i += 2; // escaped quote
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
        let upper = trimmed.to_uppercase();
        if !upper.starts_with("CREATE DOMAIN") {
            return false;
        }
        // Strip trailing semicolons
        let trimmed = trimmed.trim_end_matches(';').trim();
        // Pattern: CREATE DOMAIN <name> AS <type> [NOT NULL] [CHECK (...)]
        // Find "AS" keyword
        let upper = trimmed.to_uppercase();
        let as_pos = match upper.find(" AS ") {
            Some(p) => p,
            None => return true, // It's a CREATE DOMAIN but malformed; skip
        };
        let domain_name = trimmed["CREATE DOMAIN".len()..as_pos].trim().to_lowercase();
        let rest = trimmed[as_pos + 4..].trim();

        // Extract base type: everything before NOT NULL or CHECK
        let rest_upper = rest.to_uppercase();
        let end_pos = rest_upper
            .find(" NOT NULL")
            .or_else(|| rest_upper.find(" CHECK"))
            .or_else(|| rest_upper.find(" DEFAULT"))
            .unwrap_or(rest.len());
        let base_type_raw = rest[..end_pos].trim();

        let not_null = rest_upper.contains("NOT NULL");

        // Parse the base type through sqlparser to normalize it
        let parser_dialect = dialect.to_sqlparser_dialect();
        let normalized = match Parser::parse_sql(
            parser_dialect.as_ref(),
            &format!("CREATE TABLE _domain_tmp_ (_col_ {})", base_type_raw),
        ) {
            Ok(stmts) => {
                if let Some(Statement::CreateTable(ct)) = stmts.into_iter().next() {
                    if let Some(col) = ct.columns.first() {
                        let (t, _) = normalize_data_type(&col.data_type, &self.domains);
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

// ---------------------------------------------------------------------------
// Statement processing
// ---------------------------------------------------------------------------

impl Catalog {
    fn process_statement(
        &mut self,
        stmt: Statement,
        dialect: &SqlDialect,
    ) -> Result<(), ScytheError> {
        match stmt {
            Statement::CreateTable(ct) => self.process_create_table(ct, dialect),
            Statement::AlterTable(alter_table) => {
                self.process_alter_table(alter_table.name, alter_table.operations)
            }
            Statement::CreateType {
                name,
                representation,
            } => {
                if let Some(repr) = representation {
                    self.process_create_type(name, repr)
                } else {
                    Ok(())
                }
            }
            Statement::AlterType(alter_type) => {
                self.process_alter_type(alter_type.name, alter_type.operation)
            }
            Statement::CreateView(cv) => {
                self.process_create_view(cv.name, cv.columns, *cv.query, cv.materialized)
            }
            // Silently ignore statements we don't handle
            _ => Ok(()),
        }
    }

    // -----------------------------------------------------------------------
    // CREATE TABLE
    // -----------------------------------------------------------------------

    fn process_create_table(
        &mut self,
        ct: sqlparser::ast::CreateTable,
        dialect: &SqlDialect,
    ) -> Result<(), ScytheError> {
        let table_name = object_name_to_key(&ct.name);
        let mut columns: Vec<Column> = Vec::new();

        for col_def in &ct.columns {
            let col_name = ident_to_lower(&col_def.name);
            let (sql_type, is_serial) = normalize_data_type(&col_def.data_type, &self.domains);

            // Handle MySQL/SQLite inline ENUM: if the data type is an Enum variant,
            // register it in the catalog and use the enum type name.
            let sql_type = if let sqlparser::ast::DataType::Enum(variants, _bits) =
                &col_def.data_type
            {
                if matches!(dialect, SqlDialect::MySQL | SqlDialect::SQLite) && !variants.is_empty()
                {
                    // Synthesize an enum name from table_name + col_name
                    let enum_key = format!("{}_{}", table_name.replace('.', "_"), col_name);
                    let values: Vec<String> = variants
                        .iter()
                        .map(|v| match v {
                            sqlparser::ast::EnumMember::Name(name) => {
                                name.trim_matches('\'').to_string()
                            }
                            sqlparser::ast::EnumMember::NamedValue(name, _) => {
                                name.trim_matches('\'').to_string()
                            }
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

            let mut nullable = !is_serial; // serial types are NOT NULL
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
                        nullable = false;
                    }
                    ColumnOption::Unique(_) => {}
                    ColumnOption::Generated {
                        generation_expr: Some(expr),
                        ..
                    } => {
                        default = Some(format!("GENERATED ALWAYS AS ({})", expr));
                    }
                    ColumnOption::DialectSpecific(tokens) => {
                        // Detect AUTO_INCREMENT (MySQL) and AUTOINCREMENT (SQLite)
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

            // AUTO_INCREMENT / AUTOINCREMENT implies NOT NULL
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

        // Process table-level constraints
        for constraint in &ct.constraints {
            if let TableConstraint::PrimaryKey(pk_constraint) = constraint {
                for idx_col in &pk_constraint.columns {
                    let pk_name = idx_col.column.expr.to_string().to_lowercase();
                    if let Some(col) = columns.iter_mut().find(|c| c.name == pk_name) {
                        col.primary_key = true;
                        col.nullable = false;
                    }
                }
            }
        }

        self.tables.insert(table_name, Table { columns });
        Ok(())
    }

    // -----------------------------------------------------------------------
    // ALTER TABLE
    // -----------------------------------------------------------------------

    fn process_alter_table(
        &mut self,
        name: ObjectName,
        operations: Vec<AlterTableOperation>,
    ) -> Result<(), ScytheError> {
        let table_key = object_name_to_key(&name);

        for op in operations {
            match op {
                AlterTableOperation::AddColumn { column_def, .. } => {
                    let table = get_table_mut(&mut self.tables, &table_key);
                    if let Some(table) = table {
                        let col_name = ident_to_lower(&column_def.name);
                        let (sql_type, is_serial) =
                            normalize_data_type(&column_def.data_type, &self.domains);
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
                                    nullable = false;
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
                }
                AlterTableOperation::DropColumn { column_names, .. } => {
                    let table = get_table_mut(&mut self.tables, &table_key);
                    if let Some(table) = table {
                        for column_name in &column_names {
                            let col_lower = ident_to_lower(column_name);
                            table.columns.retain(|c| c.name != col_lower);
                        }
                    }
                }
                AlterTableOperation::RenameColumn {
                    old_column_name,
                    new_column_name,
                } => {
                    let table = get_table_mut(&mut self.tables, &table_key);
                    if let Some(table) = table {
                        let old_name = ident_to_lower(&old_column_name);
                        let new_name = ident_to_lower(&new_column_name);
                        if let Some(col) = table.columns.iter_mut().find(|c| c.name == old_name) {
                            col.name = new_name;
                        }
                    }
                }
                AlterTableOperation::RenameTable { table_name } => {
                    let new_key = match &table_name {
                        sqlparser::ast::RenameTableNameKind::To(name)
                        | sqlparser::ast::RenameTableNameKind::As(name) => object_name_to_key(name),
                    };
                    if let Some(table) = self.tables.remove(&table_key) {
                        self.tables.insert(new_key, table);
                    } else {
                        // try bare name
                        let bare = bare_name(&table_key).to_string();
                        if let Some(table) = self.tables.remove(&bare) {
                            self.tables.insert(new_key, table);
                        }
                    }
                }
                AlterTableOperation::AlterColumn { column_name, op } => {
                    let table = get_table_mut(&mut self.tables, &table_key);
                    if let Some(table) = table {
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
                                    let (new_type, _) =
                                        normalize_data_type(&data_type, &self.domains);
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
                }
                AlterTableOperation::AddConstraint { constraint, .. } => {
                    let table = get_table_mut(&mut self.tables, &table_key);
                    if let Some(table) = table
                        && let TableConstraint::PrimaryKey(pk_constraint) = &constraint
                    {
                        for idx_col in &pk_constraint.columns {
                            let pk_name = idx_col.column.expr.to_string().to_lowercase();
                            if let Some(col) = table.columns.iter_mut().find(|c| c.name == pk_name)
                            {
                                col.primary_key = true;
                                col.nullable = false;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // CREATE TYPE
    // -----------------------------------------------------------------------

    fn process_create_type(
        &mut self,
        name: ObjectName,
        repr: UserDefinedTypeRepresentation,
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
                        let (ft, _) = normalize_data_type(&attr.data_type, &self.domains);
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

    // -----------------------------------------------------------------------
    // ALTER TYPE
    // -----------------------------------------------------------------------

    fn process_alter_type(
        &mut self,
        name: ObjectName,
        operation: AlterTypeOperation,
    ) -> Result<(), ScytheError> {
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_table_mut<'a>(tables: &'a mut AHashMap<String, Table>, key: &str) -> Option<&'a mut Table> {
    if tables.contains_key(key) {
        return tables.get_mut(key);
    }
    let bare = bare_name(key);
    let found_key = tables
        .keys()
        .find(|k| k.as_str() == bare || k.ends_with(&format!(".{}", bare)))
        .cloned();
    found_key.and_then(move |k| tables.get_mut(&k))
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
        let catalog =
            Catalog::from_ddl(&["CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');"]).unwrap();

        let mood = catalog.get_enum("mood").unwrap();
        assert_eq!(mood.values, vec!["sad", "ok", "happy"]);
    }

    #[test]
    fn test_composite_type() {
        let catalog =
            Catalog::from_ddl(&["CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);"])
                .unwrap();

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
        let catalog =
            Catalog::from_ddl(&["CREATE TABLE t (tags TEXT[], scores INTEGER[]);"]).unwrap();

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

    // -----------------------------------------------------------------------
    // MySQL dialect tests
    // -----------------------------------------------------------------------

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
        // The inline enum should have been registered in the catalog
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

    // -----------------------------------------------------------------------
    // SQLite dialect tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Dialect backward compat: from_ddl still works
    // -----------------------------------------------------------------------

    #[test]
    fn test_from_ddl_backward_compat() {
        // Ensure the original from_ddl signature still works
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER);"]).unwrap();
        assert!(catalog.get_table("t").is_some());
    }

    // -----------------------------------------------------------------------
    // Redshift IDENTITY support
    // -----------------------------------------------------------------------

    #[test]
    fn test_redshift_identity_stripping() {
        // Test that IDENTITY(1,1) is stripped for Redshift (which maps to PostgreSQL dialect)
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
        // Test that IDENTITY(seed,step) is stripped for MSSQL
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
        // Test IDENTITY with various whitespace patterns
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
    fn test_redshift_full_schema() {
        // Test parsing the full Redshift integration test schema
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

        // Verify all tables were parsed
        assert!(catalog.get_table("users").is_some());
        assert!(catalog.get_table("orders").is_some());
        assert!(catalog.get_table("tags").is_some());
        assert!(catalog.get_table("user_tags").is_some());

        // Verify users table
        let users = catalog.get_table("users").unwrap();
        assert_eq!(users.columns.len(), 5);
        assert_eq!(users.columns[0].name, "id");
        assert!(!users.columns[0].nullable);
        assert_eq!(users.columns[1].name, "name");
        assert!(!users.columns[1].nullable);
        assert_eq!(users.columns[2].name, "email");
        assert!(users.columns[2].nullable);

        // Verify orders table
        let orders = catalog.get_table("orders").unwrap();
        assert_eq!(orders.columns.len(), 5);
        assert_eq!(orders.columns[0].name, "id");
        assert!(!orders.columns[0].nullable);
    }

    #[test]
    fn test_identity_case_insensitive() {
        // Test that IDENTITY works regardless of case
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
}
