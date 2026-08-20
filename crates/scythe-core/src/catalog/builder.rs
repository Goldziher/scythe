use ahash::{AHashMap, AHashSet};

use crate::dialect::SqlDialect;
use crate::errors::ScytheError;

use super::{Catalog, Column, CompositeField, CompositeType, DomainDef, EnumType, Table};

/// A database object name with its original identifier spelling preserved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CatalogObjectName {
    schema: Option<String>,
    name: String,
}

impl CatalogObjectName {
    /// Construct an unqualified object name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: None,
            name: name.into(),
        }
    }

    /// Construct a schema-qualified object name.
    pub fn qualified(schema: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            schema: Some(schema.into()),
            name: name.into(),
        }
    }

    /// Return the preserved schema spelling, if one was provided.
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// Return the preserved bare object-name spelling.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn normalized_key(&self, object_kind: &str) -> Result<String, ScytheError> {
        let name = normalized_identifier(&self.name, object_kind)?;
        match &self.schema {
            Some(schema) => Ok(format!("{}.{}", normalized_identifier(schema, "schema")?, name)),
            None => Ok(name),
        }
    }
}

/// Whether an inspected relation is a base table or a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    /// A base table.
    Table,
    /// A view.
    View,
}

/// An inspected column definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDefinition {
    name: String,
    raw_sql_type: String,
    nullable: bool,
    default: Option<String>,
    primary_key: bool,
}

impl ColumnDefinition {
    /// Construct a column using the database-reported type and nullability.
    pub fn new(name: impl Into<String>, raw_sql_type: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            raw_sql_type: raw_sql_type.into(),
            nullable,
            default: None,
            primary_key: false,
        }
    }

    /// Attach the database-reported default expression.
    #[must_use]
    pub fn default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Mark this column as part of the primary key.
    #[must_use]
    pub fn primary_key(mut self) -> Self {
        self.primary_key = true;
        self
    }
}

/// An inspected table or view definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDefinition {
    name: CatalogObjectName,
    kind: RelationKind,
    columns: Vec<ColumnDefinition>,
}

impl RelationDefinition {
    /// Construct a table definition.
    pub fn table(name: CatalogObjectName, columns: Vec<ColumnDefinition>) -> Self {
        Self {
            name,
            kind: RelationKind::Table,
            columns,
        }
    }

    /// Construct a view definition.
    pub fn view(name: CatalogObjectName, columns: Vec<ColumnDefinition>) -> Self {
        Self {
            name,
            kind: RelationKind::View,
            columns,
        }
    }
}

/// An inspected enum definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDefinition {
    name: CatalogObjectName,
    values: Vec<String>,
}

impl EnumDefinition {
    /// Construct an enum definition.
    pub fn new(name: CatalogObjectName, values: Vec<String>) -> Self {
        Self { name, values }
    }
}

/// One field in an inspected composite type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeFieldDefinition {
    name: String,
    raw_sql_type: String,
}

impl CompositeFieldDefinition {
    /// Construct a composite field definition.
    pub fn new(name: impl Into<String>, raw_sql_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            raw_sql_type: raw_sql_type.into(),
        }
    }
}

/// An inspected composite type definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeDefinition {
    name: CatalogObjectName,
    fields: Vec<CompositeFieldDefinition>,
}

impl CompositeDefinition {
    /// Construct a composite definition.
    pub fn new(name: CatalogObjectName, fields: Vec<CompositeFieldDefinition>) -> Self {
        Self { name, fields }
    }
}

/// An inspected domain definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainDefinition {
    name: CatalogObjectName,
    raw_base_type: String,
    not_null: bool,
}

impl DomainDefinition {
    /// Construct a domain definition.
    pub fn new(name: CatalogObjectName, raw_base_type: impl Into<String>, not_null: bool) -> Self {
        Self {
            name,
            raw_base_type: raw_base_type.into(),
            not_null,
        }
    }
}

/// Validates inspected schema definitions before constructing a [`Catalog`].
#[derive(Debug)]
pub struct CatalogBuilder {
    dialect: SqlDialect,
    engine: Option<String>,
    relations: Vec<RelationDefinition>,
    enums: Vec<EnumDefinition>,
    composites: Vec<CompositeDefinition>,
    domains: Vec<DomainDefinition>,
}

impl CatalogBuilder {
    /// Start a catalog builder for a SQL dialect.
    pub fn new(dialect: SqlDialect) -> Self {
        Self {
            dialect,
            engine: None,
            relations: Vec::new(),
            enums: Vec::new(),
            composites: Vec::new(),
            domains: Vec::new(),
        }
    }

