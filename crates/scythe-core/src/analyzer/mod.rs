mod expressions;
mod helpers;
mod params;
mod scope;
mod statements;
mod type_conversion;
mod types;

pub use types::{
    AnalyzedColumn, AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, EnumInfo,
};

use ahash::{AHashMap, AHashSet};

use crate::catalog::Catalog;
use crate::errors::ScytheError;
use crate::parser::Query;

use helpers::detect_select_star_source;
use type_conversion::sql_type_to_neutral;
use types::Analyzer;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn analyze(catalog: &Catalog, query: &Query) -> Result<AnalyzedQuery, ScytheError> {
    let mut analyzer = Analyzer {
        catalog,
        params: Vec::new(),
        ctes: AHashMap::new(),
        type_errors: Vec::new(),
        positional_param_counter: 0,
    };

    let (columns, _) = analyzer.analyze_statement(&query.stmt)?;

    // Check for type errors collected during analysis
    if let Some(err_msg) = analyzer.type_errors.first() {
        return Err(ScytheError::type_mismatch(err_msg.clone()));
    }

    // Apply annotation overrides
    let mut columns = columns;
    for col in &mut columns {
        if query
            .annotations
            .nullable_overrides
            .iter()
            .any(|o| o == &col.name)
        {
            col.nullable = true;
        }
        if query
            .annotations
            .nonnull_overrides
            .iter()
            .any(|o| o == &col.name)
        {
            col.nullable = false;
        }
        // Apply @json type mappings
        if let Some(mapping) = query
            .annotations
            .json_mappings
            .iter()
            .find(|m| m.column == col.name)
        {
            col.neutral_type = format!("json_typed<{}>", mapping.rust_type);
        }
    }

    // Deduplicate and sort params by position
    analyzer.params.sort_by_key(|p| p.position);
    analyzer.params.dedup_by_key(|p| p.position);

    let mut params: Vec<AnalyzedParam> = analyzer
        .params
        .iter()
        .map(|p| {
            let name = p.name.clone().unwrap_or_else(|| format!("p{}", p.position));
            let neutral_type = p
                .neutral_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            AnalyzedParam {
                name,
                neutral_type,
                nullable: p.nullable,
                position: p.position,
            }
        })
        .collect();

    // Disambiguate duplicate param names by appending _N suffix
    {
        let mut name_counts: ahash::AHashMap<String, usize> = ahash::AHashMap::new();
        for p in &params {
            *name_counts.entry(p.name.clone()).or_insert(0) += 1;
        }
        let mut name_seen: ahash::AHashMap<String, usize> = ahash::AHashMap::new();
        for p in &mut params {
            if name_counts.get(&p.name).copied().unwrap_or(0) > 1 {
                let idx = name_seen.entry(p.name.clone()).or_insert(0);
                *idx += 1;
                p.name = format!("{}_{}", p.name, idx);
            }
        }
    }

    // Detect SELECT * from single table for model struct reuse
    let source_table = detect_select_star_source(&query.stmt);

    // Collect composite type definitions needed
    let mut composites = Vec::new();
    let mut seen_composites: AHashSet<String> = AHashSet::new();
    for col in &columns {
        if let Some(comp_name) = col.neutral_type.strip_prefix("composite::")
            && seen_composites.insert(comp_name.to_string())
            && let Some(comp) = catalog.get_composite(comp_name)
        {
            composites.push(CompositeInfo {
                sql_name: comp_name.to_string(),
                fields: comp
                    .fields
                    .iter()
                    .map(|f| CompositeFieldInfo {
                        name: f.name.clone(),
                        neutral_type: sql_type_to_neutral(&f.sql_type, catalog).into_owned(),
                    })
                    .collect(),
            });
        }
    }

    // Collect enum type definitions needed
    let mut enums = Vec::new();
    let mut seen_enums: AHashSet<String> = AHashSet::new();
    let all_types: Vec<&str> = columns
        .iter()
        .map(|c| c.neutral_type.as_str())
        .chain(params.iter().map(|p| p.neutral_type.as_str()))
        .collect();
    for nt in &all_types {
        if let Some(enum_name) = nt.strip_prefix("enum::")
            && seen_enums.insert(enum_name.to_string())
            && let Some(enum_type) = catalog.get_enum(enum_name)
        {
            enums.push(EnumInfo {
                sql_name: enum_name.to_string(),
                values: enum_type.values.clone(),
            });
        }
    }

    Ok(AnalyzedQuery {
        name: query.name.clone(),
        command: query.command.clone(),
        sql: query.sql.clone(),
        columns,
        params,
        deprecated: query.annotations.deprecated.clone(),
        source_table,
        composites,
        enums,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_query;

    fn make_catalog() -> Catalog {
        Catalog::from_ddl(&[
            "CREATE TABLE users (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                email VARCHAR(255) NOT NULL,
                age INTEGER,
                active BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
                bio TEXT,
                score NUMERIC
            );",
            "CREATE TABLE posts (
                id SERIAL PRIMARY KEY,
                user_id INTEGER NOT NULL REFERENCES users(id),
                title TEXT NOT NULL,
                body TEXT,
                published BOOLEAN NOT NULL DEFAULT false,
                created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
            );",
            "CREATE TABLE comments (
                id SERIAL PRIMARY KEY,
                post_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                body TEXT NOT NULL,
                created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
            );",
        ])
        .unwrap()
    }

    #[test]
    fn test_simple_select() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUser
