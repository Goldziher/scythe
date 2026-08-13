mod expressions;
mod helpers;
mod naming;
mod params;
mod query_fingerprint;
mod scope;
mod statements;
mod type_conversion;
mod types;

pub use type_conversion::sql_type_to_neutral;
pub use types::{
    AnalyzedColumn, AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, EnumInfo, GroupByConfig,
    NestedFieldInfo, NestedStructInfo,
};

use ahash::{AHashMap, AHashSet};

use crate::catalog::Catalog;
use crate::dialect::SqlDialect;
use crate::errors::ScytheError;
use crate::parser::{Query, QueryCommand};

use helpers::{detect_select_star_source, disambiguate_duplicate_names, find_nested_placeholder_id};
use types::Analyzer;

pub fn analyze(catalog: &Catalog, query: &Query) -> Result<AnalyzedQuery, ScytheError> {
    let mut analyzer = Analyzer {
        catalog,
        params: Vec::new(),
        ctes: AHashMap::new(),
        type_errors: Vec::new(),
        positional_param_counter: 0,
        pending_nested: Vec::new(),
        next_nested_id: 0,
        resolved_placeholders: AHashMap::new(),
    };

    let (columns, _) = analyzer.analyze_statement(&query.stmt)?;

    if let Some(err_msg) = analyzer.type_errors.first() {
        return Err(ScytheError::type_mismatch(err_msg.clone()));
    }

    let mut columns = columns;
    for col in &mut columns {
        if query.annotations.nullable_overrides.iter().any(|o| o == &col.name) {
            col.nullable = true;
        }
        if query.annotations.nonnull_overrides.iter().any(|o| o == &col.name) {
            col.nullable = false;
        }
        if let Some(mapping) = query.annotations.json_mappings.iter().find(|m| m.column == col.name) {
            col.neutral_type = format!("json_typed<{}>", mapping.rust_type);
        }
    }

    // ~keep Phase 2 of nested-struct naming (see `types::PendingNestedStruct`):
    // columns now have their final names (aliases and overrides applied),
    // so each `__nested__{id}` placeholder pushed during expression
    // inference (phase 1) can be resolved to a real struct name.
    let nested_structs = resolve_nested_struct_names(
        catalog,
        &query.name,
        std::mem::take(&mut analyzer.pending_nested),
        &mut columns,
    );

    analyzer.params.sort_by_key(|p| p.position);
    analyzer.params.dedup_by_key(|p| p.position);

    let mut params: Vec<AnalyzedParam> = analyzer
        .params
        .iter()
        .map(|p| {
            // Apply explicit @param $N name override first; fall back to the
            let name = query
                .annotations
                .positional_param_docs
                .iter()
                .find(|doc| doc.position == p.position)
                .map(|doc| doc.name.clone())
                .unwrap_or_else(|| p.name.clone().unwrap_or_else(|| format!("p{}", p.position)));
            let neutral_type = p.neutral_type.clone().unwrap_or_else(|| "unknown".to_string());
            AnalyzedParam {
                name,
                neutral_type,
                nullable: p.nullable,
                position: p.position,
            }
        })
        .collect();

    for opt_name in &query.annotations.optional_params {
        for p in &mut params {
            if p.name == *opt_name {
                p.nullable = true;
            }
        }
    }

    for opt_name in &query.annotations.optional_params {
        if !params.iter().any(|p| p.name == *opt_name) {
            return Err(ScytheError::invalid_annotation(format!(
                "@optional references unknown parameter '{}'",
                opt_name
            )));
        }
    }

    // Shared with the result-column path in `analyze_select` /
    // `analyze_returning` -- one rule for every identifier scythe generates
    // from a user-supplied name (#175).
    disambiguate_duplicate_names(params.iter_mut().map(|p| &mut p.name));

    let source_table = detect_select_star_source(&query.stmt);

    // ~keep Every neutral type that a generated file must be able to name. Nested
    // struct fields belong here alongside the top-level columns and params:
    // a `json_agg(o.*)` over a table with an enum or composite column puts
    // that type in the *nested* struct's field list and nowhere else, so
    // scanning only `columns`/`params` emits `pub status: OrderStatus` with
    // no `enum OrderStatus` in the file — E0412 in Rust, an undefined type
    // in Go, a `NameError` in Python.
    let nested_field_types: Vec<&str> = nested_structs
        .iter()
        .flat_map(|nested| nested.fields.iter())
        .map(|field| field.neutral_type.as_str())
        .collect();

    let mut composites = Vec::new();
    let mut seen_composites: AHashSet<String> = AHashSet::new();
    for neutral_type in columns
        .iter()
        .map(|c| c.neutral_type.as_str())
        .chain(nested_field_types.iter().copied())
    {
        if let Some(comp_name) = neutral_type.strip_prefix("composite::")
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

    let mut enums = Vec::new();
    let mut seen_enums: AHashSet<String> = AHashSet::new();
    let all_types: Vec<&str> = columns
        .iter()
        .map(|c| c.neutral_type.as_str())
        .chain(params.iter().map(|p| p.neutral_type.as_str()))
        .chain(nested_field_types.iter().copied())
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

    let group_by = if query.command == QueryCommand::Grouped {
        if let Some(ref group_by_value) = query.annotations.group_by {
            let (table, key_column) = if let Some(dot_pos) = group_by_value.find('.') {
                (
                    group_by_value[..dot_pos].to_string(),
                    group_by_value[dot_pos + 1..].to_string(),
                )
            } else {
                return Err(ScytheError::invalid_annotation(format!(
                    "@group_by must be in 'table.column' format, got: {}",
                    group_by_value
                )));
            };

            let parent_table_columns: Vec<String> = catalog
                .get_table(&table)
                .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default();

            let mut parent_columns = Vec::new();
            let mut child_columns = Vec::new();

            for col in &columns {
                if parent_table_columns.contains(&col.name) {
                    parent_columns.push(col.clone());
                } else {
                    child_columns.push(col.clone());
                }
            }

            // This split matches projected column names against the parent
            // table's catalog columns, so anything that stops a projected
            // name from being the catalog name -- an alias in `@group_by`
            // (`@group_by u.id`, where `u` is not a table), a table that does
            // not exist, or a name the duplicate-column pass had to suffix
            // (`u.id`/`o.id` -> `id_1`/`id_2`, see
            // `disambiguate_duplicate_names`) -- silently sends every column
            // to the child struct and leaves the parent struct empty. That
            // generates a parent row type with no fields at exit 0, so it is
            // reported here instead.
            if parent_columns.is_empty() {
                return Err(ScytheError::invalid_annotation(format!(
                    "@group_by {group_by_value} matched none of the query's result columns, so the \
                     parent row type would have no fields -- name the table as it appears in the \
                     schema (not a query alias), and make sure the projection selects its columns \
                     under their own names"
                )));
            }

            Some(types::GroupByConfig {
                table,
                key_column,
                parent_columns,
                child_columns,
            })
        } else {
            None
        }
    } else {
        None
    };

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
        optional_params: query.annotations.optional_params.clone(),
        group_by,
        custom: query.annotations.custom.clone(),
        nested_structs,
    })
}