    /// Attach the concrete engine name when it is known.
    #[must_use]
    pub fn engine(mut self, engine: impl Into<String>) -> Self {
        self.engine = Some(engine.into());
        self
    }

    /// Add a table or view definition.
    #[must_use]
    pub fn relation(mut self, definition: RelationDefinition) -> Self {
        self.relations.push(definition);
        self
    }

    /// Add an enum definition.
    #[must_use]
    pub fn enum_type(mut self, definition: EnumDefinition) -> Self {
        self.enums.push(definition);
        self
    }

    /// Add a composite definition.
    #[must_use]
    pub fn composite(mut self, definition: CompositeDefinition) -> Self {
        self.composites.push(definition);
        self
    }

    /// Add a domain definition.
    #[must_use]
    pub fn domain(mut self, definition: DomainDefinition) -> Self {
        self.domains.push(definition);
        self
    }

    /// Validate all definitions and construct the catalog.
    pub fn build(self) -> Result<Catalog, ScytheError> {
        let mut catalog = Catalog {
            tables: AHashMap::new(),
            enums: AHashMap::new(),
            composites: AHashMap::new(),
            domains: AHashMap::new(),
            dialect: self.dialect,
            engine: normalize_optional_engine(self.engine)?,
            relation_names: AHashMap::new(),
            relation_kinds: AHashMap::new(),
            raw_column_types: AHashMap::new(),
            enum_names: AHashMap::new(),
            composite_names: AHashMap::new(),
            domain_names: AHashMap::new(),
            raw_domain_types: AHashMap::new(),
        };

        add_relations(&mut catalog, self.relations)?;
        add_enums(&mut catalog, self.enums)?;
        add_composites(&mut catalog, self.composites)?;
        add_domains(&mut catalog, self.domains)?;
        Ok(catalog)
    }
}

fn add_relations(catalog: &mut Catalog, definitions: Vec<RelationDefinition>) -> Result<(), ScytheError> {
    for definition in definitions {
        let key = definition.name.normalized_key("relation")?;
        if catalog.tables.contains_key(&key) {
            return Err(duplicate_error("relation", &key));
        }

        let mut column_names = AHashSet::new();
        let mut columns = Vec::with_capacity(definition.columns.len());
        let mut raw_types = AHashMap::new();
        for column in definition.columns {
            let column_key = normalized_identifier(&column.name, "column")?;
            if !column_names.insert(column_key.clone()) {
                return Err(duplicate_error("column", &format!("{key}.{column_key}")));
            }
            let sql_type = normalized_sql_type(&column.raw_sql_type, "column type")?;
            raw_types.insert(column_key, column.raw_sql_type);
            columns.push(Column {
                name: column.name,
                sql_type,
                nullable: column.nullable,
                default: column.default,
                primary_key: column.primary_key,
            });
        }

        catalog.tables.insert(
            key.clone(),
            Table {
                columns,
                raw_name: definition.name.name.clone(),
            },
        );
        catalog.relation_names.insert(key.clone(), definition.name);
        catalog.relation_kinds.insert(key.clone(), definition.kind);
        catalog.raw_column_types.insert(key, raw_types);
    }
    Ok(())
}

fn add_enums(catalog: &mut Catalog, definitions: Vec<EnumDefinition>) -> Result<(), ScytheError> {
    for definition in definitions {
        let key = definition.name.normalized_key("enum")?;
        if catalog.enums.contains_key(&key) {
            return Err(duplicate_error("enum", &key));
        }
        catalog.enums.insert(
            key.clone(),
            EnumType {
                values: definition.values,
            },
        );
        catalog.enum_names.insert(key, definition.name);
    }
    Ok(())
}

fn add_composites(catalog: &mut Catalog, definitions: Vec<CompositeDefinition>) -> Result<(), ScytheError> {
    for definition in definitions {
        let key = definition.name.normalized_key("composite")?;
        if catalog.composites.contains_key(&key) || catalog.enums.contains_key(&key) {
            return Err(duplicate_error("type", &key));
        }
        let mut field_names = AHashSet::new();
        let mut fields = Vec::with_capacity(definition.fields.len());
        for field in definition.fields {
            let field_key = normalized_identifier(&field.name, "composite field")?;
            if !field_names.insert(field_key.clone()) {
                return Err(duplicate_error("composite field", &format!("{key}.{field_key}")));
            }
            fields.push(CompositeField {
                name: field.name,
                sql_type: normalized_sql_type(&field.raw_sql_type, "composite field type")?,
            });
        }
        catalog.composites.insert(key.clone(), CompositeType { fields });
        catalog.composite_names.insert(key, definition.name);
    }
    Ok(())
}