-- @returns :one
SELECT id, name, email FROM users WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 3);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[0].neutral_type, "int32");
        assert!(!result.columns[0].nullable);
        assert_eq!(result.columns[1].name, "name");
        assert_eq!(result.columns[1].neutral_type, "string");
        assert_eq!(result.columns[2].name, "email");
        assert_eq!(result.columns[2].neutral_type, "string");

        assert_eq!(result.params.len(), 1);
        assert_eq!(result.params[0].position, 1);
        assert_eq!(result.params[0].neutral_type, "int32");
        assert_eq!(result.params[0].name, "id");
    }

    #[test]
    fn test_select_star() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name ListUsers
-- @returns :many
SELECT * FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 8);
    }

    #[test]
    fn test_left_join_nullability() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name UsersWithPosts
-- @returns :many
SELECT u.id, u.name, p.title, p.body FROM users u LEFT JOIN posts p ON u.id = p.user_id;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 4);
        assert!(!result.columns[0].nullable);
        assert!(!result.columns[1].nullable);
        assert!(result.columns[2].nullable);
        assert!(result.columns[3].nullable);
    }

    #[test]
    fn test_aggregate_functions() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name UserStats
-- @returns :one
SELECT COUNT(*) as total, AVG(age) as avg_age, MAX(score) as max_score FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 3);
        assert_eq!(result.columns[0].neutral_type, "int64");
        assert!(!result.columns[0].nullable);
        assert_eq!(result.columns[1].neutral_type, "decimal");
        assert!(result.columns[1].nullable);
        assert!(result.columns[2].nullable);
    }

    #[test]
    fn test_insert_returning() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name CreateUser
-- @returns :one
INSERT INTO users (name, email) VALUES ($1, $2) RETURNING id, name, email;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns.len(), 3);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[0].neutral_type, "int32");

        assert_eq!(result.params.len(), 2);
        assert_eq!(result.params[0].name, "name");
        assert_eq!(result.params[0].neutral_type, "string");
        assert_eq!(result.params[1].name, "email");
        assert_eq!(result.params[1].neutral_type, "string");
    }

    #[test]
    fn test_coalesce_nullability() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetBio
-- @returns :one
SELECT COALESCE(bio, 'No bio') as bio FROM users WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[0].neutral_type, "string");
        assert!(!result.columns[0].nullable);
    }

    #[test]
    fn test_case_expression() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetStatus
-- @returns :many
SELECT name, CASE WHEN active THEN 'active' ELSE 'inactive' END as status FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[1].name, "status");
        assert_eq!(result.columns[1].neutral_type, "string");
        assert!(!result.columns[1].nullable);
    }

    #[test]
    fn test_nullif() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetScore
-- @returns :many
SELECT NULLIF(score, 0) as adjusted_score FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[0].neutral_type, "decimal");
        assert!(result.columns[0].nullable);
    }

    #[test]
    fn test_cast_expression() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetAgeText
-- @returns :many
SELECT CAST(age AS TEXT) as age_text FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.columns[0].neutral_type, "string");
    }

    #[test]
    fn test_annotation_overrides() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUser
-- @returns :one
-- @nullable name
-- @nonnull age
SELECT name, age FROM users WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert!(result.columns[0].nullable);
        assert!(!result.columns[1].nullable);
    }
}
