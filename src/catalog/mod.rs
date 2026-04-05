use ahash::AHashMap;
use sqlparser::ast::{
    AlterColumnOperation, AlterTableOperation, AlterTypeOperation, ArrayElemTypeDef, ColumnOption,
    DataType, ObjectName, Statement, TableConstraint, TimezoneInfo, UserDefinedTypeRepresentation,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::errors::ScytheError;

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
#[allow(dead_code)]
struct DomainDef {
    base_type: String,
    not_null: bool,
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
        let mut catalog = Catalog {
            tables: AHashMap::new(),
            enums: AHashMap::new(),
            composites: AHashMap::new(),
            domains: AHashMap::new(),
        };

        let dialect = PostgreSqlDialect {};

        for sql in schema_sql {
            // Handle CREATE DOMAIN manually since sqlparser doesn't support it
            if catalog.try_parse_create_domain(sql) {
                continue;
            }
            // Handle CREATE SCHEMA (silently ignore)
            let trimmed = sql.trim().to_lowercase();
            if trimmed.starts_with("create schema") {
                continue;
            }

            let statements =
                Parser::parse_sql(&dialect, sql).map_err(|e| ScytheError::syntax(e.to_string()))?;

            for stmt in statements {
                catalog.process_statement(stmt)?;
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
    /// Try to parse `CREATE DOMAIN <name> AS <type> [NOT NULL] [CHECK ...]`.
    /// Returns true if the SQL was a CREATE DOMAIN statement (even if parsing
    /// was only partial).
    fn try_parse_create_domain(&mut self, sql: &str) -> bool {
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
        let dialect = PostgreSqlDialect {};
        let normalized = match Parser::parse_sql(
            &dialect,
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
    fn process_statement(&mut self, stmt: Statement) -> Result<(), ScytheError> {
        match stmt {
            Statement::CreateTable(ct) => self.process_create_table(ct),
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

    fn process_create_table(&mut self, ct: sqlparser::ast::CreateTable) -> Result<(), ScytheError> {
        let table_name = object_name_to_key(&ct.name);
        let mut columns: Vec<Column> = Vec::new();

        for col_def in &ct.columns {
            let col_name = ident_to_lower(&col_def.name);
            let (sql_type, is_serial) = normalize_data_type(&col_def.data_type, &self.domains);

            let mut nullable = !is_serial; // serial types are NOT NULL
            let mut default: Option<String> = None;
            let mut primary_key = false;

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
                    _ => {}
                }
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

    // -----------------------------------------------------------------------
    // CREATE VIEW / CREATE MATERIALIZED VIEW
    // -----------------------------------------------------------------------

    fn process_create_view(
        &mut self,
        name: ObjectName,
        view_columns: Vec<sqlparser::ast::ViewColumnDef>,
        query: sqlparser::ast::Query,
        _materialized: bool,
    ) -> Result<(), ScytheError> {
        let view_key = object_name_to_key(&name);

        // If explicit column list is provided with types, use those
        if !view_columns.is_empty() {
            let columns: Vec<Column> = view_columns
                .iter()
                .map(|vc| {
                    let sql_type = vc
                        .data_type
                        .as_ref()
                        .map(|dt| {
                            let (t, _) = normalize_data_type(dt, &self.domains);
                            t
                        })
                        .unwrap_or_else(|| "unknown".to_string());
                    Column {
                        name: ident_to_lower(&vc.name),
                        sql_type,
                        nullable: true,
                        default: None,
                        primary_key: false,
                    }
                })
                .collect();
            self.tables.insert(view_key, Table { columns });
            return Ok(());
        }

        // Try to resolve column types from the view's query
        let columns = self.resolve_view_columns(&query);
        self.tables.insert(view_key, Table { columns });
        Ok(())
    }

    /// Resolve columns from a view's SELECT query by matching against known tables.
    fn resolve_view_columns(&self, query: &sqlparser::ast::Query) -> Vec<Column> {
        use sqlparser::ast::{SelectItem, SetExpr, TableFactor};

        if let SetExpr::Select(select) = query.body.as_ref() {
            // Build a map of alias/table -> columns from FROM clause
            let mut source_cols: Vec<(String, String, Vec<Column>)> = Vec::new(); // (alias, table_name, columns)
            for twj in &select.from {
                if let TableFactor::Table { name, alias, .. } = &twj.relation {
                    let table_name = object_name_to_key(name);
                    let alias_name = alias
                        .as_ref()
                        .map(|a| a.name.value.to_lowercase())
                        .unwrap_or_else(|| bare_name(&table_name).to_string());
                    if let Some(table) = self
                        .tables
                        .get(&table_name)
                        .or_else(|| self.tables.get(bare_name(&table_name)))
                    {
                        source_cols.push((alias_name, table_name, table.columns.clone()));
                    }
                }
                for join in &twj.joins {
                    if let TableFactor::Table { name, alias, .. } = &join.relation {
                        let table_name = object_name_to_key(name);
                        let alias_name = alias
                            .as_ref()
                            .map(|a| a.name.value.to_lowercase())
                            .unwrap_or_else(|| bare_name(&table_name).to_string());
                        if let Some(table) = self
                            .tables
                            .get(&table_name)
                            .or_else(|| self.tables.get(bare_name(&table_name)))
                        {
                            source_cols.push((alias_name, table_name, table.columns.clone()));
                        }
                    }
                }
            }

            let mut result = Vec::new();
            for item in &select.projection {
                match item {
                    SelectItem::UnnamedExpr(expr) => {
                        let (name, sql_type, nullable) = self.resolve_view_expr(expr, &source_cols);
                        result.push(Column {
                            name,
                            sql_type,
                            nullable,
                            default: None,
                            primary_key: false,
                        });
                    }
                    SelectItem::ExprWithAlias { expr, alias } => {
                        let (_, sql_type, nullable) = self.resolve_view_expr(expr, &source_cols);
                        result.push(Column {
                            name: ident_to_lower(alias),
                            sql_type,
                            nullable,
                            default: None,
                            primary_key: false,
                        });
                    }
                    SelectItem::Wildcard(_) => {
                        for (_, _, cols) in &source_cols {
                            for col in cols {
                                result.push(col.clone());
                            }
                        }
                    }
                    SelectItem::QualifiedWildcard(kind, _) => {
                        let qualifier = match kind {
                            sqlparser::ast::SelectItemQualifiedWildcardKind::ObjectName(name) => {
                                object_name_to_key(name)
                            }
                            _ => String::new(),
                        };
                        for (alias, tname, cols) in &source_cols {
                            if *alias == qualifier || *tname == qualifier {
                                for col in cols {
                                    result.push(col.clone());
                                }
                            }
                        }
                    }
                }
            }
            if !result.is_empty() {
                return result;
            }
        }
        Vec::new()
    }

    /// Resolve a single expression in a view SELECT to (name, sql_type, nullable)
    fn resolve_view_expr(
        &self,
        expr: &sqlparser::ast::Expr,
        sources: &[(String, String, Vec<Column>)],
    ) -> (String, String, bool) {
        use sqlparser::ast::Expr;
        match expr {
            Expr::Identifier(ident) => {
                let col_name = ident_to_lower(ident);
                for (_, _, cols) in sources {
                    if let Some(col) = cols.iter().find(|c| c.name == col_name) {
                        return (col_name, col.sql_type.clone(), col.nullable);
                    }
                }
                (col_name, "unknown".to_string(), true)
            }
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let qualifier = parts[0].value.to_lowercase();
                let col_name = ident_to_lower(&parts[1]);
                for (alias, _, cols) in sources {
                    if *alias == qualifier
                        && let Some(col) = cols.iter().find(|c| c.name == col_name)
                    {
                        return (col_name, col.sql_type.clone(), col.nullable);
                    }
                }
                (col_name, "unknown".to_string(), true)
            }
            Expr::Function(func) => {
                let func_name = object_name_to_key(&func.name).to_lowercase();
                let name = func_name.clone();
                match func_name.as_str() {
                    "count" => (name, "bigint".to_string(), false),
                    "sum" => (name, "bigint".to_string(), true),
                    "avg" => (name, "numeric".to_string(), true),
                    "min" | "max" => {
                        // Try to get arg type
                        if let sqlparser::ast::FunctionArguments::List(args) = &func.args
                            && let Some(first) = args.args.first()
                            && let sqlparser::ast::FunctionArg::Unnamed(
                                sqlparser::ast::FunctionArgExpr::Expr(e),
                            ) = first
                        {
                            let (_, t, _) = self.resolve_view_expr(e, sources);
                            return (name, t, true);
                        }
                        (name, "unknown".to_string(), true)
                    }
                    _ => (name, "unknown".to_string(), true),
                }
            }
            _ => ("unknown".to_string(), "unknown".to_string(), true),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn object_name_to_key(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|part| match part {
            sqlparser::ast::ObjectNamePart::Identifier(ident) => ident.value.to_lowercase(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn ident_to_lower(ident: &sqlparser::ast::Ident) -> String {
    // Preserve case for double-quoted identifiers
    if ident.quote_style.is_some() {
        ident.value.clone()
    } else {
        ident.value.to_lowercase()
    }
}

fn bare_name(key: &str) -> &str {
    key.rsplit_once('.').map_or(key, |(_, name)| name)
}

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

/// Normalize a sqlparser DataType into a lowercase PostgreSQL type string.
/// Returns (type_string, is_serial).
fn normalize_data_type(dt: &DataType, domains: &AHashMap<String, DomainDef>) -> (String, bool) {
    match dt {
        // Custom types (includes serial, timestamptz, and user-defined types)
        DataType::Custom(name, _tokens) => {
            let raw = object_name_to_key(name);
            match raw.as_str() {
                "serial" | "serial4" => return ("integer".to_string(), true),
                "bigserial" | "serial8" => return ("bigint".to_string(), true),
                "smallserial" | "serial2" => return ("smallint".to_string(), true),
                "timestamptz" => return ("timestamptz".to_string(), false),
                "timetz" => return ("timetz".to_string(), false),
                _ => {}
            }
            // Check if it's a domain
            if let Some(domain) = domains.get(&raw) {
                return (domain.base_type.clone(), false);
            }
            (raw, false)
        }

        // Integer family
        DataType::Int(_) | DataType::Int4(_) | DataType::Integer(_) => {
            ("integer".to_string(), false)
        }
        DataType::SmallInt(_) | DataType::Int2(_) => ("smallint".to_string(), false),
        DataType::BigInt(_) | DataType::Int8(_) => ("bigint".to_string(), false),

        // Boolean
        DataType::Bool | DataType::Boolean => ("boolean".to_string(), false),

        // Float family
        DataType::Real | DataType::Float4 => ("real".to_string(), false),
        DataType::DoublePrecision | DataType::Float8 => ("double precision".to_string(), false),
        DataType::Float(info) => {
            use sqlparser::ast::ExactNumberInfo;
            match info {
                ExactNumberInfo::Precision(p) if *p <= 24 => ("real".to_string(), false),
                ExactNumberInfo::Precision(_) | ExactNumberInfo::PrecisionAndScale(_, _) => {
                    ("double precision".to_string(), false)
                }
                ExactNumberInfo::None => ("double precision".to_string(), false),
            }
        }

        // Character types
        DataType::Varchar(len) | DataType::CharVarying(len) | DataType::CharacterVarying(len) => {
            match len {
                Some(sqlparser::ast::CharacterLength::IntegerLength { length, .. }) => {
                    (format!("varchar({})", length), false)
                }
                _ => ("text".to_string(), false),
            }
        }
        DataType::Char(len) | DataType::Character(len) => match len {
            Some(sqlparser::ast::CharacterLength::IntegerLength { length, .. }) => {
                (format!("char({})", length), false)
            }
            _ => ("char(1)".to_string(), false),
        },
        DataType::Text => ("text".to_string(), false),

        // Numeric
        DataType::Numeric(info) | DataType::Decimal(info) | DataType::Dec(info) => {
            use sqlparser::ast::ExactNumberInfo;
            match info {
                ExactNumberInfo::PrecisionAndScale(p, s) => {
                    (format!("numeric({},{})", p, s), false)
                }
                ExactNumberInfo::Precision(p) => (format!("numeric({})", p), false),
                ExactNumberInfo::None => ("numeric".to_string(), false),
            }
        }

        // Date/time
        DataType::Date => ("date".to_string(), false),
        DataType::Time(prec, tz) => {
            let base = match tz {
                TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "timetz",
                TimezoneInfo::WithoutTimeZone | TimezoneInfo::None => "time",
            };
            match prec {
                Some(p) => (format!("{}({})", base, p), false),
                None => (base.to_string(), false),
            }
        }
        DataType::Timestamp(prec, tz) => {
            let base = match tz {
                TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "timestamptz",
                TimezoneInfo::WithoutTimeZone | TimezoneInfo::None => "timestamp",
            };
            match prec {
                Some(p) => (format!("{}({})", base, p), false),
                None => (base.to_string(), false),
            }
        }
        DataType::Interval { .. } => ("interval".to_string(), false),

        // JSON
        DataType::JSON => ("json".to_string(), false),
        DataType::JSONB => ("jsonb".to_string(), false),

        // Binary
        DataType::Bytea => ("bytea".to_string(), false),

        // UUID
        DataType::Uuid => ("uuid".to_string(), false),

        // Array types
        DataType::Array(elem) => {
            let inner = match elem {
                ArrayElemTypeDef::SquareBracket(inner_dt, _) => {
                    let (inner_type, _) = normalize_data_type(inner_dt, domains);
                    inner_type
                }
                ArrayElemTypeDef::AngleBracket(inner_dt) => {
                    let (inner_type, _) = normalize_data_type(inner_dt, domains);
                    inner_type
                }
                ArrayElemTypeDef::Parenthesis(inner_dt) => {
                    let (inner_type, _) = normalize_data_type(inner_dt, domains);
                    inner_type
                }
                ArrayElemTypeDef::None => "unknown".to_string(),
            };
            // Use short forms for common array element types
            let short = match inner.as_str() {
                "integer" => "int",
                "character varying" => "text",
                _ => &inner,
            };
            (format!("{}[]", short), false)
        }

        // Bit types
        DataType::Bit(_) => ("bit".to_string(), false),
        DataType::BitVarying(_) | DataType::VarBit(_) => ("bit varying".to_string(), false),

        // Fallback: use the Display impl and lowercase it
        other => (other.to_string().to_lowercase(), false),
    }
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
}