fn add_domains(catalog: &mut Catalog, definitions: Vec<DomainDefinition>) -> Result<(), ScytheError> {
    for definition in definitions {
        let key = definition.name.normalized_key("domain")?;
        if catalog.domains.contains_key(&key)
            || catalog.enums.contains_key(&key)
            || catalog.composites.contains_key(&key)
        {
            return Err(duplicate_error("type", &key));
        }
        let base_type = normalized_sql_type(&definition.raw_base_type, "domain base type")?;
        catalog.domains.insert(
            key.clone(),
            DomainDef {
                base_type,
                not_null: definition.not_null,
            },
        );
        catalog.domain_names.insert(key.clone(), definition.name);
        catalog.raw_domain_types.insert(key, definition.raw_base_type);
    }
    Ok(())
}

fn normalize_optional_engine(engine: Option<String>) -> Result<Option<String>, ScytheError> {
    engine.map(|value| normalized_identifier(&value, "engine")).transpose()
}

fn normalized_identifier(value: &str, description: &str) -> Result<String, ScytheError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ScytheError::invalid_config(format!(
            "invalid catalog: {description} must not be empty"
        )));
    }
    Ok(trimmed.to_lowercase())
}

fn normalized_sql_type(value: &str, description: &str) -> Result<String, ScytheError> {
    normalized_identifier(value, description)
}

fn duplicate_error(object_kind: &str, key: &str) -> ScytheError {
    ScytheError::invalid_config(format!("invalid catalog: duplicate {object_kind} `{key}`"))
}

#[cfg(test)]
mod tests {
    use crate::catalog::{
        Catalog, CatalogBuilder, CatalogObjectName, ColumnDefinition, CompositeDefinition, CompositeFieldDefinition,
        DomainDefinition, EnumDefinition, RelationDefinition, RelationKind,
    };
    use crate::dialect::SqlDialect;

    fn id_column() -> ColumnDefinition {
        ColumnDefinition::new("id", "INTEGER", false).primary_key()
    }

    #[test]
    fn should_reject_relations_that_normalize_to_the_same_name() {
        let error = CatalogBuilder::new(SqlDialect::PostgreSQL)
            .relation(RelationDefinition::table(
                CatalogObjectName::qualified("Public", "Users"),
                vec![id_column()],
            ))
            .relation(RelationDefinition::view(
                CatalogObjectName::qualified("public", "users"),
                vec![id_column()],
            ))
            .build()
            .expect_err("normalized duplicate relations must fail");

        assert!(error.to_string().contains("public.users"));
        assert!(error.to_string().contains("duplicate relation"));
    }

    #[test]
    fn should_preserve_relation_metadata_raw_types_and_defaults() {
        let catalog = CatalogBuilder::new(SqlDialect::SQLite)
            .engine("sqlite")
            .relation(RelationDefinition::view(
                CatalogObjectName::qualified("Main", "UserRollup"),
                vec![
                    ColumnDefinition::new("UserId", "INTEGER", false)
                        .default("next_value()")
                        .primary_key(),
                ],
            ))
            .build()
            .expect("valid inspected catalog");

        let table = catalog.get_table("main.userrollup").expect("view relation");
        assert_eq!(table.raw_name, "UserRollup");
        assert_eq!(table.columns[0].sql_type, "integer");
        assert_eq!(table.columns[0].default.as_deref(), Some("next_value()"));
        assert_eq!(
            catalog.relation_name("main.userrollup"),
            Some(&CatalogObjectName::qualified("Main", "UserRollup"))
        );
        assert_eq!(catalog.relation_kind("main.userrollup"), Some(RelationKind::View));
        assert_eq!(
            catalog.column_raw_sql_type("main.userrollup", "UserId"),
            Some("INTEGER")
        );
    }