/// Phase 2 of nested-struct naming. Walks `columns` for `__nested__{id}`
/// placeholders left by phase-1 expression inference, assigns each a final
/// name (deduping identical shapes, suffixing collisions against the
/// catalog or against a differently-shaped struct), substitutes the
/// placeholder with the resulting PascalCase name in place, and returns the
/// resolved [`NestedStructInfo`] list for `AnalyzedQuery::nested_structs`.
///
/// A non-PostgreSQL dialect returns an empty list and leaves `columns`
/// untouched. In practice `pending` is only ever non-empty for a catalog
/// that passed the fuller gate in `Analyzer::infer_nested_aggregate_type`
/// (the phase-1 producer, `expressions.rs`), which also excludes engines
/// like Redshift that map onto the PostgreSQL dialect without having
/// `json_agg` — the dialect check here is a second, independent net so this
/// pass cannot leave a half-substituted placeholder behind if that
/// invariant is ever violated. It is deliberately the weaker of the two:
/// its job is to never *partially* resolve, not to re-derive the engine
/// policy.
fn resolve_nested_struct_names(
    catalog: &Catalog,
    query_name: &str,
    pending: Vec<types::PendingNestedStruct>,
    columns: &mut [AnalyzedColumn],
) -> Vec<NestedStructInfo> {
    if pending.is_empty() || catalog.dialect() != SqlDialect::PostgreSQL {
        return Vec::new();
    }

    let snake_query = naming::to_snake_case(query_name).into_owned();
    let mut resolved: AHashMap<u32, String> = AHashMap::new();
    let mut structs: Vec<NestedStructInfo> = Vec::new();

    for column in columns.iter_mut() {
        while let Some(id) = find_nested_placeholder_id(&column.neutral_type) {
            let final_name = if let Some(name) = resolved.get(&id) {
                name.clone()
            } else {
                let fields = pending
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.fields.clone())
                    .unwrap_or_default();
                let base = format!("{snake_query}_row_{}", column.name);
                let name = assign_nested_struct_name(&base, fields, catalog, &mut structs);
                resolved.insert(id, name.clone());
                name
            };

            let pascal = naming::to_pascal_case(&final_name);
            column.neutral_type = column.neutral_type.replacen(&format!("__nested__{id}"), &pascal, 1);
        }
    }

    structs
}

/// Find a free name for a nested struct, starting from `base` (already
/// snake_case) and trying `{base}_1`, `{base}_2`, ... in order.
///
/// Two independent collision sources are checked per candidate: a catalog
/// composite or enum sharing the name always forces the next suffix; a
/// same-named struct already assigned earlier in this `analyze()` call is
/// reused as-is when its field shape is identical (two output columns can
/// legitimately produce the same nested shape) and otherwise also forces
/// the next suffix.
fn assign_nested_struct_name(
    base: &str,
    fields: Vec<NestedFieldInfo>,
    catalog: &Catalog,
    structs: &mut Vec<NestedStructInfo>,
) -> String {
    let mut suffix: u32 = 0;
    loop {
        let candidate = if suffix == 0 {
            base.to_string()
        } else {
            format!("{base}_{suffix}")
        };

        if let Some(existing) = structs.iter().find(|s| s.name == candidate) {
            if existing.fields == fields {
                return candidate;
            }
            suffix += 1;
            continue;
        }

        if catalog.get_composite(&candidate).is_none() && catalog.get_enum(&candidate).is_none() {
            structs.push(NestedStructInfo {
                name: candidate.clone(),
                fields,
            });
            return candidate;
        }

        suffix += 1;
    }
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

    /// `INSERT INTO t VALUES (...)` with no column list binds positionally to
    /// every column of the table in catalog order. Placeholders must be typed
    /// and named from that ordering, not dropped (regression: F2 — params in a
    /// column-list-less INSERT were silently lost).
    #[test]
    fn test_insert_without_column_list_binds_to_catalog_columns() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name CreateUserFull
-- @returns :exec
INSERT INTO users VALUES ($1, $2, $3, $4, $5, $6, $7, $8);",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 8, "all 8 placeholders must be registered");
        let names: Vec<&str> = result.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["id", "name", "email", "age", "active", "created_at", "bio", "score"]
        );
        assert_eq!(result.params[0].neutral_type, "int32");
        assert_eq!(result.params[1].neutral_type, "string");
        assert_eq!(result.params[3].neutral_type, "int32");
        assert_eq!(result.params[4].neutral_type, "bool");
        assert_eq!(result.params[5].neutral_type, "datetime_tz");
        assert_eq!(result.params[7].neutral_type, "decimal");
        assert!(!result.params[1].nullable, "name is NOT NULL");
        assert!(result.params[3].nullable, "age is nullable");
        assert!(result.params[6].nullable, "bio is nullable");
    }

    /// When neither a column list nor a catalog entry is available, params in a
    /// column-list-less INSERT must still be registered with an inferred type
    /// rather than silently dropped.
    #[test]
    fn test_insert_without_column_list_unknown_table_registers_inferred_params() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name InsertNoSchema
