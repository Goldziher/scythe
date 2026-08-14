use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{NamingConfig, field_name, param_name};
use scythe_backend::types::resolve_type_pair;

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam};
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::{ResolvedColumn, ResolvedParam};
use crate::overrides::{TypeOverride, find_override};

/// Resolve analyzed columns into resolved columns using a backend manifest.
///
/// When `overrides` is non-empty, each column is checked against the override
/// list before the normal type-resolution path. The first matching override
/// replaces the column's neutral type.
///
/// `column_match` is built per column from [`AnalyzedColumn::source_relation`] -- the
/// analyzer's real owning-table for that column -- rather than from a single query-level
/// table name. A query-level name only ever exists for a single-table `SELECT *`
/// (`detect_select_star_source`), so building `column_match` from it made a `column =
/// "table.col"` override a no-op for every other projection (#189).
pub fn resolve_columns(
    columns: &[AnalyzedColumn],
    manifest: &BackendManifest,
    overrides: &[TypeOverride],
) -> Result<Vec<ResolvedColumn>, ScytheError> {
    let resolved: Vec<ResolvedColumn> = columns
        .iter()
        .map(|col| {
            let column_match = match col.source_relation {
                Some(ref relation) => format!("{relation}.{}", col.name),
                None => String::new(),
            };
            let effective_neutral_type =
                find_override(overrides, &column_match, &col.neutral_type).unwrap_or(&col.neutral_type);

            let (full_type, lang_type) = resolve_type_pair(effective_neutral_type, manifest, col.nullable)
                .map(|(f, l)| (f.into_owned(), l.into_owned()))
                .map_err(|e| {
                    ScytheError::new(
                        ErrorCode::InternalError,
                        format!("type resolution failed for column '{}': {}", col.name, e),
                    )
                })?;
            Ok(ResolvedColumn {
                name: col.name.clone(),
                field_name: field_name(&col.name, &manifest.naming).into_owned(),
                lang_type,
                full_type,
                neutral_type: effective_neutral_type.to_string(),
                nullable: col.nullable,
                join_group: col.join_group.clone(),
                nullable_before_join: col.nullable_before_join,
                sql_type: col.sql_type.clone(),
            })
        })
        .collect::<Result<_, ScytheError>>()?;

    check_field_name_collisions(
        resolved.iter().map(|c| (c.name.as_str(), c.field_name.as_str())),
        "columns",
        &manifest.naming,
    )?;

    Ok(resolved)
}

/// Resolve analyzed params into resolved params using a backend manifest.
///
/// When `overrides` is non-empty, each param is checked against the override
/// list before the normal type-resolution path.
///
/// `column_match` prefers [`AnalyzedParam::source_relation`] -- the real relation a direct
/// `WHERE table.column = $N` comparison bound this parameter against -- over the query-level
/// `source_table` fallback. `source_table` only ever comes from `detect_select_star_source`
/// (a single-table `SELECT *`), so before `AnalyzedParam` carried its own relation, a
/// qualified `column = "table.col"` override on a parameter was a silent no-op for any other
/// projection -- the parameter half of #189, left tracked when the column half was fixed in
/// 99227e8e. `source_table` stays as the fallback (rather than being removed) so a parameter
/// with no traceable owning column -- `IN` list, `LIKE`, a literal comparison -- keeps
/// resolving the same way it did before this field existed.
pub fn resolve_params(
    params: &[AnalyzedParam],
    manifest: &BackendManifest,
    overrides: &[TypeOverride],
    source_table: &str,
) -> Result<Vec<ResolvedParam>, ScytheError> {
    let resolved: Vec<ResolvedParam> = params
        .iter()
        .map(|param| {
            let column_match = match param.source_relation {
                Some(ref relation) => format!("{relation}.{}", param.name),
                None if !source_table.is_empty() => format!("{}.{}", source_table, param.name),
                None => String::new(),
            };
            let effective_neutral_type =
                find_override(overrides, &column_match, &param.neutral_type).unwrap_or(&param.neutral_type);

            let (full_type, lang_type) = resolve_type_pair(effective_neutral_type, manifest, param.nullable)
                .map(|(f, l)| (f.into_owned(), l.into_owned()))
                .map_err(|e| {
                    ScytheError::new(
                        ErrorCode::InternalError,
                        format!("type resolution failed for param '{}': {}", param.name, e),
                    )
                })?;
            let borrowed_type = param_type_to_borrowed(&full_type);
            Ok(ResolvedParam {
                name: param.name.clone(),
                field_name: param_name(&param.name, &manifest.naming),
                lang_type,
                full_type,
                borrowed_type,
                neutral_type: effective_neutral_type.to_string(),
                nullable: param.nullable,
            })
        })
        .collect::<Result<_, ScytheError>>()?;

    check_field_name_collisions(
        resolved.iter().map(|p| (p.name.as_str(), p.field_name.as_str())),
        "params",
        &manifest.naming,
    )?;

    Ok(resolved)
}