    #[test]
    fn should_match_an_equivalent_parser_built_catalog() {
        let parsed = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE users (id INTEGER PRIMARY KEY NOT NULL, name TEXT DEFAULT 'guest');"],
            &SqlDialect::PostgreSQL,
        )
        .expect("valid DDL");
        let built = CatalogBuilder::new(SqlDialect::PostgreSQL)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("users"),
                vec![
                    id_column(),
                    ColumnDefinition::new("name", "TEXT", true).default("'guest'"),
                ],
            ))
            .build()
            .expect("valid definition");

        assert_eq!(built.fingerprint(), parsed.fingerprint());
        let built_table = built.get_table("users").expect("built table");
        let parsed_table = parsed.get_table("users").expect("parsed table");
        assert_eq!(built_table.columns.len(), parsed_table.columns.len());
        for (built_column, parsed_column) in built_table.columns.iter().zip(&parsed_table.columns) {
            assert_eq!(built_column.name, parsed_column.name);
            assert_eq!(built_column.sql_type, parsed_column.sql_type);
            assert_eq!(built_column.nullable, parsed_column.nullable);
            assert_eq!(built_column.primary_key, parsed_column.primary_key);
            assert_eq!(built_column.default, parsed_column.default);
        }
    }

    #[test]
    fn should_fingerprint_equivalent_definitions_independent_of_insertion_order() {
        let first = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("alpha"),
                vec![id_column()],
            ))
            .relation(RelationDefinition::table(
                CatalogObjectName::new("beta"),
                vec![id_column()],
            ))
            .build()
            .expect("valid definitions");
        let second = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("beta"),
                vec![id_column()],
            ))
            .relation(RelationDefinition::table(
                CatalogObjectName::new("alpha"),
                vec![id_column()],
            ))
            .build()
            .expect("valid definitions");

        assert_eq!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn should_construct_all_definition_kinds() {
        let catalog = CatalogBuilder::new(SqlDialect::PostgreSQL)
            .enum_type(EnumDefinition::new(
                CatalogObjectName::qualified("Public", "Mood"),
                vec!["happy".to_string(), "sad".to_string()],
            ))
            .composite(CompositeDefinition::new(
                CatalogObjectName::qualified("Public", "Address"),
                vec![CompositeFieldDefinition::new("ZipCode", "VARCHAR(12)")],
            ))
            .domain(DomainDefinition::new(
                CatalogObjectName::qualified("Public", "PositiveId"),
                "BIGINT",
                true,
            ))
            .build()
            .expect("valid definitions");

        assert_eq!(catalog.get_enum("public.mood").expect("enum").values, ["happy", "sad"]);
        let composite = catalog.get_composite("public.address").expect("composite");
        assert_eq!(composite.fields[0].name, "ZipCode");
        assert_eq!(composite.fields[0].sql_type, "varchar(12)");
        assert_eq!(catalog.get_domain_base_type("public.positiveid"), Some("bigint"));
        assert_eq!(catalog.domain_raw_base_type("public.positiveid"), Some("BIGINT"));
        assert_eq!(
            catalog.enum_name("public.mood"),
            Some(&CatalogObjectName::qualified("Public", "Mood"))
        );
        assert_eq!(
            catalog.composite_name("public.address"),
            Some(&CatalogObjectName::qualified("Public", "Address"))
        );
        assert_eq!(
            catalog.domain_name("public.positiveid"),
            Some(&CatalogObjectName::qualified("Public", "PositiveId"))
        );
    }

    #[test]
    fn should_reject_duplicate_columns_after_normalization() {
        let error = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("users"),
                vec![
                    ColumnDefinition::new("UserId", "INTEGER", false),
                    ColumnDefinition::new("userid", "TEXT", true),
                ],
            ))
            .build()
            .expect_err("duplicate columns must fail");

        assert!(error.to_string().contains("duplicate column `users.userid`"));
    }

    #[test]
    fn should_reject_duplicate_types_after_normalization() {
        let error = CatalogBuilder::new(SqlDialect::PostgreSQL)
            .enum_type(EnumDefinition::new(
                CatalogObjectName::qualified("Public", "Status"),
                vec!["active".to_string()],
            ))
            .domain(DomainDefinition::new(
                CatalogObjectName::qualified("public", "status"),
                "TEXT",
                false,
            ))
            .build()
            .expect_err("duplicate type names must fail");

        assert!(error.to_string().contains("duplicate type `public.status`"));
    }

    #[test]
    fn should_change_fingerprint_when_semantic_definition_changes() {
        let integer_catalog = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("users"),
                vec![ColumnDefinition::new("id", "INTEGER", false)],
            ))
            .build()
            .expect("valid catalog");
        let text_catalog = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("users"),
                vec![ColumnDefinition::new("id", "TEXT", false)],
            ))
            .build()
            .expect("valid catalog");

        assert_ne!(integer_catalog.fingerprint(), text_catalog.fingerprint());
    }
}