-- @returns :exec
INSERT INTO t VALUES ($1, $2::text);",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 2, "$1 and $2 must both be registered");
        assert_eq!(result.params[0].position, 1);
        assert_eq!(result.params[0].name, "p1");
        assert_eq!(result.params[1].position, 2);
        assert_eq!(result.params[1].neutral_type, "string", "$2::text must infer as string");
    }

    /// An explicit column list still names params even when the table has no
    /// catalog entry — only the type falls back to inference.
    #[test]
    fn test_insert_explicit_columns_unknown_table_keeps_declared_names() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name InsertNoSchemaCols
-- @returns :exec
INSERT INTO t (a, b) VALUES ($1, $2);",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 2);
        assert_eq!(result.params[0].name, "a");
        assert_eq!(result.params[1].name, "b");
    }

    /// Placeholders nested inside function calls in INSERT VALUES must be
    /// collected (regression: F10 — the `_ => {}` arm swallowed them).
    #[test]
    fn test_insert_coalesce_param_collected() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name InsertBio
-- @returns :exec
INSERT INTO users (bio) VALUES (COALESCE($1, 'unknown'));",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 1, "$1 inside COALESCE must be registered");
        assert_eq!(result.params[0].name, "bio");
        assert_eq!(result.params[0].neutral_type, "string");
        assert!(result.params[0].nullable, "bio is nullable");
    }

    /// Placeholders inside CASE branches of INSERT VALUES must be collected.
    #[test]
    fn test_insert_case_param_collected() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name InsertName
-- @returns :exec
INSERT INTO users (name) VALUES (CASE WHEN $1 THEN 'x' ELSE 'y' END);",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 1, "$1 inside CASE must be registered");
        assert_eq!(result.params[0].name, "name");
        assert_eq!(result.params[0].neutral_type, "string");
    }

    /// Placeholders nested in function calls in UPDATE SET must be collected.
    #[test]
    fn test_update_function_arg_param_collected() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name RenameUser
-- @returns :exec
UPDATE users SET name = LOWER(CONCAT($1, '_suffix')) WHERE id = $2;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 2, "$1 in nested function args must be registered");
        assert_eq!(result.params[0].name, "name");
        assert_eq!(result.params[0].neutral_type, "string");
        assert_eq!(result.params[1].name, "id");
        assert_eq!(result.params[1].neutral_type, "int32");
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
    fn test_param_inside_derived_table_propagates() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name BucketCounts
-- @returns :many
SELECT b.bucket, count(*) AS n
FROM posts p
CROSS JOIN (SELECT $1::text AS bucket) b
GROUP BY 1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(
            result.params.len(),
            1,
            "$1 inside derived table must appear in analyzed.params; got {:?}",
            result.params
        );
        assert_eq!(result.params[0].position, 1);
        assert_eq!(result.params[0].neutral_type, "string");
    }

    // Task 3: @param $N name overrides the fallback pN name

    #[test]
    fn test_positional_param_name_override() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUser
-- @returns :one
-- @param $1 user_id: the primary key
SELECT id, name FROM users WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 1);
        assert_eq!(result.params[0].name, "user_id");
        assert_eq!(result.params[0].position, 1);
    }

    #[test]
    fn test_positional_param_override_does_not_affect_unrelated_params() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name UpdateUser
-- @returns :exec
-- @param $2 target_id
UPDATE users SET name = $1 WHERE id = $2;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 2);
        assert_eq!(result.params[0].name, "name");
        assert_eq!(result.params[1].name, "target_id");
    }

    #[test]
    fn test_update_set_arithmetic_expr_collects_all_params() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name IncrementUserAge