/// Every `"table.column"` reference actually present across a set of analyzed params, keyed
/// on [`AnalyzedParam::source_relation`] -- the parameter counterpart of
/// [`crate::overrides::column_references`], which does the same job for
/// [`AnalyzedColumn::source_relation`]. A param with no single owning relation (`IN` list,
/// `LIKE` pattern, literal comparison: `source_relation: None`) contributes nothing, which is
/// exactly what a qualified `column` override can never legitimately target either.
///
/// Not wired into the CLI's unmatched-override preflight
/// ([`crate::overrides::unmatched_column_overrides`]) by this change -- that call site
/// (`scythe-cli`'s `check_type_overrides_resolve`) needs to chain this alongside
/// `column_references` into the `known` set it already builds; this function is the
/// primitive that closing gap needs, reusing the existing diagnostic rather than adding a
/// second one (#189's remainder).
pub fn param_references<'a>(params: impl Iterator<Item = &'a AnalyzedParam>) -> ahash::AHashSet<String> {
    params
        .filter_map(|param| {
            param
                .source_relation
                .as_deref()
                .map(|relation| format!("{relation}.{}", param.name))
        })
        .collect()
}

/// Reject two SQL identifiers in the same column list or param list that
/// collapse onto the same generated field name.
///
/// Not just a `camelCase` problem: `field_name` always runs `to_snake_case`
/// as its base transform (see `naming::apply_case`), and `to_snake_case` is
/// not the identity function for mixed- or upper-case identifiers, so
/// quoted SQL like `SELECT "USER_ID", user_id FROM t` folds both onto
/// `user_id` even under the default `snake_case`. The analyzer's
/// `duplicate_alias` check cannot catch this in any case -- it runs on the
/// raw SQL names, before this conversion. So this runs unconditionally,
/// regardless of `field_case`: it is O(n^2) over one query's column or
/// param list, typically well under thirty items, so the cost is
/// negligible next to the type resolution this function already does.
///
/// `pub(crate)` rather than private: `lib.rs`'s `:grouped` path reuses this
/// same check against the parent column list *plus* the synthesized
/// `children` field a grouped backend injects, so that collision is caught
/// by the same single rule instead of a second, independent one (#188).
pub(crate) fn check_field_name_collisions<'a>(
    items: impl Iterator<Item = (&'a str, &'a str)>,
    kind: &str,
    naming: &NamingConfig,
) -> Result<(), ScytheError> {
    let items: Vec<(&str, &str)> = items.collect();
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let (sql_a, field_a) = items[i];
            let (sql_b, field_b) = items[j];
            if field_a == field_b {
                let alternative = if naming.field_case == "snake_case" {
                    String::new()
                } else {
                    ", or set field_case = \"snake_case\"".to_string()
                };
                return Err(ScytheError::new(
                    ErrorCode::DuplicateAlias,
                    format!(
                        "{kind} '{sql_a}' and '{sql_b}' both resolve to field name '{field_a}' under \
                         field_case = \"{}\" -- alias one of them in SQL{alternative}",
                        naming.field_case
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Reject two generated *type* names -- an enum, a query's own row/model
/// struct -- that collapse onto the same identifier within one generated
/// file.
///
/// The same `ErrorCode::DuplicateAlias` mechanism
/// [`check_field_name_collisions`] uses, generalized for names cased under
/// `struct_case` rather than `field_case`: an enum whose SQL name
/// case-converts to the same spelling as another enum, or as the query's own
/// row/model type, is two type declarations sharing one name -- `E0428` in
/// Rust, a redeclaration in every other target (#136). Deliberately not
/// built by widening `check_field_name_collisions` itself: that function's
/// error message offers "switch `field_case`" as the fix, which does not
/// apply here -- every manifest's `struct_case` is PascalCase, so there is
/// no alternative case to point at instead.
pub(crate) fn check_type_name_collisions<'a>(
    items: impl Iterator<Item = (&'a str, &'a str)>,
    kind: &str,
) -> Result<(), ScytheError> {
    let items: Vec<(&str, &str)> = items.collect();
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let (source_a, name_a) = items[i];
            let (source_b, name_b) = items[j];
            if name_a == name_b {
                return Err(ScytheError::new(
                    ErrorCode::DuplicateAlias,
                    format!(
                        "{kind} '{source_a}' and '{source_b}' both resolve to generated name \
                         '{name_a}' -- rename one of them"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Convert a resolved Rust type to its borrowed form for function parameters.
/// Copy types (primitives) stay as-is; String becomes &str; other non-Copy types get a & prefix.
pub fn param_type_to_borrowed(rust_type: &str) -> String {
    let copy_types = ["bool", "i16", "i32", "i64", "f32", "f64", "u64"];
    if copy_types.contains(&rust_type) {
        return rust_type.to_string();
    }
    if rust_type == "String" {
        return "&str".to_string();
    }
    if let Some(inner) = rust_type.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        let borrowed_inner = param_type_to_borrowed(inner);
        return format!("Option<{}>", borrowed_inner);
    }
    if let Some(inner) = rust_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        return format!("&[{}]", inner);
    }
    format!("&{}", rust_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_type_to_borrowed_string() {
        assert_eq!(param_type_to_borrowed("String"), "&str");
    }

    #[test]
    fn test_param_type_to_borrowed_vec() {
        assert_eq!(param_type_to_borrowed("Vec<i32>"), "&[i32]");
        assert_eq!(param_type_to_borrowed("Vec<String>"), "&[String]");
    }

    #[test]
    fn test_param_type_to_borrowed_passthrough() {
        assert_eq!(param_type_to_borrowed("i32"), "i32");
        assert_eq!(param_type_to_borrowed("i64"), "i64");
        assert_eq!(param_type_to_borrowed("bool"), "bool");
        assert_eq!(param_type_to_borrowed("f64"), "f64");
    }

    #[test]
    fn test_param_type_to_borrowed_option_string() {
        assert_eq!(param_type_to_borrowed("Option<String>"), "Option<&str>");
    }

    #[test]
    fn test_param_type_to_borrowed_option_copy() {
        assert_eq!(param_type_to_borrowed("Option<i32>"), "Option<i32>");
    }

    #[test]
    fn test_param_type_to_borrowed_other() {
        assert_eq!(param_type_to_borrowed("Uuid"), "&Uuid");
        assert_eq!(param_type_to_borrowed("NaiveDateTime"), "&NaiveDateTime");
    }

    fn test_manifest(field_case: &str) -> BackendManifest {
        use ahash::AHashMap;
        use scythe_backend::manifest::{BackendMeta, TypeMappings};

        let mut scalars = AHashMap::new();
        scalars.insert("string".to_string(), "String".to_string());
        scalars.insert("int32".to_string(), "i32".to_string());

        BackendManifest {
            backend: BackendMeta {
                name: "test".to_string(),
                language: "rust".to_string(),
                file_extension: "rs".to_string(),
                engine: "postgresql".to_string(),
                description: None,
            },
            types: TypeMappings {
                scalars,
                containers: AHashMap::new(),
                docblock_containers: AHashMap::new(),
            },
            naming: NamingConfig {
                struct_case: "PascalCase".to_string(),
                fn_case: "snake_case".to_string(),
                enum_variant_case: "PascalCase".to_string(),
                row_suffix: "Row".to_string(),
                field_case: field_case.to_string(),
                reserved: Vec::new(),
                reserved_bindings: Vec::new(),
                sanitize_field_names: false,
            },
            imports: None,
        }
    }

    fn test_column(name: &str) -> AnalyzedColumn {
        AnalyzedColumn {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            ..Default::default()
        }
    }

    /// A column with a real owning relation, the way the analyzer populates it for both an
    /// explicit select list and a `SELECT *` expansion alike (#189) -- unlike `test_column`
    /// above, whose `source_relation` defaults to `None`, the shape a computed expression or
    /// a literal gets.
    fn test_column_with_relation(name: &str, relation: &str) -> AnalyzedColumn {
        AnalyzedColumn {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            source_relation: Some(relation.to_string()),
            ..Default::default()
        }
    }

    fn test_param(name: &str, position: i64) -> AnalyzedParam {
        AnalyzedParam {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            position,
            source_relation: None,
        }
    }

    /// A param with a real owning relation, the way the analyzer populates it for a direct
    /// `col = $N` comparison (`try_bind_param_from_comparison`) -- unlike `test_param` above,
    /// whose `source_relation` defaults to `None`, the shape an `IN` list, a `LIKE` pattern, or
    /// a literal comparison gets (#189's remainder).
    fn test_param_with_relation(name: &str, position: i64, relation: &str) -> AnalyzedParam {
        AnalyzedParam {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            position,
            source_relation: Some(relation.to_string()),
        }
    }

    #[test]
    fn test_resolve_columns_defaults_to_snake_case_field_names() {
        let manifest = test_manifest("snake_case");
        let resolved = resolve_columns(&[test_column("UserId")], &manifest, &[]).unwrap();
        assert_eq!(resolved[0].field_name, "user_id");
    }

    #[test]
    fn test_resolve_columns_honors_camel_case_field_case() {
        let manifest = test_manifest("camelCase");
        let resolved = resolve_columns(&[test_column("user_id")], &manifest, &[]).unwrap();
        assert_eq!(resolved[0].field_name, "userId");
    }

    #[test]
    fn test_resolve_params_honors_camel_case_field_case() {
        let manifest = test_manifest("camelCase");
        let resolved = resolve_params(&[test_param("user_id", 1)], &manifest, &[], "").unwrap();
        assert_eq!(resolved[0].field_name, "userId");
    }

    /// This must fail before the fix: `field_name` always runs
    /// `to_snake_case` as its base transform, and `to_snake_case` is not the
    /// identity function for mixed-/upper-case identifiers --
    /// `to_snake_case("USER_ID") == "user_id"`. Quoted SQL like
    /// `SELECT "USER_ID", user_id FROM t` is legal and passes the
    /// analyzer's case-sensitive `duplicate_alias` check, so this collision
    /// must be caught here, even under the default `field_case`. Skipping
    /// the check when `field_case == "snake_case"` (the previous behavior)
    /// let it through and produced two struct fields both named `user_id`.
    #[test]
    fn test_resolve_columns_rejects_collision_under_snake_case() {
        let manifest = test_manifest("snake_case");
        let err = resolve_columns(&[test_column("user_id"), test_column("USER_ID")], &manifest, &[])
            .expect_err("user_id and USER_ID must collide under snake_case too");
        let message = err.to_string();
        assert!(message.contains("user_id"), "{message}");
        assert!(message.contains("USER_ID"), "{message}");
        assert!(
            !message.contains("or set field_case"),
            "already snake_case -- switching field_case cannot fix this: {message}"
        );
    }

    #[test]
    fn test_resolve_columns_rejects_collision_under_camel_case() {
        let manifest = test_manifest("camelCase");
        let err = resolve_columns(&[test_column("user_id"), test_column("userId")], &manifest, &[])
            .expect_err("user_id and userId must collide under camelCase");
        let message = err.to_string();
        assert!(message.contains("user_id"), "{message}");
        assert!(message.contains("userId"), "{message}");
        assert!(message.contains("field_case"), "{message}");
    }

    #[test]
    fn test_resolve_columns_rejects_collision_across_four_spellings() {
        let manifest = test_manifest("camelCase");
        for (a, b) in [
            ("user_id", "USER_ID"),
            ("user_id", "UserId"),
            ("USER_ID", "userId"),
            ("UserId", "userId"),
        ] {
            resolve_columns(&[test_column(a), test_column(b)], &manifest, &[])
                .expect_err(&format!("{a} and {b} must collide under camelCase"));
        }
    }

    #[test]
    fn test_resolve_columns_rejects_double_underscore_collision() {
        let manifest = test_manifest("camelCase");
        resolve_columns(&[test_column("a_b"), test_column("a__b")], &manifest, &[])
            .expect_err("a_b and a__b must both collapse to aB under camelCase");
    }

    #[test]
    fn test_resolve_columns_rejects_leading_underscore_collision() {
        let manifest = test_manifest("camelCase");
        resolve_columns(&[test_column("id"), test_column("_id")], &manifest, &[])
            .expect_err("id and _id must both collapse to id under camelCase");
    }

    #[test]
    fn test_resolve_params_rejects_collision_under_camel_case() {
        let manifest = test_manifest("camelCase");
        let err = resolve_params(&[test_param("user_id", 1), test_param("userId", 2)], &manifest, &[], "")
            .expect_err("user_id and userId must collide under camelCase");
        assert!(err.to_string().contains("params"), "{err}");
    }

    /// Must fail before the fix: `column_match` used to be built once, for the whole query,
    /// from `analyzed.source_table` -- which `detect_select_star_source` only ever
    /// populates for a single-table `SELECT *`. For `SELECT id FROM users` (an explicit
    /// select list), that query-level name was empty, so `column_match` was `""` for every
    /// column and `"users.id"` could never match it -- the override silently never fired
    /// (#189). Rebuilding `column_match` per column from `AnalyzedColumn::source_relation`
    /// fixes it because the analyzer populates that field the same way regardless of
    /// whether the column came from a wildcard or an explicit list.
    #[test]
    fn test_resolve_columns_applies_qualified_override_on_explicit_select_list() {
        let manifest = test_manifest("snake_case");
        let overrides = vec![TypeOverride {
            column: Some("users.id".to_string()),
            db_type: None,
            neutral_type: Some("int32".to_string()),
        }];
        let resolved = resolve_columns(&[test_column_with_relation("id", "users")], &manifest, &overrides).unwrap();
        assert_eq!(resolved[0].neutral_type, "int32");
        assert_eq!(resolved[0].lang_type, "i32");
    }

    /// The scenario #189 says already worked and must keep working: a `SELECT *` expansion
    /// resolves every column to the same single table, and the analyzer stamps that same
    /// `source_relation` on each one, so a qualified override still applies post-fix.
    #[test]
    fn test_resolve_columns_qualified_override_still_applies_for_select_star() {
        let manifest = test_manifest("snake_case");
        let overrides = vec![TypeOverride {
            column: Some("users.id".to_string()),
            db_type: None,
            neutral_type: Some("int32".to_string()),
        }];
        let columns = [
            test_column_with_relation("id", "users"),
            test_column_with_relation("name", "users"),
        ];
        let resolved = resolve_columns(&columns, &manifest, &overrides).unwrap();
        assert_eq!(
            resolved[0].neutral_type, "int32",
            "id: the qualified override must apply"
        );
        assert_eq!(
            resolved[1].neutral_type, "string",
            "name: untouched by an override naming a different column"
        );
    }

    /// Must fail before the fix: `TypeOverride::matches` returned `false` as soon as
    /// `column` was set and didn't match, without ever checking `db_type` on that same
    /// entry -- a combined `column` + `db_type` override went completely silent for every
    /// column but the one literally named in `column` (#189), instead of degrading to the
    /// type-level rule.
    #[test]
    fn test_resolve_columns_combined_override_falls_through_to_db_type() {
        let manifest = test_manifest("snake_case");
        let overrides = vec![TypeOverride {
            column: Some("users.name".to_string()),
            db_type: Some("string".to_string()),
            neutral_type: Some("int32".to_string()),
        }];
        // `orders.total` doesn't match `column`, but its neutral_type ("string") matches
        // `db_type` -- the entry must still fire via that fallback.
        let resolved = resolve_columns(&[test_column_with_relation("total", "orders")], &manifest, &overrides).unwrap();
        assert_eq!(resolved[0].neutral_type, "int32");
    }

    /// Must fail before the fix: `resolve_params`'s `column_match` was built once, for the
    /// whole query, from a `source_table: &str` argument -- which `detect_select_star_source`
    /// only ever populates for a single-table `SELECT *`. For `SELECT id FROM users WHERE
    /// email = $1` (an explicit select list), `source_table` was empty, so `column_match` was
    /// `""` for every param and `"users.email"` could never match it -- the override silently
    /// never fired (#189's remainder). Building `column_match` from
    /// `AnalyzedParam::source_relation` first fixes it because the analyzer now populates that
    /// field from the column a direct comparison bound the parameter against, regardless of
    /// whether the query is `SELECT *`.
    #[test]
    fn test_resolve_params_applies_qualified_override_on_explicit_select_list() {
        let manifest = test_manifest("snake_case");
        let overrides = vec![TypeOverride {
            column: Some("users.email".to_string()),
            db_type: None,
            neutral_type: Some("int32".to_string()),
        }];
        let resolved = resolve_params(
            &[test_param_with_relation("email", 1, "users")],
            &manifest,
            &overrides,
            "",
        )
        .unwrap();
        assert_eq!(resolved[0].neutral_type, "int32");
        // ~keep Assert the resolved language type too: the override has to survive
        // `resolve_type`, not merely relabel the neutral type on the way past it.
        assert_eq!(resolved[0].lang_type, "i32");
    }

    /// A parameter with no single owning column -- an `IN` list, a `LIKE` pattern, a literal
    /// comparison -- keeps `source_relation: None`. A qualified override naming a table and
    /// column therefore cannot match it via `resolve_params` alone (#189's remainder says this
    /// case must be *reported*, not silently accepted as a match; that diagnostic is built
    /// from [`crate::overrides::column_references`]/`unmatched_column_overrides` plus this
    /// param's own qualified reference -- see the module doc on why `resolve_params` itself
    /// only ever silently falls through here rather than erroring).
    #[test]
    fn test_resolve_params_qualified_override_does_not_match_param_with_no_relation() {
        let manifest = test_manifest("snake_case");
        let overrides = vec![TypeOverride {
            column: Some("users.email".to_string()),
            db_type: None,
            neutral_type: Some("int32".to_string()),
        }];
        let resolved = resolve_params(&[test_param("email", 1)], &manifest, &overrides, "").unwrap();
        assert_eq!(
            resolved[0].neutral_type, "string",
            "no source_relation means the qualified override cannot legitimately target this param"
        );
    }

    #[test]
    fn test_resolve_columns_no_collision_when_names_differ() {
        let manifest = test_manifest("camelCase");
        let resolved = resolve_columns(&[test_column("user_id"), test_column("order_id")], &manifest, &[]).unwrap();
        assert_eq!(resolved[0].field_name, "userId");
        assert_eq!(resolved[1].field_name, "orderId");
    }

    #[test]
    fn test_check_type_name_collisions_rejects_two_enums() {
        let err = check_type_name_collisions(
            [("order-status", "OrderStatus"), ("order_status", "OrderStatus")].into_iter(),
            "enums",
        )
        .expect_err("order-status and order_status must collide as OrderStatus");
        assert_eq!(err.code, ErrorCode::DuplicateAlias);
        let message = err.to_string();
        assert!(message.contains("order-status"), "{message}");
        assert!(message.contains("order_status"), "{message}");
        assert!(message.contains("OrderStatus"), "{message}");
    }

    #[test]
    fn test_check_type_name_collisions_rejects_enum_vs_query_type() {
        let err = check_type_name_collisions(
            [("user_status", "GetUserRow"), ("<query row/model type>", "GetUserRow")].into_iter(),
            "enum and query type names",
        )
        .expect_err("an enum type name colliding with the query's own row type must be rejected");
        assert_eq!(err.code, ErrorCode::DuplicateAlias);
    }

    #[test]
    fn test_check_type_name_collisions_passes_when_distinct() {
        check_type_name_collisions(
            [("user_status", "UserStatus"), ("order_status", "OrderStatus")].into_iter(),
            "enums",
        )
        .expect("distinct generated names must not collide");
    }

    /// Unlike `check_field_name_collisions`, there is no `field_case`
    /// alternative to suggest -- `struct_case` has no per-manifest escape
    /// hatch, so the message must not claim one.
    #[test]
    fn test_check_type_name_collisions_message_has_no_field_case_suggestion() {
        let err = check_type_name_collisions([("a", "X"), ("b", "X")].into_iter(), "enums").unwrap_err();
        assert!(!err.to_string().contains("field_case"), "{err}");
    }

    #[test]
    fn test_param_references_collects_qualified_names() {
        let params = [test_param_with_relation("email", 1, "users"), test_param("total", 2)];
        let refs = param_references(params.iter());
        assert_eq!(refs.len(), 1);
        assert!(refs.contains("users.email"));
    }

    /// Must fail before the fix existed (there was no `AnalyzedParam::source_relation` to
    /// build this set from at all): a `column` override naming a table/column that no
    /// parameter's `source_relation` reaches is unmatched, the same way an unmatched column
    /// override is caught by `crate::overrides::unmatched_column_overrides` today. Composing
    /// `param_references` with that existing function (rather than a parallel "unmatched
    /// param override" check) is the extension #189's remainder asks for.
    #[test]
    fn test_unmatched_column_overrides_reports_qualified_param_override_that_matches_nothing() {
        use crate::overrides::unmatched_column_overrides;

        let known = param_references([test_param_with_relation("email", 1, "users")].iter());
        let overrides = vec![
            TypeOverride {
                column: Some("users.email".to_string()),
                db_type: None,
                neutral_type: Some("json".to_string()),
            },
            TypeOverride {
                column: Some("users.emial".to_string()),
                db_type: None,
                neutral_type: Some("json".to_string()),
            },
        ];
        let unmatched = unmatched_column_overrides(&overrides, &known);
        assert_eq!(unmatched.len(), 1);
        assert_eq!(unmatched[0].column.as_deref(), Some("users.emial"));
    }

    /// A param with no owning column contributes nothing to `param_references`, mirroring
    /// `column_references`'s handling of a computed/literal column.
    #[test]
    fn test_param_references_skips_params_with_no_source_relation() {
        let params = [test_param("total", 1)];
        let refs = param_references(params.iter());
        assert!(refs.is_empty());
    }
}