-- @returns :exec
UPDATE users SET age = age + $2 WHERE id = $1;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 2, "both $1 and $2 must be present");
        let positions: Vec<i64> = result.params.iter().map(|p| p.position).collect();
        assert!(positions.contains(&1), "missing $1; got {positions:?}");
        assert!(positions.contains(&2), "missing $2; got {positions:?}");
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

    // -----------------------------------------------------------------
    // Phase-2 nested-struct naming (`resolve_nested_struct_names`).
    //
    // ~keep `Analyzer::infer_nested_aggregate_type` (expressions.rs) is the real
    // producer now, exercised end to end by the json_agg/row_to_json tests
    // further down. These tests instead exercise the resolver directly
    // with a hand-built `pending` list and column set, to pin name
    // derivation, collision handling, and the dialect gate in isolation
    // from expression inference.
    // -----------------------------------------------------------------

    fn nested_column(name: &str, neutral_type: &str) -> AnalyzedColumn {
        AnalyzedColumn {
            name: name.to_string(),
            neutral_type: neutral_type.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_resolve_nested_struct_names_derives_name_from_query_and_column() {
        let catalog = make_catalog();
        let pending = vec![types::PendingNestedStruct {
            id: 0,
            fields: vec![NestedFieldInfo {
                name: "title".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            }],
        }];
        let mut columns = vec![nested_column("orders", "json_nested<array<__nested__0>>")];

        let structs = resolve_nested_struct_names(&catalog, "GetUserOrders", pending, &mut columns);

        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "get_user_orders_row_orders");
        assert_eq!(structs[0].fields.len(), 1);
        assert_eq!(structs[0].fields[0].name, "title");
    }

    /// The resolver only ever performs `to_pascal_case(name)`, never a
    /// round trip back through `to_snake_case` — pin that the PascalCase
    /// form embedded in the neutral type matches `to_pascal_case` applied
    /// to the returned `NestedStructInfo.name` exactly.
    #[test]
    fn test_resolve_nested_struct_names_pascal_case_matches_neutral_type() {
        let catalog = make_catalog();
        let pending = vec![types::PendingNestedStruct {
            id: 3,
            fields: Vec::new(),
        }];
        let mut columns = vec![nested_column("orders", "json_nested<array<__nested__3>>")];

        let structs = resolve_nested_struct_names(&catalog, "GetUserOrders", pending, &mut columns);

        let expected_pascal = naming::to_pascal_case(&structs[0].name);
        assert_eq!(
            columns[0].neutral_type,
            format!("json_nested<array<{expected_pascal}>>")
        );
    }

    #[test]
    fn test_resolve_nested_struct_names_collision_with_catalog_composite_suffixes() {
        let catalog = Catalog::from_ddl(&["CREATE TYPE get_user_row_profile AS (x TEXT);"]).unwrap();
        let pending = vec![types::PendingNestedStruct {
            id: 0,
            fields: vec![NestedFieldInfo {
                name: "bio".to_string(),
                neutral_type: "string".to_string(),
                nullable: true,
            }],
        }];
        let mut columns = vec![nested_column("profile", "json_nested<__nested__0>")];

        let structs = resolve_nested_struct_names(&catalog, "GetUser", pending, &mut columns);

        assert_eq!(structs.len(), 1);
        assert_eq!(
            structs[0].name, "get_user_row_profile_1",
            "must suffix rather than collide with the catalog composite \"get_user_row_profile\""
        );
        assert_eq!(columns[0].neutral_type, "json_nested<GetUserRowProfile1>");
    }

    #[test]
    fn test_resolve_nested_struct_names_duplicate_column_names_dedupe_identical_shape() {
        let catalog = make_catalog();
        let shared_fields = vec![NestedFieldInfo {
            name: "id".to_string(),
            neutral_type: "int32".to_string(),
            nullable: false,
        }];
        let pending = vec![
            types::PendingNestedStruct {
                id: 0,
                fields: shared_fields.clone(),
            },
            types::PendingNestedStruct {
                id: 1,
                fields: shared_fields,
            },
        ];
        let mut columns = vec![
            nested_column("items", "json_nested<array<__nested__0>>"),
            nested_column("items", "json_nested<array<__nested__1>>"),
        ];

        let structs = resolve_nested_struct_names(&catalog, "GetOrder", pending, &mut columns);

        assert_eq!(
            structs.len(),
            1,
            "two columns with the same name and identical field shape must dedupe to one struct"
        );
        assert_eq!(columns[0].neutral_type, columns[1].neutral_type);
    }

    #[test]
    fn test_resolve_nested_struct_names_duplicate_column_names_differing_shape_suffixes() {
        let catalog = make_catalog();
        let pending = vec![
            types::PendingNestedStruct {
                id: 0,
                fields: vec![NestedFieldInfo {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                }],
            },
            types::PendingNestedStruct {
                id: 1,
                fields: vec![NestedFieldInfo {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                }],
            },
        ];
        let mut columns = vec![
            nested_column("items", "json_nested<array<__nested__0>>"),
            nested_column("items", "json_nested<array<__nested__1>>"),
        ];

        let structs = resolve_nested_struct_names(&catalog, "GetOrder", pending, &mut columns);

        assert_eq!(
            structs.len(),
            2,
            "same-named columns with different field shapes must not collapse into one struct"
        );
        assert_eq!(structs[0].name, "get_order_row_items");
        assert_eq!(structs[1].name, "get_order_row_items_1");
        assert_ne!(columns[0].neutral_type, columns[1].neutral_type);
    }

    #[test]
    fn test_resolve_nested_struct_names_mysql_dialect_produces_none() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE orders (id INTEGER NOT NULL);"], &SqlDialect::MySQL)
                .unwrap();
        let pending = vec![types::PendingNestedStruct {
            id: 0,
            fields: vec![NestedFieldInfo {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
            }],
        }];
        let original_neutral_type = "json_nested<array<__nested__0>>".to_string();
        let mut columns = vec![nested_column("orders", &original_neutral_type)];

        let structs = resolve_nested_struct_names(&catalog, "GetUserOrders", pending, &mut columns);

        assert!(
            structs.is_empty(),
            "non-PostgreSQL dialects must never produce nested_structs"
        );
        assert_eq!(
            columns[0].neutral_type, original_neutral_type,
            "the placeholder must be left untouched, not partially substituted"
        );
    }

    // -----------------------------------------------------------------
    // Phase-1 producer: json_agg / row_to_json nested-aggregate inference
    // end to end through analyze().
    // -----------------------------------------------------------------

    #[test]
    fn test_json_agg_wildcard_produces_nested_struct() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPosts
-- @returns :many
SELECT u.id, json_agg(p.*) AS posts FROM users u JOIN posts p ON u.id = p.user_id GROUP BY u.id;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[1].name, "posts");
        assert_eq!(
            result.nested_structs.len(),
            1,
            "exactly one nested struct must be produced"
        );

        let nested = &result.nested_structs[0];
        assert_eq!(nested.name, "get_user_posts_row_posts");
        assert_eq!(
            result.columns[1].neutral_type, "json_nested<array<GetUserPostsRowPosts>>",
            "json_agg wraps the resolved name in array<> and must match the PascalCase of nested.name exactly"
        );
        assert!(result.columns[1].nullable);

        let field_names: Vec<&str> = nested.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            field_names,
            ["id", "user_id", "title", "body", "published", "created_at"]
        );
        let title_field = nested.fields.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(title_field.neutral_type, "string");
        assert!(!title_field.nullable, "posts.title is NOT NULL and the join is INNER");
        let body_field = nested.fields.iter().find(|f| f.name == "body").unwrap();
        assert!(body_field.nullable, "posts.body has no NOT NULL constraint");
    }

    /// A LEFT JOIN moves *element* nullability, not field nullability. With
    /// no matching row PostgreSQL makes the whole-row variable `p` itself
    /// NULL, so `json_agg(p.*)` produces the JSON array `[null]` — a null
    /// element, never an object whose fields are all null. The element type
    /// must therefore be optional while each field keeps its schema
    /// nullability.
    #[test]
    fn test_json_agg_left_join_makes_array_elements_nullable() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostsOuter
-- @returns :many
SELECT u.id, json_agg(p.*) AS posts FROM users u LEFT JOIN posts p ON u.id = p.user_id GROUP BY u.id;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(
            result.columns[1].neutral_type, "json_nested<array<nullable<GetUserPostsOuterRowPosts>>>",
            "json_agg over a LEFT JOIN with no match yields [null], so the element type must be nullable"
        );

        let nested = &result.nested_structs[0];
        let id_field = nested.fields.iter().find(|f| f.name == "id").unwrap();
        assert!(
            !id_field.nullable,
            "posts.id is NOT NULL; inside an object json_agg actually emitted it can never be null, so \
             widening the field instead of the element would model a value PostgreSQL never produces"
        );
        let body_field = nested.fields.iter().find(|f| f.name == "body").unwrap();
        assert!(body_field.nullable, "posts.body has no NOT NULL constraint");
    }

    /// The INNER-JOIN counterpart: every aggregated row matched, so no
    /// element can be null and the `nullable<>` wrapper must be absent.
    #[test]
    fn test_json_agg_inner_join_elements_are_not_nullable() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostsInner
-- @returns :many
SELECT u.id, json_agg(p.*) AS posts FROM users u JOIN posts p ON u.id = p.user_id GROUP BY u.id;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(
            result.columns[1].neutral_type,
            "json_nested<array<GetUserPostsInnerRowPosts>>"
        );
    }

    /// `row_to_json` over a null-extended row returns SQL NULL, not a JSON
    /// null: the column is nullable and there is no element to wrap, so the
    /// `nullable<>` element wrapper must not appear on this path.
    #[test]
    fn test_row_to_json_left_join_does_not_wrap_element() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostJson
-- @returns :many
SELECT u.id, row_to_json(p.*) AS post FROM users u LEFT JOIN posts p ON u.id = p.user_id;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns[1].neutral_type, "json_nested<GetUserPostJsonRowPost>");
        assert!(result.columns[1].nullable, "the column itself carries the NULL");
    }

    #[test]
    fn test_row_to_json_wildcard_produces_nested_struct_without_array() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPostAsJson
-- @returns :many
SELECT row_to_json(p.*) AS post FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.nested_structs.len(), 1);
        assert_eq!(
            result.columns[0].neutral_type, "json_nested<GetPostAsJsonRowPost>",
            "row_to_json must not wrap in array<> -- it emits one object per output row, not an aggregate"
        );
    }

    #[test]
    fn test_json_agg_scalar_argument_falls_back_to_plain_json() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserNames
-- @returns :one
SELECT json_agg(u.name) AS names FROM users u;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(
            result.columns[0].neutral_type, "json",
            "a scalar/column argument is not a relation shape -- must match pre-existing json_agg behaviour exactly"
        );
        assert!(result.nested_structs.is_empty());
    }

    #[test]
    fn test_json_agg_bare_wildcard_falls_back_to_plain_json() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserCount
-- @returns :one
SELECT json_agg(*) AS everything FROM users;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns[0].neutral_type, "json");
        assert!(result.nested_structs.is_empty());
    }

    #[test]
    fn test_json_agg_non_postgres_dialect_falls_back_to_plain_json() {
        let catalog = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE posts (id INTEGER NOT NULL, title TEXT NOT NULL);"],
            &crate::dialect::SqlDialect::MySQL,
        )
        .unwrap();
        let query = parse_query(
            "-- @name GetPosts
-- @returns :many
SELECT json_agg(p.*) AS posts FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(
            result.columns[0].neutral_type, "json",
            "the dialect gate must produce byte-identical output to today's json_agg behaviour on non-PostgreSQL catalogs"
        );
        assert!(result.nested_structs.is_empty());
    }

    /// Guardrail: `string_agg` must never be reinterpreted as a nested
    /// aggregate, even when its argument is a relation wildcard shape (not
    /// valid SQL for `string_agg`, which expects a text expression, but the
    /// analyzer must not special-case it regardless).
    #[test]
    fn test_string_agg_never_produces_nested_struct() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserNamesJoined
-- @returns :one
SELECT string_agg(u.name, ',') AS names FROM users u;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns[0].neutral_type, "string");
        assert!(result.nested_structs.is_empty());
    }

    /// `jsonb_agg` differs from `json_agg` only in storage type — both
    /// aggregate one JSON object per row into a JSON array — so it must get
    /// the identical nested struct, array wrapper included.
    #[test]
    fn test_jsonb_agg_wildcard_produces_nested_struct() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostsB
-- @returns :many
SELECT jsonb_agg(p.*) AS posts FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.nested_structs.len(), 1);
        assert_eq!(
            result.columns[0].neutral_type,
            "json_nested<array<GetUserPostsBRowPosts>>"
        );
        assert!(result.columns[0].nullable, "an aggregate over zero rows is SQL NULL");

        let field_names: Vec<&str> = result.nested_structs[0]
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            field_names,
            ["id", "user_id", "title", "body", "published", "created_at"]
        );
    }

    /// The LEFT JOIN element-nullability rule is a property of the aggregate,
    /// not of the spelling: `jsonb_agg` over a null-extended row yields
    /// `[null]` exactly as `json_agg` does.
    #[test]
    fn test_jsonb_agg_left_join_makes_array_elements_nullable() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostsBOuter
-- @returns :many
SELECT u.id, jsonb_agg(p.*) AS posts FROM users u LEFT JOIN posts p ON u.id = p.user_id GROUP BY u.id;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(
            result.columns[1].neutral_type,
            "json_nested<array<nullable<GetUserPostsBOuterRowPosts>>>"
        );
    }

    /// `to_json(p.*)` is `row_to_json(p.*)` spelled differently — PostgreSQL
    /// returns the identical document — so it must produce the same single
    /// nested object, with no array wrapper.
    #[test]
    fn test_to_json_wildcard_produces_nested_struct_without_array() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPostAsJson2
-- @returns :many
SELECT to_json(p.*) AS post FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.nested_structs.len(), 1);
        assert_eq!(result.columns[0].neutral_type, "json_nested<GetPostAsJson2RowPost>");
    }

    #[test]
    fn test_to_jsonb_wildcard_produces_nested_struct_without_array() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPostAsJsonb
-- @returns :many
SELECT to_jsonb(p.*) AS post FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.nested_structs.len(), 1);
        assert_eq!(result.columns[0].neutral_type, "json_nested<GetPostAsJsonbRowPost>");
    }

    /// `to_json` is `proisstrict = t`: `to_json(NULL)` is SQL NULL, not the
    /// JSON document `null`, so a scalar conversion inherits its argument's
    /// nullability instead of being unconditionally non-null.
    #[test]
    fn test_to_json_scalar_follows_argument_nullability() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPostBodyJson
-- @returns :many
SELECT to_json(p.body) AS body_json, to_json(p.title) AS title_json FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns[0].neutral_type, "json");
        assert!(
            result.columns[0].nullable,
            "posts.body is nullable and to_json is strict"
        );
        assert_eq!(result.columns[1].neutral_type, "json");
        assert!(!result.columns[1].nullable, "posts.title is NOT NULL");
        assert!(result.nested_structs.is_empty());
    }

    /// Guardrail for the deliberate non-change: `json_object_agg(k, v)`
    /// builds a JSON object keyed by the *runtime values* of `k`, so it has
    /// no fixed field set and must stay a flat, nullable `json` even though
    /// its `json_agg` neighbour now infers a struct.
    #[test]
    fn test_json_object_agg_stays_plain_nullable_json() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPostTitles
-- @returns :one
SELECT json_object_agg(p.id, p.title) AS titles, jsonb_object_agg(p.id, p.title) AS titles_b FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns[0].neutral_type, "json");
        assert!(result.columns[0].nullable, "an aggregate over zero rows is SQL NULL");
        assert_eq!(result.columns[1].neutral_type, "json");
        assert!(result.columns[1].nullable);
        assert!(result.nested_structs.is_empty());
    }

    /// Guardrail for the other half of the split arm: the `json_build_*`
    /// family is `proisstrict = f` — `json_build_object('a', NULL)` is
    /// `{"a": null}`, a non-NULL document — so it stays non-nullable even
    /// with a nullable argument.
    #[test]
    fn test_json_build_object_stays_non_nullable_with_nullable_argument() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name BuildPostJson
-- @returns :many
SELECT json_build_object('body', p.body) AS built FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns[0].neutral_type, "json");
        assert!(
            !result.columns[0].nullable,
            "json_build_object embeds a JSON null rather than returning SQL NULL"
        );
    }

    /// Nested-of-nested: an outer `json_agg` over a CTE column that is
    /// itself the result of an inner `json_agg`. Phase 2 naming only walks
    /// the query's own top-level output columns, never recursively into a
    /// NestedStructInfo's own fields, so the inner `__nested__{id}`
    /// placeholder can never be substituted if this were allowed through --
    /// it must be rejected with a clear diagnostic instead of reaching
    /// codegen as a leaked, unresolvable placeholder.
    #[test]
    fn test_nested_of_nested_aggregate_is_rejected_with_clear_diagnostic() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetAllUserPosts
-- @returns :many
WITH user_posts AS (
    SELECT u.id AS user_id, json_agg(p.*) AS posts
    FROM users u JOIN posts p ON p.user_id = u.id
    GROUP BY u.id
)
SELECT up.user_id, json_agg(up.*) AS all_posts FROM user_posts up;",
        )
        .unwrap();

        let err = analyze(&catalog, &query).unwrap_err();

        assert!(
            err.message.contains("nested aggregate over nested aggregate"),
            "expected a clear nested-of-nested diagnostic, got: {}",
            err.message
        );
        assert!(
            err.message.contains("posts"),
            "diagnostic should name the offending field, got: {}",
            err.message
        );
    }

    /// UNION arm widening: two arms that both `json_agg` the *same*
    /// underlying table shape must widen cleanly into one nested struct,
    /// even though each arm's `json_agg` call independently allocated its
    /// own `__nested__{id}` and so never compares textually equal.
    #[test]
    fn test_union_arms_with_identical_nested_shape_widen_to_one_struct() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostsEitherWay
-- @returns :many
SELECT u.id, json_agg(p.*) AS posts FROM users u JOIN posts p ON p.user_id = u.id GROUP BY u.id
UNION
SELECT u2.id, json_agg(p2.*) AS posts FROM users u2 JOIN posts p2 ON p2.user_id = u2.id GROUP BY u2.id;",
        )
        .unwrap();

        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(
            result.nested_structs.len(),
            1,
            "both arms describe the same posts shape and must widen to a single struct"
        );
        assert!(result.columns[1].neutral_type.starts_with("json_nested<array<"));
    }

    /// UNION arm widening: two arms that `json_agg` genuinely different
    /// table shapes must be rejected with a clear diagnostic, not silently
    /// resolved to whichever arm happened to be on the left -- the pre-fix
    /// behaviour would drop the right arm's shape with no signal at all.
    #[test]
    fn test_union_arms_with_differing_nested_shape_is_rejected_with_clear_diagnostic() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostsOrComments
-- @returns :many
SELECT u.id, json_agg(p.*) AS posts FROM users u JOIN posts p ON p.user_id = u.id GROUP BY u.id
UNION
SELECT u2.id, json_agg(c.*) AS posts FROM users u2 JOIN comments c ON c.user_id = u2.id GROUP BY u2.id;",
        )
        .unwrap();

        let err = analyze(&catalog, &query).unwrap_err();

        assert!(
            err.message.contains("different row shapes"),
            "expected a clear shape-mismatch diagnostic, got: {}",
            err.message
        );
    }

    /// UNION arm widening, nested against non-nested. `widen_type`'s
    /// "different types, left wins" rule would keep the strongly-typed
    /// struct and then deserialize the right arm's arbitrary JSON into it at
    /// runtime, with nothing failing at build time.
    #[test]
    fn test_union_nested_arm_against_plain_json_arm_is_rejected() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetUserPostsOrEmpty
-- @returns :many
SELECT json_agg(p.*) AS posts FROM posts p
UNION
SELECT '[]'::json AS posts;",
        )
        .unwrap();

        let err = analyze(&catalog, &query).unwrap_err();

        assert!(
            err.message.contains("nested aggregate") && err.message.contains("left arm"),
            "expected a nested-vs-non-nested diagnostic naming the offending side, got: {}",
            err.message
        );
    }

    /// The mirror image: the nested arm on the right, where the pre-fix
    /// behaviour silently discarded the struct entirely.
    #[test]
    fn test_union_plain_json_arm_against_nested_arm_is_rejected() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetEmptyOrUserPosts
-- @returns :many
SELECT '[]'::json AS posts
UNION
SELECT json_agg(p.*) AS posts FROM posts p;",
        )
        .unwrap();

        let err = analyze(&catalog, &query).unwrap_err();

        assert!(
            err.message.contains("nested aggregate") && err.message.contains("right arm"),
            "expected a nested-vs-non-nested diagnostic naming the offending side, got: {}",
            err.message
        );
    }

    /// Guardrail: `array_agg` shares the wildcard-argument shape with
    /// `json_agg` but aggregates into a SQL array, not JSON. It must keep
    /// resolving through the pre-existing `get_first_arg_type` path and
    /// never acquire a nested struct.
    #[test]
    fn test_array_agg_wildcard_never_produces_nested_struct() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPostIdsAgg
-- @returns :one
SELECT array_agg(p.id) AS ids FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.columns[0].neutral_type, "array<int32>");
        assert!(result.nested_structs.is_empty());
    }

    /// The engine gate, independent of the dialect gate: Redshift catalogs
    /// map to `SqlDialect::PostgreSQL` (see `SqlDialect::from_str`) but
    /// Redshift has no `json_agg`, so inference must fall back to plain
    /// `json` exactly as it does for MySQL.
    #[test]
    fn test_json_agg_redshift_engine_falls_back_to_plain_json() {
        let catalog = make_catalog().with_engine("redshift");
        let query = parse_query(
            "-- @name GetPostsRedshift
-- @returns :many
SELECT json_agg(p.*) AS posts FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(
            result.columns[0].neutral_type, "json",
            "a PostgreSQL-dialect catalog on the Redshift engine must not infer a nested struct"
        );
        assert!(result.nested_structs.is_empty());
    }

    /// The same catalog with the engine stated as PostgreSQL still infers,
    /// so the gate above is the engine and not merely the presence of
    /// `with_engine`.
    #[test]
    fn test_json_agg_postgresql_engine_still_infers() {
        let catalog = make_catalog().with_engine("postgresql");
        let query = parse_query(
            "-- @name GetPostsPg
-- @returns :many
SELECT json_agg(p.*) AS posts FROM posts p;",
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();

        assert_eq!(result.nested_structs.len(), 1);
    }

    /// The explicit column alias list on a CTE (`WITH t(a, b) AS ...`) must
    /// name the CTE's columns even when the body projection carries no names
    /// of its own — `SELECT 1` otherwise labels its column "unknown" and the
    /// outer query's `SELECT a, b` fails with "column a does not exist".
    #[test]
    fn test_cte_column_alias_list_names_literal_columns() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPair
-- @returns :many
WITH t(a, b) AS (SELECT 1, 2) SELECT a, b FROM t;",
        )
        .unwrap();

        let result = analyze(&catalog, &query).unwrap();

        let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        // `1` and `2` are small integer literals, typed `int32` to match
        // PostgreSQL (see #122).
        assert!(result.columns.iter().all(|c| c.neutral_type == "int32"));
    }

    /// `SELECT *` over a CTE with an explicit column alias list expands to the
    /// aliased names, not the body's inferred ones.
    #[test]
    fn test_cte_column_alias_list_consumed_by_select_star() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetPairStar
-- @returns :many
WITH t(a, b) AS (SELECT 1, 2) SELECT * FROM t;",
        )
        .unwrap();

        let result = analyze(&catalog, &query).unwrap();

        let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
    }

    /// A CTE column alias list whose entry count disagrees with the body's
    /// column count is a hard error (PostgreSQL rejects it too), not a
    /// positional guess.
    #[test]
    fn test_cte_column_alias_count_mismatch_is_rejected() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name GetMismatch
-- @returns :many
WITH t(a) AS (SELECT 1, 2) SELECT * FROM t;",
        )
        .unwrap();

        let err = analyze(&catalog, &query).unwrap_err();

        assert!(
            err.message
                .contains("CTE column alias list has 1 entries but the CTE body produces 2 columns"),
            "expected a column-alias-count diagnostic, got: {}",
            err.message
        );
    }

    /// Recursive CTEs resolve column references by name, so the alias list
    /// must also apply to the anchor's seeded scope — `t(n)` must make `n`
    /// referenceable inside the recursive term and in the outer query.
    #[test]
    fn test_recursive_cte_with_column_alias_list_names_columns() {
        let catalog = make_catalog();
        let query = parse_query(
            "-- @name CountDown
-- @returns :many
WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM t WHERE n < 10) SELECT n FROM t;",
        )
        .unwrap();

        let result = analyze(&catalog, &query).unwrap();

        let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["n"]);
        // `1` and `n + 1` are both `int32` (small integer literal, and the
        // anchor-typed `n` widened against another `int32` literal); see
        // #122.
        assert_eq!(result.columns[0].neutral_type, "int32");
    }

    // Regression tests for #170: `analyze_select` walks every projection
    // expression twice -- `collect_params_from_where` then `infer_expr_type`
    // -- and shapes like `Between` independently reach
    // `resolve_placeholder_position` from both passes. `$N` is idempotent
    // (it just parses the explicit number), but a bare `?` auto-increments,
    // so without occurrence-based memoization the second pass minted brand
    // new positions and doubled the reported parameter count. `?` only
    // tokenizes as `Token::Placeholder` in dialects that don't claim it for
    // PostgreSQL geometric operators (`?-`, `?|`, `?#`), so these use a
    // MySQL-dialect catalog and parse.

    fn make_mysql_catalog() -> Catalog {
        Catalog::from_ddl_with_dialect(
            &["CREATE TABLE users (id INTEGER PRIMARY KEY, age INTEGER);"],
            &SqlDialect::MySQL,
        )
        .unwrap()
    }

    #[test]
    fn test_bare_placeholder_in_between_projection_is_not_double_counted() {
        let catalog = make_mysql_catalog();
        let query = crate::parser::parse_query_with_dialect(
            "-- @name UsersInRange
-- @returns :many
SELECT (age BETWEEN ? AND ?) AS in_range FROM users;",
            &SqlDialect::MySQL,
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(
            result.params.len(),
            2,
            "two distinct `?` occurrences in a BETWEEN projection must report exactly two parameters, \
             not four"
        );
        assert_eq!(result.params[0].position, 1);
        assert_eq!(result.params[1].position, 2);
    }

    /// Same double-traversal shape as above, but inside a derived subquery,
    /// to prove the occurrence memo survives the sub-analyzer boundary the
    /// same way `positional_param_counter` and `next_nested_id` already do
    /// (see `TableFactor::Derived` in `scope.rs`).
    #[test]
    fn test_bare_placeholder_in_between_inside_derived_subquery_is_not_double_counted() {
        let catalog = make_mysql_catalog();
        let query = crate::parser::parse_query_with_dialect(
            "-- @name UsersInRangeSub
-- @returns :many
SELECT * FROM (SELECT (age BETWEEN ? AND ?) AS in_range FROM users) sub;",
            &SqlDialect::MySQL,
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(
            result.params.len(),
            2,
            "two distinct `?` occurrences inside a derived subquery must report exactly two parameters"
        );
        assert_eq!(result.params[0].position, 1);
        assert_eq!(result.params[1].position, 2);
    }

    /// A placeholder inside a derived subquery and one outside it must number
    /// sequentially across the boundary, exercising the same
    /// into-and-back-out threading `positional_param_counter` already relies
    /// on -- `resolved_placeholders` has to make the same round trip.
    #[test]
    fn test_bare_placeholder_numbers_sequentially_across_derived_subquery_boundary() {
        let catalog = make_mysql_catalog();
        let query = crate::parser::parse_query_with_dialect(
            "-- @name UsersInRangeOuter
-- @returns :many
SELECT * FROM (SELECT id, age FROM users WHERE age > ?) sub WHERE sub.id > ?;",
            &SqlDialect::MySQL,
        )
        .unwrap();
        let result = analyze(&catalog, &query).unwrap();
        assert_eq!(result.params.len(), 2);
        assert_eq!(
            result.params[0].position, 1,
            "the placeholder inside the subquery must resolve first"
        );
        assert_eq!(
            result.params[1].position, 2,
            "the outer placeholder must continue numbering after it"
        );
    }
}
