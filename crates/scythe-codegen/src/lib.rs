pub mod backend_options;
pub mod backend_trait;
pub mod backends;
pub mod overrides;
pub mod provenance;
pub mod resolve;
pub mod validation;

pub use backend_trait::{
    CodegenBackend, RbsEnumInfo, RbsGenerationContext, RbsQueryInfo, ResolvedColumn, ResolvedParam,
};
pub use backends::get_backend;
pub use overrides::TypeOverride;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{row_struct_name, to_pascal_case};

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, EnumInfo, NestedStructInfo};
use scythe_core::catalog::Catalog;
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

/// SQL-level name of the field every `generate_grouped_structs`
/// implementation synthesizes on a `:grouped` query's parent struct to hold
/// the folded child rows. Named once here so the collision guard in
/// [`generate_with_backend_and_overrides`] and each backend's synthesized
/// field agree on what name they're both talking about (#188).
const GROUPED_CHILDREN_FIELD: &str = "children";

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct GeneratedCode {
    pub query_fn: Option<String>,
    pub row_struct: Option<String>,
    pub model_struct: Option<String>,
    pub enum_def: Option<String>,
    /// Nested-aggregate struct definitions this query needs, one entry per
    /// [`scythe_core::analyzer::NestedStructInfo`] the backend opted into.
    ///
    /// Kept out of `model_struct` (where composites go) because these have
    /// to be deduplicated *across* the queries written into one file: two
    /// queries can legitimately derive the same struct name, and emitting
    /// the definition twice is E0428 in Rust and a redeclaration in every
    /// other target. `model_struct` is already a rendered blob with no
    /// identity attached, so the name has to survive alongside the code for
    /// the writer to dedupe on. See `scythe-cli`'s `generate_for_backend`.
    pub nested_struct_defs: Vec<NestedStructDef>,
}

impl GeneratedCode {
    /// Build a `GeneratedCode` from [`Default`], assigning fields inside
    /// `init`.
    ///
    /// `#[non_exhaustive]` forbids a struct literal outside this crate --
    /// including with `..Default::default()` -- so downstream crates
    /// (`scythe-cli`, and anyone embedding the generator) have no other way
    /// to construct one. Mirrors
    /// [`scythe_core::analyzer::AnalyzedQuery::build`], and keeping
    /// construction a single expression avoids clippy's
    /// `field_reassign_with_default`.
    ///
    /// ```
    /// use scythe_codegen::GeneratedCode;
    ///
    /// let code = GeneratedCode::build(|c| {
    ///     c.query_fn = Some("fn get_user() {}".to_string());
    /// });
    /// assert!(code.row_struct.is_none());
    /// ```
    #[must_use]
    pub fn build(init: impl FnOnce(&mut Self)) -> Self {
        let mut code = Self::default();
        init(&mut code);
        code
    }
}

/// One rendered nested-aggregate struct definition, paired with the name it
/// declares so file writers can deduplicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedStructDef {
    /// snake_case name from [`scythe_core::analyzer::NestedStructInfo::name`],
    /// which is unique per (name, shape) within one query.
    pub name: String,
    /// The backend's rendered definition.
    pub code: String,
}

/// Simple singularization: remove trailing 's'.
pub fn singularize(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        format!("{stem}y")
    } else if name.ends_with("sses")
        || name.ends_with("shes")
        || name.ends_with("ches")
        || name.ends_with("xes")
        || name.ends_with("zes")
        || name.ends_with("ses")
    {
        name[..name.len() - 2].to_string()
    } else if name.ends_with('s') && !name.ends_with("ss") {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

/// Get the manifest for a backend. Defaults to PostgreSQL engine.
pub fn get_manifest_for_backend(backend_name: &str) -> Result<BackendManifest, ScytheError> {
    let backend = get_backend(backend_name, "postgresql")?;
    Ok(backend.manifest().clone())
}

/// Determine the struct name for a query (model struct or row struct).
fn determine_struct_name(analyzed: &AnalyzedQuery, manifest: &BackendManifest) -> String {
    if let Some(ref table_name) = analyzed.source_table {
        let singular = singularize(table_name);
        to_pascal_case(&singular).into_owned()
    } else {
        row_struct_name(&analyzed.name, &manifest.naming)
    }
}

/// Generate code using a specific backend.
pub fn generate_with_backend(
    analyzed: &AnalyzedQuery,
    backend: &dyn CodegenBackend,
) -> Result<GeneratedCode, ScytheError> {
    generate_with_backend_and_overrides(analyzed, backend, &[])
}

/// Generate code using a specific backend with type overrides.
pub fn generate_with_backend_and_overrides(
    analyzed: &AnalyzedQuery,
    backend: &dyn CodegenBackend,
    overrides: &[TypeOverride],
) -> Result<GeneratedCode, ScytheError> {
    let manifest = backend.manifest();
    let source_table = analyzed.source_table.as_deref().unwrap_or("");

    // ~keep Degradation pass: must run before any resolve_columns call. A backend
    // that doesn't opt into a nested struct (generate_nested_struct_def
    // returns Ok(None)) never sees json_nested<...> referencing it -- the
    // column is rewritten to plain json first, matching this backend's
    // output from before nested-aggregate inference existed byte for byte.
    //
    // Skipped entirely when nested_structs is empty (the overwhelming
    // majority of queries) so the common path stays the zero-copy
    // `&analyzed.columns` it always was -- degrade_unsupported_nested_structs
    // would otherwise clone every column, String fields included, on every
    // call regardless of whether the feature is in use.
    let (degraded_columns, nested_struct_defs) = if analyzed.nested_structs.is_empty() {
        (None, Vec::new())
    } else {
        let (cols, defs) = degrade_unsupported_nested_structs(&analyzed.columns, &analyzed.nested_structs, backend)?;
        (Some(cols), defs)
    };
    let nested_refs = NestedTypeRefs::collect(analyzed, &nested_struct_defs);

    let columns = resolve::resolve_columns(
        degraded_columns.as_deref().unwrap_or(&analyzed.columns),
        manifest,
        overrides,
        source_table,
    )?;
    let params = resolve::resolve_params(&analyzed.params, manifest, overrides, source_table)?;

    let mut result = GeneratedCode::default();

    let enum_def = generate_enum_defs_via_backend(analyzed, backend, &nested_refs)?;
    if !enum_def.is_empty() {
        result.enum_def = Some(enum_def);
    }

    let needs_row_struct = matches!(
        analyzed.command,
        QueryCommand::One | QueryCommand::Opt | QueryCommand::Many
    );
    if needs_row_struct && !analyzed.columns.is_empty() {
        if let Some(ref table_name) = analyzed.source_table {
            result.model_struct = Some(backend.generate_model_struct(table_name, &columns)?);
        } else {
            result.row_struct = Some(backend.generate_row_struct(&analyzed.name, &columns)?);
        }
    }

    let mut extra_defs = String::new();
    for comp in &analyzed.composites {
        // `analyzed.composites` covers types reachable from the top-level
        // columns *and* from nested-aggregate fields. A composite reachable
        // only through a nested struct this backend degraded away would be
        // an unused definition that did not exist before nested-aggregate
        // inference, breaking the byte-identity guarantee the degradation
        // pass exists to provide.
        let from_nested = nested_refs.composites.contains(comp.sql_name.as_str());
        if !from_nested && !type_referenced_by_columns(analyzed, "composite::", &comp.sql_name) {
            continue;
        }
        if !extra_defs.is_empty() {
            extra_defs.push_str("\n\n");
        }
        if from_nested {
            extra_defs.push_str(&backend.generate_composite_def_for_nested(comp)?);
        } else {
            extra_defs.push_str(&backend.generate_composite_def(comp)?);
        }
    }
    if !extra_defs.is_empty() {
        if let Some(ref mut existing) = result.model_struct {
            existing.push_str("\n\n");
            existing.push_str(&extra_defs);
        } else {
            result.model_struct = Some(extra_defs);
        }
    }
    result.nested_struct_defs = nested_struct_defs;

    let struct_name = determine_struct_name(analyzed, manifest);

    if analyzed.command == QueryCommand::Grouped {
        let group_by = analyzed.group_by.as_ref().ok_or_else(|| {
            ScytheError::new(
                ErrorCode::InternalError,
                format!(
                    "query '{}' is :grouped but is missing @group_by annotation",
                    analyzed.name
                ),
            )
        })?;

        let parent_struct_name = scythe_backend::naming::row_struct_name(&analyzed.name, &manifest.naming);
        let child_struct_name = {
            let suffix = &manifest.naming.row_suffix;
            let base = parent_struct_name.trim_end_matches(suffix.as_str());
            format!("{}Child{}", base, suffix)
        };

        // Same degradation pass as the flat-column path above, and the same
        // empty-nested_structs skip to keep the common case zero-copy: a
        // :grouped query's parent/child split is a distinct copy of the
        // analyzed columns, so an unsupported nested reference there needs
        // its own rewrite before resolution -- but only when there is one.
        let (degraded_parent, degraded_child) = if analyzed.nested_structs.is_empty() {
            (None, None)
        } else {
            let (dp, _) =
                degrade_unsupported_nested_structs(&group_by.parent_columns, &analyzed.nested_structs, backend)?;
            let (dc, _) =
                degrade_unsupported_nested_structs(&group_by.child_columns, &analyzed.nested_structs, backend)?;
            (Some(dp), Some(dc))
        };
        let parent_cols = resolve::resolve_columns(
            degraded_parent.as_deref().unwrap_or(&group_by.parent_columns),
            manifest,
            overrides,
            source_table,
        )?;
        let child_cols = resolve::resolve_columns(
            degraded_child.as_deref().unwrap_or(&group_by.child_columns),
            manifest,
            overrides,
            source_table,
        )?;

        // Every `generate_grouped_structs` implementation injects one more
        // field into the parent struct beyond `parent_cols`: `children`
        // (`GROUPED_CHILDREN_FIELD`), holding the folded child rows. That
        // field is synthesized here, not resolved from SQL, so it never
        // passes through `resolve::resolve_columns` -- which is exactly why
        // `check_field_name_collisions` inside it never sees it. Running the
        // same collision check again here, against the parent columns plus
        // this synthesized field, is the single rule that also catches a
        // real `u.children` column colliding with it (#188) instead of a
        // second, independent "no collisions" check that only ever covers
        // SQL-derived columns.
        let children_field_name = scythe_backend::naming::field_name(GROUPED_CHILDREN_FIELD, &manifest.naming);
        resolve::check_field_name_collisions(
            parent_cols
                .iter()
                .map(|c| (c.name.as_str(), c.field_name.as_str()))
                .chain(std::iter::once((
                    "<synthesized :grouped children field>",
                    children_field_name.as_ref(),
                ))),
            "parent columns",
            &manifest.naming,
        )?;

        result.row_struct = Some(backend.generate_grouped_structs(
            &parent_struct_name,
            &child_struct_name,
            &parent_cols,
            &child_cols,
            &group_by.key_column,
        )?);
        result.query_fn = Some(
            backend.generate_grouped_query_fn(&crate::backend_trait::GroupedQueryFn {
                analyzed,
                parent_struct_name: &parent_struct_name,
                child_struct_name: &child_struct_name,
                all_columns: &columns,
                parent_columns: &parent_cols,
                child_columns: &child_cols,
                params: &params,
                key_column: &group_by.key_column,
            })?,
        );
    } else {
        result.query_fn = Some(backend.generate_query_fn(analyzed, &struct_name, &columns, &params)?);
    }

    Ok(result)
}

/// Catalog types reachable from a nested-aggregate struct the backend
/// actually opted into.
///
/// A nested struct is decoded from JSON, so every type appearing in one of
/// its fields must be decodable that way too — which for the Rust backends
/// means an extra `serde::Deserialize`/`Serialize` the plain
/// `generate_enum_def`/`generate_composite_def` output does not carry (those
/// derive only the driver's own `sqlx::Type` / `postgres_types` traits). The
/// requirement is a property of *where the type is used*, not of the type,
/// so it cannot live in `EnumInfo`/`CompositeInfo`; it is computed here and
/// routed to the `*_for_nested` variants of those two trait methods.
///
/// Built from the definitions the backend returned rather than from
/// `analyzed.nested_structs`, so a backend that degraded the nested column
/// back to plain `json` also keeps its enum output byte-identical.
#[derive(Debug, Default)]
struct NestedTypeRefs {
    enums: ahash::AHashSet<String>,
    composites: ahash::AHashSet<String>,
}

impl NestedTypeRefs {
    fn collect(analyzed: &AnalyzedQuery, supported: &[NestedStructDef]) -> Self {
        let mut refs = Self::default();
        for nested in &analyzed.nested_structs {
            if !supported.iter().any(|def| def.name == nested.name) {
                continue;
            }
            for field in &nested.fields {
                let unwrapped = unwrap_containers(&field.neutral_type);
                if let Some(name) = unwrapped.strip_prefix("enum::") {
                    refs.enums.insert(name.to_string());
                } else if let Some(name) = unwrapped.strip_prefix("composite::") {
                    refs.composites.insert(name.to_string());
                }
            }
        }
        refs
    }
}

/// Strip any number of `array<...>` / `nullable<...>` container wrappers
/// from a neutral type, returning the innermost type.
///
/// These are the only two generic container names the analyzer emits around
/// an arbitrary inner neutral type (see `sql_type_to_neutral` and the outer
/// -join nullable-widening path in `scythe-core`); `json_nested<...>` is
/// deliberately not one of them -- see [`nested_struct_shape`]'s doc comment
/// -- so it is never unwrapped here.
///
/// The single point every "is `enum::x` / `composite::x` reachable"
/// question in this file must go through: matching only the bare
/// `"enum::x"` / `"composite::x"` string, as every call site here used to,
/// misses `array<enum::x>` and `array<nullable<composite::x>>` entirely --
/// the type is inferred correctly by `resolve.rs` -> `types.rs`, which
/// *does* recurse into containers, but the definition-emission side stayed
/// an exact-match check, so the row struct referenced a type whose
/// definition was never emitted (#187).
pub(crate) fn unwrap_containers(neutral: &str) -> &str {
    let mut current = neutral;
    loop {
        if let Some(inner) = current.strip_prefix("array<").and_then(|r| r.strip_suffix('>')) {
            current = inner.trim();
        } else if let Some(inner) = current.strip_prefix("nullable<").and_then(|r| r.strip_suffix('>')) {
            current = inner.trim();
        } else {
            return current;
        }
    }
}

/// Whether a query's top-level columns or params reference `sql_name` under
/// `prefix` (`"enum::"` or `"composite::"`), directly or through any number
/// of `array<...>` / `nullable<...>` wrappers.
///
/// Matches the `prefix + sql_name` neutral type after [`unwrap_containers`],
/// the same rule `generate_enum_defs_via_backend` uses, so "reachable from
/// columns" means the same thing in both places.
fn type_referenced_by_columns(analyzed: &AnalyzedQuery, prefix: &str, sql_name: &str) -> bool {
    let neutral = format!("{prefix}{sql_name}");
    analyzed
        .columns
        .iter()
        .any(|col| unwrap_containers(&col.neutral_type) == neutral)
        || analyzed
            .params
            .iter()
            .any(|param| unwrap_containers(&param.neutral_type) == neutral)
}

/// Whether `sql_name` is reachable from a nested-aggregate struct the
/// backend actually emitted a definition for.
///
/// The `supported` filter is what keeps a degraded backend's output
/// byte-identical: a type reachable only through a nested struct that was
/// rewritten back to plain `json` must not be emitted at all.
pub fn nested_type_is_emitted(
    analyzed: &AnalyzedQuery,
    supported: &[NestedStructDef],
    prefix: &str,
    sql_name: &str,
) -> bool {
    let neutral = format!("{prefix}{sql_name}");
    analyzed
        .nested_structs
        .iter()
        .filter(|nested| supported.iter().any(|def| def.name == nested.name))
        .any(|nested| {
            nested
                .fields
                .iter()
                .any(|field| unwrap_containers(&field.neutral_type) == neutral)
        })
}

/// Generate enum definitions via the backend trait.
fn generate_enum_defs_via_backend(
    analyzed: &AnalyzedQuery,
    backend: &dyn CodegenBackend,
    nested_refs: &NestedTypeRefs,
) -> Result<String, ScytheError> {
    use ahash::AHashSet;
    use std::fmt::Write;

    let mut out = String::new();
    let mut seen_enums: AHashSet<String> = AHashSet::new();

    let enum_sources: Vec<&str> = analyzed
        .columns
        .iter()
        .filter_map(|col| unwrap_containers(&col.neutral_type).strip_prefix("enum::"))
        .chain(
            analyzed
                .params
                .iter()
                .filter_map(|p| unwrap_containers(&p.neutral_type).strip_prefix("enum::")),
        )
        .chain(nested_refs.enums.iter().map(String::as_str))
        .collect();

    for sql_name in enum_sources {
        if !seen_enums.insert(sql_name.to_string()) {
            continue;
        }

        if !out.is_empty() {
            let _ = writeln!(out);
        }

        let stub_info;
        let enum_info = match analyzed.enums.iter().find(|e| e.sql_name == sql_name) {
            Some(info) => info,
            None => {
                stub_info = EnumInfo {
                    sql_name: sql_name.to_string(),
                    values: vec![],
                };
                &stub_info
            }
        };
        if nested_refs.enums.contains(sql_name) {
            out.push_str(&backend.generate_enum_def_for_nested(enum_info)?);
        } else {
            out.push_str(&backend.generate_enum_def(enum_info)?);
        }
    }

    Ok(out)
}

/// The decomposed form of a `json_nested<...>` neutral type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NestedColumnShape<'a> {
    /// `json_agg` produces an array of objects; `row_to_json` a single one.
    pub(crate) is_array: bool,
    /// Only ever true for an array: `json_agg` over an outer join emits
    /// `[null]` when nothing matched, so the *element* is optional. See
    /// `Analyzer::infer_nested_aggregate_type`.
    pub(crate) element_nullable: bool,
    /// PascalCase name of the generated struct.
    pub(crate) name: &'a str,
}

/// Decompose a `json_nested<...>` neutral type, or `None` for anything else.
///
/// `json_nested` is the container the analyzer emits for a nested aggregate,
/// deliberately distinct from `json_typed`, which is what a user's own
/// `@json` annotation produces (`analyzer/mod.rs`). Keeping them apart is
/// what lets this function -- and every backend decision keyed off it --
/// mean "a struct *scythe* synthesized and knows the field-by-field shape
/// of", rather than "some type the user named". Matching `json_typed` here
/// would make a user's `@json EventData` mapping (which may be a
/// `TypedDict`, a JSON array shape, or not constructible from a mapping at
/// all) take the nested-construction path.
///
/// ```text
/// "json_nested<array<nullable<GetUserPostsRowPosts>>>" -> array, nullable elements
/// "json_nested<array<GetUserPostsRowPosts>>"           -> array, non-null elements
/// "json_nested<GetPostAsJsonRowPost>"                  -> single object
/// "json_typed<EventData>", "int32", "json", "array<int32>" -> None
/// ```
pub(crate) fn nested_struct_shape(neutral_type: &str) -> Option<NestedColumnShape<'_>> {
    let inner = neutral_type.strip_prefix("json_nested<")?.strip_suffix('>')?;
    let Some(element) = inner.strip_prefix("array<").and_then(|r| r.strip_suffix('>')) else {
        return Some(NestedColumnShape {
            is_array: false,
            element_nullable: false,
            name: inner,
        });
    };
    match element.strip_prefix("nullable<").and_then(|r| r.strip_suffix('>')) {
        Some(name) => Some(NestedColumnShape {
            is_array: true,
            element_nullable: true,
            name,
        }),
        None => Some(NestedColumnShape {
            is_array: true,
            element_nullable: false,
            name: element,
        }),
    }
}

/// `nested_struct_shape` reduced to just the struct name, for callers (the
/// degradation pass) that don't care how it is wrapped.
fn nested_struct_pascal_name(neutral_type: &str) -> Option<&str> {
    nested_struct_shape(neutral_type).map(|shape| shape.name)
}

/// Degradation pass for nested-aggregate columns (`json_agg(o.*)`,
/// `row_to_json(u.*)`). Must run before `resolve::resolve_columns` -- every
/// caller that resolves `AnalyzedColumn`s into a backend's types needs this,
/// not just [`generate_with_backend_and_overrides`]: `scythe-cli`'s RBS
/// signature generation (`generate_rbs_if_supported`) calls
/// `resolve::resolve_columns` directly on a second, independent path, so it
/// must run this pass too or it silently bypasses the byte-identity
/// guarantee for any backend that hasn't opted in.
///
/// For each entry in `nested_structs`, asks the backend whether it opts in
/// via [`CodegenBackend::generate_nested_struct_def`]:
/// - `Ok(Some(def))`: the struct is supported. Its definition is collected
///   to append to the backend's output (the same channel
///   `generate_composite_def` output goes through), and columns
///   referencing it are left as `json_nested<...>`.
/// - `Ok(None)` (the default): not supported. Every column referencing it
///   is rewritten to plain `json` -- byte-identical to this backend's
///   output before nested-aggregate inference existed, since that is
///   exactly what the pre-existing `json_agg`/`row_to_json` arms already
///   produced.
/// - `Err(_)`: a genuine failure, propagated rather than degraded.
///
/// Returns the (possibly rewritten) columns and one [`NestedStructDef`] per
/// struct the backend supports. Skip calling this entirely when
/// `nested_structs` is empty (the common case) to keep that path zero-copy
/// -- see the callers in this file for the pattern.
pub fn degrade_unsupported_nested_structs(
    columns: &[AnalyzedColumn],
    nested_structs: &[NestedStructInfo],
    backend: &dyn CodegenBackend,
) -> Result<(Vec<AnalyzedColumn>, Vec<NestedStructDef>), ScytheError> {
    if nested_structs.is_empty() {
        return Ok((columns.to_vec(), Vec::new()));
    }

    use ahash::AHashSet;

    let mut unsupported: AHashSet<String> = AHashSet::new();
    let mut defs: Vec<NestedStructDef> = Vec::new();
    for nested in nested_structs {
        match backend.generate_nested_struct_def(nested)? {
            Some(def) => defs.push(NestedStructDef {
                name: nested.name.clone(),
                code: def,
            }),
            None => {
                unsupported.insert(to_pascal_case(&nested.name).into_owned());
            }
        }
    }

    if unsupported.is_empty() {
        return Ok((columns.to_vec(), defs));
    }

    let degraded = columns
        .iter()
        .cloned()
        .map(|mut col| {
            if let Some(name) = nested_struct_pascal_name(&col.neutral_type)
                && unsupported.contains(name)
            {
                col.neutral_type = "json".to_string();
            }
            col
        })
        .collect();

    Ok((degraded, defs))
}

/// Backward-compatible: generate code using the default sqlx backend.
pub fn generate(analyzed: &AnalyzedQuery) -> Result<GeneratedCode, ScytheError> {
    let backend = get_backend("rust-sqlx", "postgresql")?;
    generate_with_backend(analyzed, &*backend)
}

/// Stub for catalog-level codegen. Returns default for now.
pub fn generate_from_catalog(_catalog: &Catalog) -> Result<GeneratedCode, ScytheError> {
    Ok(GeneratedCode::default())
}

/// Generate a single enum definition using a specific backend.
///
/// `reachable_from_nested` selects [`CodegenBackend::generate_enum_def_for_nested`]
/// instead of the plain form. A file writer that emits enums once for the
/// whole file must pass `true` when *any* query in that file reaches the
/// enum from a nested-aggregate field: one definition serves every use, and
/// the nested-capable form is a superset (extra derives), so widening is
/// safe where narrowing is not.
pub fn generate_single_enum_def_with_backend(
    enum_info: &EnumInfo,
    backend: &dyn CodegenBackend,
    reachable_from_nested: bool,
) -> Result<String, ScytheError> {
    if reachable_from_nested {
        backend.generate_enum_def_for_nested(enum_info)
    } else {
        backend.generate_enum_def(enum_info)
    }
}

/// Backward-compatible: generate a single enum definition (sqlx backend).
/// Uses the manifest directly for backward compatibility with existing callers.
pub fn generate_single_enum_def(enum_info: &EnumInfo, manifest: &BackendManifest) -> String {
    use scythe_backend::naming::{enum_type_name, enum_variant_name};
    use std::fmt::Write;

    let mut out = String::with_capacity(256);
    let type_name = enum_type_name(&enum_info.sql_name, &manifest.naming);

    let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]");
    let _ = writeln!(
        out,
        "#[sqlx(type_name = \"{}\", rename_all = \"snake_case\")]",
        enum_info.sql_name
    );
    let _ = writeln!(out, "pub enum {type_name} {{");

    for value in &enum_info.values {
        let variant = enum_variant_name(value, &manifest.naming);
        let _ = writeln!(out, "    {variant},");
    }

    let _ = write!(out, "}}");
    out
}

/// Backward-compatible: load the default sqlx manifest.
pub fn load_or_default_manifest() -> Result<BackendManifest, ScytheError> {
    let b = backends::sqlx::SqlxBackend::new("postgresql")?;
    Ok(b.manifest().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig, NestedFieldInfo};
    use scythe_core::parser::QueryCommand;

    fn make_query(
        name: &str,
        command: QueryCommand,
        sql: &str,
        columns: Vec<AnalyzedColumn>,
        params: Vec<AnalyzedParam>,
    ) -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = name.to_string();
            aq.command = command;
            aq.sql = sql.to_string();
            aq.columns = columns;
            aq.params = params;
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = Vec::new();
            aq.enums = Vec::new();
            aq.optional_params = Vec::new();
            aq.group_by = None;
            aq.custom = Vec::new();
        })
    }

    #[test]
    fn test_generate_select_many() {
        let query = make_query(
            "ListUsers",
            QueryCommand::Many,
            "SELECT id, name, email FROM users",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "email".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    ..Default::default()
                },
            ],
            vec![],
        );

        let result = generate(&query).unwrap();

        let row_struct = result.row_struct.unwrap();
        assert!(row_struct.contains("pub struct ListUsersRow"));
        assert!(row_struct.contains("pub id: i32"));
        assert!(row_struct.contains("pub name: String"));
        assert!(row_struct.contains("pub email: Option<String>"));

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn list_users("));
        assert!(query_fn.contains("-> Result<Vec<ListUsersRow>, sqlx::Error>"));
        assert!(query_fn.contains(".fetch_all(pool)"));
    }

    #[test]
    fn test_generate_select_one_with_param() {
        let query = make_query(
            "GetUser",
            QueryCommand::One,
            "SELECT id, name FROM users WHERE id = $1",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate(&query).unwrap();

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn get_user("));
        assert!(query_fn.contains("id: i32"));
        assert!(query_fn.contains("-> Result<GetUserRow, sqlx::Error>"));
        assert!(query_fn.contains(".fetch_one(pool)"));
    }

    #[test]
    fn test_generate_exec() {
        let query = make_query(
            "DeleteUser",
            QueryCommand::Exec,
            "DELETE FROM users WHERE id = $1",
            vec![],
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate(&query).unwrap();

        assert!(result.row_struct.is_none());

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn delete_user("));
        assert!(query_fn.contains("-> Result<(), sqlx::Error>"));
        assert!(query_fn.contains(".execute(pool)"));
    }

    #[test]
    fn test_generate_with_enum_column() {
        let query = make_query(
            "GetUserStatus",
            QueryCommand::One,
            "SELECT id, status FROM users WHERE id = $1",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "status".to_string(),
                    neutral_type: "enum::user_status".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate(&query).unwrap();

        assert!(result.enum_def.is_some());
        let enum_def = result.enum_def.unwrap();
        assert!(enum_def.contains("pub enum UserStatus"));
        assert!(enum_def.contains("type_name = \"user_status\""));

        let row_struct = result.row_struct.unwrap();
        assert!(row_struct.contains("pub status: UserStatus"));
    }

    #[test]
    fn test_generate_from_catalog_returns_default() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER);"]).unwrap();
        let result = generate_from_catalog(&catalog).unwrap();
        assert!(result.query_fn.is_none());
        assert!(result.row_struct.is_none());
    }

    #[test]
    fn test_singularize_basic() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("orders"), "order");
        assert_eq!(singularize("posts"), "post");
    }

    #[test]
    fn test_singularize_ies() {
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("entries"), "entry");
    }

    #[test]
    fn test_singularize_sses() {
        assert_eq!(singularize("addresses"), "address");
        assert_eq!(singularize("classes"), "class");
    }

    #[test]
    fn test_singularize_no_change() {
        assert_eq!(singularize("status"), "statu");
        assert_eq!(singularize("boss"), "boss");
        assert_eq!(singularize("address"), "address");
    }

    #[test]
    fn test_singularize_shes_ches_xes() {
        assert_eq!(singularize("batches"), "batch");
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("wishes"), "wish");
    }

    fn make_grouped_query() -> AnalyzedQuery {
        let parent_cols = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "email".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let child_cols = vec![
            AnalyzedColumn {
                name: "order_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "total".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();

        AnalyzedQuery::build(|aq| {
            aq.name = "GetUsersWithOrders".to_string();
            aq.command = QueryCommand::Grouped;
            aq.sql = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
                  SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\n\
                  FROM users u\n\
                  JOIN orders o ON o.user_id = u.id"
                .to_string();
            aq.columns = all_cols;
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            });
            aq.custom = vec![];
        })
    }

    #[test]
    fn test_grouped_sqlx_structs() {
        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("pub struct GetUsersWithOrdersChildRow"),
            "missing child struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub order_id: i32"),
            "child struct missing order_id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub struct GetUsersWithOrdersRow"),
            "missing parent struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub id: i32"),
            "parent struct missing id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub name: String"),
            "parent struct missing name field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub email: String"),
            "parent struct missing email field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub children: Vec<GetUsersWithOrdersChildRow>"),
            "parent struct missing children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("pub struct GetUsersWithOrdersRow").unwrap();
        assert!(
            child_pos < parent_pos,
            "child struct must be defined before parent struct"
        );

        assert!(
            result.model_struct.is_none(),
            "grouped should not produce a model_struct"
        );
    }

    #[test]
    fn test_grouped_sqlx_query_fn() {
        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("pub async fn get_users_with_orders("),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("-> Result<Vec<GetUsersWithOrdersRow>, sqlx::Error>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("sqlx::query!("),
            "grouped fn must use sqlx::query!; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow {"),
            "fn must construct child struct; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("children: vec![child]"),
            "fn must initialize children vec; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("parent.children.push(child)"),
            "fn must fold child into existing parent; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Ok(result)"),
            "fn must return result; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_grouped_python_asyncpg_structs() {
        let backend = get_backend("python-asyncpg", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("class GetUsersWithOrdersChildRow"),
            "missing child class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("order_id: int"),
            "child class missing order_id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("class GetUsersWithOrdersRow"),
            "missing parent class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("id: int"),
            "parent class missing id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("name: str"),
            "parent class missing name field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: list[GetUsersWithOrdersChildRow]"),
            "parent class missing children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("class GetUsersWithOrdersRow").unwrap();
        assert!(
            child_pos < parent_pos,
            "child class must be defined before parent class"
        );
    }

    #[test]
    fn test_grouped_python_asyncpg_query_fn() {
        let backend = get_backend("python-asyncpg", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("async def get_users_with_orders(conn: Connection)"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("-> list[GetUsersWithOrdersRow]:"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("conn.fetch("),
            "fn must use conn.fetch; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow("),
            "fn must construct child class; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersRow(**parent_kwargs, children=children)"),
            "fn must construct parent with **kwargs; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("_index"),
            "fn must use index dict for O(1) lookup; got:\n{query_fn}"
        );
    }

    /// Minimal backend that implements only the required trait methods and
    /// leaves the grouped methods as their default (erroring) impl. Every
    /// shipped backend now supports grouped codegen, so the "not yet supported"
    /// path can only be exercised by a backend that has not opted in — this stub
    /// stands in for such a backend so the contract stays covered.
    struct StubBackend {
        manifest: BackendManifest,
    }

    impl CodegenBackend for StubBackend {
        fn name(&self) -> &str {
            "stub-backend"
        }
        fn manifest(&self) -> &BackendManifest {
            &self.manifest
        }
        fn manifest_mut(&mut self) -> &mut BackendManifest {
            &mut self.manifest
        }
        fn generate_row_struct(&self, _query_name: &str, _columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_model_struct(&self, _table_name: &str, _columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_query_fn(
            &self,
            _analyzed: &AnalyzedQuery,
            _struct_name: &str,
            _columns: &[ResolvedColumn],
            _params: &[ResolvedParam],
        ) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_enum_def(&self, _enum_info: &scythe_core::analyzer::EnumInfo) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_composite_def(
            &self,
            _composite: &scythe_core::analyzer::CompositeInfo,
        ) -> Result<String, ScytheError> {
            Ok(String::new())
        }
    }

    #[test]
    fn test_grouped_unsupported_backend_returns_clear_error() {
        let manifest = get_backend("rust-sqlx", "postgresql").unwrap().manifest().clone();
        let backend = StubBackend { manifest };
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend);
        assert!(
            result.is_err(),
            "stub backend should return an error for grouped queries"
        );
        let err = result.unwrap_err();
        assert!(
            err.message.contains("not yet supported"),
            "error should contain 'not yet supported', got: {}",
            err.message
        );
        assert!(
            err.message.contains("stub-backend"),
            "error should name the backend, got: {}",
            err.message
        );
    }

    #[test]
    fn test_tokio_postgres_backend_basic() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

        let query = make_query(
            "ListUsers",
            QueryCommand::Many,
            "SELECT id, name FROM users",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            vec![],
        );

        let result = generate_with_backend(&query, &*backend).unwrap();

        let row_struct = result.row_struct.unwrap();
        assert!(row_struct.contains("pub struct ListUsersRow"));
        assert!(row_struct.contains("pub id: i32"));
        assert!(row_struct.contains("pub name: String"));
        assert!(row_struct.contains("from_row"));
        assert!(row_struct.contains("tokio_postgres::Row"));
        assert!(!row_struct.contains("sqlx"));

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn list_users("));
        assert!(query_fn.contains("tokio_postgres::GenericClient"));
        assert!(query_fn.contains("tokio_postgres::Error"));
        assert!(!query_fn.contains("sqlx"));
    }

    #[test]
    fn test_tokio_postgres_enum() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

        let enum_info = scythe_core::analyzer::EnumInfo {
            sql_name: "user_status".to_string(),
            values: vec!["active".to_string(), "inactive".to_string()],
        };

        let def = backend.generate_enum_def(&enum_info).unwrap();
        assert!(def.contains("pub enum UserStatus"));
        assert!(def.contains("Active"));
        assert!(def.contains("Inactive"));
        assert!(def.contains("impl std::fmt::Display"));
        assert!(def.contains("impl std::str::FromStr"));
        assert!(!def.contains("sqlx"));
    }

    #[test]
    fn test_sql_with_least_coalesce_sum_preserved() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

        let query = make_query(
            "GetBillingAggregates",
            QueryCommand::One,
            "SELECT LEAST(COALESCE(SUM(ba.free_pages_remaining), 0), 10000) as aggregated_free_pages FROM billing_aggregates ba WHERE ba.customer_id = $1",
            vec![AnalyzedColumn {
                name: "aggregated_free_pages".to_string(),
                neutral_type: "int64".to_string(),
                nullable: false,
                ..Default::default()
            }],
            vec![AnalyzedParam {
                name: "customer_id".to_string(),
                neutral_type: "uuid".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.unwrap();

        assert!(
            query_fn.contains("LEAST(COALESCE(SUM(ba.free_pages_remaining), 0), 10000)"),
            "SQL should preserve original LEAST/COALESCE/SUM function names and casing"
        );
        assert!(
            query_fn.contains("as aggregated_free_pages"),
            "SQL should preserve alias keyword (as)"
        );
    }

    #[test]
    fn test_generated_rust_code_structure() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

        let query = make_query(
            "GetUser",
            QueryCommand::One,
            "SELECT id, name FROM users WHERE id = $1",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.unwrap();

        assert!(query_fn.contains("pub async fn get_user("));
        assert!(query_fn.contains("tokio_postgres::GenericClient"));
        assert!(query_fn.contains("GetUserRow"));

        assert!(query_fn.contains("SELECT id, name FROM users"));
    }

    // -----------------------------------------------------------------
    // Nested-aggregate degradation pass (degrade_unsupported_nested_structs).
    // -----------------------------------------------------------------

    fn a_nested_struct() -> NestedStructInfo {
        NestedStructInfo {
            name: "get_user_posts_row_posts".to_string(),
            fields: vec![NestedFieldInfo {
                name: "title".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            }],
        }
    }

    #[test]
    fn test_nested_struct_shape_unwraps_array() {
        let shape = nested_struct_shape("json_nested<array<GetUserPostsRowPosts>>").unwrap();
        assert!(shape.is_array);
        assert!(!shape.element_nullable);
        assert_eq!(shape.name, "GetUserPostsRowPosts");
    }

    /// The LEFT JOIN form: `json_agg` emits `[null]` for a non-matching row,
    /// so the analyzer wraps the element in `nullable<>`.
    #[test]
    fn test_nested_struct_shape_unwraps_nullable_array_element() {
        let shape = nested_struct_shape("json_nested<array<nullable<GetUserPostsRowPosts>>>").unwrap();
        assert!(shape.is_array);
        assert!(shape.element_nullable);
        assert_eq!(shape.name, "GetUserPostsRowPosts");
    }

    #[test]
    fn test_nested_struct_shape_bare() {
        let shape = nested_struct_shape("json_nested<GetPostAsJsonRowPost>").unwrap();
        assert!(!shape.is_array);
        assert_eq!(shape.name, "GetPostAsJsonRowPost");
    }

    /// The N9 regression: a user's own `@json` mapping resolves to
    /// `json_typed<...>`, a type scythe knows nothing about. It must not be
    /// mistaken for a struct scythe synthesized, or backends keyed off this
    /// (python-psycopg3's row construction) start calling constructors on a
    /// type that may not have one.
    #[test]
    fn test_nested_struct_shape_ignores_user_json_typed_mapping() {
        assert!(nested_struct_shape("json_typed<EventData>").is_none());
        assert!(nested_struct_pascal_name("json_typed<EventData>").is_none());
    }

    #[test]
    fn test_nested_struct_pascal_name_none_for_ordinary_types() {
        assert_eq!(nested_struct_pascal_name("int32"), None);
        assert_eq!(nested_struct_pascal_name("json"), None);
        assert_eq!(nested_struct_pascal_name("array<int32>"), None);
    }

    #[test]
    fn test_degrade_unsupported_nested_structs_rewrites_to_plain_json() {
        let manifest = get_backend("rust-sqlx", "postgresql").unwrap().manifest().clone();
        let backend = StubBackend { manifest };
        let nested = a_nested_struct();
        let columns = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "posts".to_string(),
                neutral_type: "json_nested<array<GetUserPostsRowPosts>>".to_string(),
                nullable: true,
                ..Default::default()
            },
        ];

        let (degraded, defs) = degrade_unsupported_nested_structs(&columns, &[nested], &backend).unwrap();

        assert_eq!(
            degraded[0].neutral_type, "int32",
            "an unrelated column must be untouched"
        );
        assert_eq!(
            degraded[1].neutral_type, "json",
            "StubBackend does not override generate_nested_struct_def, so it must degrade to plain json"
        );
        assert!(
            defs.is_empty(),
            "an unsupported backend must not emit a struct definition"
        );
    }

    /// Minimal backend that opts into nested-struct support, to prove the
    /// degradation pass leaves a supported column and definition alone.
    struct NestedSupportingBackend {
        manifest: BackendManifest,
    }

    impl CodegenBackend for NestedSupportingBackend {
        fn name(&self) -> &str {
            "nested-supporting-backend"
        }
        fn manifest(&self) -> &BackendManifest {
            &self.manifest
        }
        fn manifest_mut(&mut self) -> &mut BackendManifest {
            &mut self.manifest
        }
        fn generate_row_struct(&self, _query_name: &str, _columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_model_struct(&self, _table_name: &str, _columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_query_fn(
            &self,
            _analyzed: &AnalyzedQuery,
            _struct_name: &str,
            _columns: &[ResolvedColumn],
            _params: &[ResolvedParam],
        ) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_enum_def(&self, _enum_info: &scythe_core::analyzer::EnumInfo) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_composite_def(
            &self,
            _composite: &scythe_core::analyzer::CompositeInfo,
        ) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_nested_struct_def(&self, nested: &NestedStructInfo) -> Result<Option<String>, ScytheError> {
            Ok(Some(format!("struct {} {{}}", to_pascal_case(&nested.name))))
        }
    }

    #[test]
    fn test_degrade_unsupported_nested_structs_keeps_supported_column_and_collects_def() {
        let manifest = get_backend("rust-sqlx", "postgresql").unwrap().manifest().clone();
        let backend = NestedSupportingBackend { manifest };
        let nested = a_nested_struct();
        let columns = vec![AnalyzedColumn {
            name: "posts".to_string(),
            neutral_type: "json_nested<array<GetUserPostsRowPosts>>".to_string(),
            nullable: true,
            ..Default::default()
        }];

        let (degraded, defs) = degrade_unsupported_nested_structs(&columns, &[nested], &backend).unwrap();

        assert_eq!(
            degraded[0].neutral_type, "json_nested<array<GetUserPostsRowPosts>>",
            "a supported backend must leave the nested reference untouched"
        );
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "get_user_posts_row_posts");
        assert_eq!(defs[0].code, "struct GetUserPostsRowPosts {}");
    }

    /// The review check for this batch: for a backend that does not opt in,
    /// generated output for a nested-aggregate column must be byte-identical
    /// to what the same backend produces for an ordinary plain-`json` column
    /// -- proving the degradation pass, not a per-backend special case,
    /// is what keeps ~44 non-opted-in backends safe.
    #[test]
    fn test_unopted_backend_output_is_byte_identical_to_plain_json_baseline() {
        let backend = get_backend("java-jdbc", "postgresql").unwrap();

        let baseline = make_query(
            "GetUserPosts",
            QueryCommand::Many,
            "SELECT json_agg(p.*) AS posts FROM users u JOIN posts p ON u.id = p.user_id",
            vec![AnalyzedColumn {
                name: "posts".to_string(),
                neutral_type: "json".to_string(),
                nullable: true,
                ..Default::default()
            }],
            vec![],
        );

        let mut nested_query = baseline.clone();
        nested_query.columns[0].neutral_type = "json_nested<array<GetUserPostsRowPosts>>".to_string();
        nested_query.nested_structs = vec![a_nested_struct()];

        let baseline_result = generate_with_backend(&baseline, &*backend).unwrap();
        let nested_result = generate_with_backend(&nested_query, &*backend).unwrap();

        assert_eq!(
            baseline_result.row_struct, nested_result.row_struct,
            "row struct output must match byte for byte"
        );
        assert_eq!(
            baseline_result.query_fn, nested_result.query_fn,
            "query fn output must match byte for byte"
        );
        assert_eq!(baseline_result.model_struct, nested_result.model_struct);
    }

    // -----------------------------------------------------------------
    // #187: an enum or composite inside `array<...>` must be both
    // referenced *and* defined, not just referenced.
    // -----------------------------------------------------------------

    #[test]
    fn test_enum_inside_array_is_emitted_not_just_referenced() {
        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let mut query = make_query(
            "GetMany",
            QueryCommand::Many,
            "SELECT id, many_status FROM t",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "many_status".to_string(),
                    neutral_type: "array<enum::user_status>".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            vec![],
        );
        query.enums = vec![EnumInfo {
            sql_name: "user_status".to_string(),
            values: vec!["active".to_string(), "banned".to_string()],
        }];

        let result = generate_with_backend(&query, &*backend).unwrap();

        let enum_def = result
            .enum_def
            .expect("an enum reached only through array<enum::...> must still get a definition emitted");
        assert!(
            enum_def.contains("pub enum UserStatus"),
            "expected a UserStatus enum definition; got:\n{enum_def}"
        );

        let row_struct = result.row_struct.unwrap();
        assert!(
            row_struct.contains("Vec<UserStatus>"),
            "row struct must reference the array of the enum; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_composite_inside_array_is_emitted_not_just_referenced() {
        use scythe_core::analyzer::{CompositeFieldInfo, CompositeInfo};

        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let mut query = make_query(
            "GetManyAddrs",
            QueryCommand::Many,
            "SELECT id, addrs FROM t",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "addrs".to_string(),
                    neutral_type: "array<composite::address>".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            vec![],
        );
        query.composites = vec![CompositeInfo {
            sql_name: "address".to_string(),
            fields: vec![CompositeFieldInfo {
                name: "street".to_string(),
                neutral_type: "string".to_string(),
            }],
        }];

        let result = generate_with_backend(&query, &*backend).unwrap();

        let model = result
            .model_struct
            .expect("a composite reached only through array<composite::...> must still get a definition emitted");
        assert!(
            model.contains("pub struct Address"),
            "expected an Address composite definition; got:\n{model}"
        );
    }

    #[test]
    fn test_unwrap_containers() {
        assert_eq!(unwrap_containers("enum::user_status"), "enum::user_status");
        assert_eq!(unwrap_containers("array<enum::user_status>"), "enum::user_status");
        assert_eq!(unwrap_containers("array<composite::address>"), "composite::address");
        assert_eq!(
            unwrap_containers("array<nullable<enum::user_status>>"),
            "enum::user_status"
        );
        assert_eq!(unwrap_containers("nullable<array<int32>>"), "int32");
        assert_eq!(unwrap_containers("int32"), "int32");
    }

    // -----------------------------------------------------------------
    // #188: a `:grouped` parent column named `children` must not silently
    // collide with the synthesized `children` field.
    // -----------------------------------------------------------------

    #[test]
    fn test_grouped_parent_column_named_children_collides_with_synthesized_field() {
        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let parent_cols = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "children".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let child_cols = vec![AnalyzedColumn {
            name: "order_id".to_string(),
            neutral_type: "int32".to_string(),
            nullable: false,
            ..Default::default()
        }];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();

        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetUsersWithOrders".to_string();
            aq.command = QueryCommand::Grouped;
            aq.sql = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
                  SELECT u.id, u.children, o.id AS order_id\n\
                  FROM users u\n\
                  JOIN orders o ON o.user_id = u.id"
                .to_string();
            aq.columns = all_cols;
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            });
            aq.custom = vec![];
        });

        let result = generate_with_backend(&query, &*backend);
        let err = result.expect_err(
            "a parent column literally named 'children' must be rejected, not silently \
             produce two `children` fields on the generated struct",
        );
        assert_eq!(err.code, ErrorCode::DuplicateAlias);
        assert!(
            err.message.contains("children"),
            "error should name the colliding field, got: {}",
            err.message
        );
    }

    // -----------------------------------------------------------------
    // #164: `SELECT *` must declare and reference the same struct name,
    // across every registered backend -- not just the two (rust-sqlx,
    // rust-tokio-postgres) with a hand-written `generate_model_struct`.
    // -----------------------------------------------------------------

    /// Strip line comments (`//`, `#`), block comments (`/* ... */`), and
    /// triple-quoted docstrings (`"""..."""`, `'''...'''`) from generated
    /// source.
    ///
    /// Every backend's doc comment for a `SELECT *` query says something
    /// like "Row type for User query" / "Fetch all User rows" -- the *SQL*
    /// entity name in prose, which is a whole-word match for `struct_name`
    /// that has nothing to do with whether the code actually declares or
    /// references that identifier. Without stripping these first,
    /// [`contains_identifier`] finds "User" in that prose and reports a
    /// backend as correct when its actual declaration says `UserRow`.
    fn strip_comments(source: &str) -> String {
        let mut out = String::with_capacity(source.len());
        let mut i = 0;
        while i < source.len() {
            let rest = &source[i..];
            if rest.starts_with("//") || rest.starts_with('#') {
                match rest.find('\n') {
                    Some(nl) => i += nl,
                    None => i = source.len(),
                }
                continue;
            }
            if rest.starts_with("/*") {
                match rest.find("*/") {
                    Some(end) => i += end + 2,
                    None => i = source.len(),
                }
                continue;
            }
            if let Some(quote) = ["\"\"\"", "'''"].into_iter().find(|q| rest.starts_with(q)) {
                match rest[quote.len()..].find(quote) {
                    Some(end) => i += quote.len() + end + quote.len(),
                    None => i = source.len(),
                }
                continue;
            }
            let ch = rest.chars().next().expect("i < source.len()");
            out.push(ch);
            i += ch.len_utf8();
        }
        out
    }

    /// Whether `needle` appears in `haystack` as a whole identifier --
    /// bounded by a non-identifier character (or a string edge) on both
    /// sides.
    ///
    /// The 50+ backends here don't share one declaration/reference syntax
    /// (`pub struct X {`, `export interface X {`, `type X struct {`,
    /// `class X:`, `record X(...)`, `object XTable : IntIdTable(...)`, ...),
    /// so this test cannot parse each one's grammar and still be a single
    /// check "worth more than a per-backend one". What every one of them
    /// does share is spelling the struct/class/record name as one
    /// identifier token -- so a boundary-aware substring search is enough
    /// to tell "declares/references `User`" apart from "declares/references
    /// `UserRow`" without knowing any backend's actual syntax.
    fn contains_identifier(haystack: &str, needle: &str) -> bool {
        if needle.is_empty() {
            return false;
        }
        let is_ident_char = |c: char| c.is_alphanumeric() || c == '_';
        let mut start = 0;
        while let Some(rel) = haystack[start..].find(needle) {
            let idx = start + rel;
            let before_ok = haystack[..idx].chars().next_back().is_none_or(|c| !is_ident_char(c));
            let after_ok = haystack[idx + needle.len()..]
                .chars()
                .next()
                .is_none_or(|c| !is_ident_char(c));
            if before_ok && after_ok {
                return true;
            }
            start = idx + 1;
            if start >= haystack.len() {
                break;
            }
        }
        false
    }

    /// Every backend `get_backend` recognizes, keyed by its canonical
    /// (first-listed) name in `backends::get_backend`'s match arms.
    const ALL_BACKEND_NAMES: &[&str] = &[
        "rust-sqlx",
        "rust-tokio-postgres",
        "python-psycopg3",
        "python-asyncpg",
        "python-aiomysql",
        "python-aiosqlite",
        "python-duckdb",
        "typescript-postgres",
        "javascript-postgres",
        "typescript-pg",
        "javascript-pg",
        "typescript-mysql2",
        "javascript-mysql2",
        "typescript-better-sqlite3",
        "javascript-better-sqlite3",
        "typescript-duckdb",
        "typescript-node-sqlite",
        "typescript-wasm-sqlite",
        "typescript-kysely",
        "go-database-sql",
        "go-pgx",
        "java-jdbc",
        "java-r2dbc",
        "kotlin-exposed",
        "kotlin-jdbc",
        "kotlin-r2dbc",
        "csharp-npgsql",
        "csharp-mysqlconnector",
        "csharp-microsoft-sqlite",
        "elixir-postgrex",
        "elixir-ecto",
        "elixir-myxql",
        "elixir-exqlite",
        "ruby-pg",
        "ruby-mysql2",
        "ruby-sqlite3",
        "ruby-trilogy",
        "php-pdo",
        "php-amphp",
        "rust-tiberius",
        "python-pyodbc",
        "typescript-mssql",
        "csharp-sqlclient",
        "ruby-tiny-tds",
        "elixir-tds",
        "rust-sibyl",
        "python-oracledb",
        "typescript-oracledb",
        "go-godror",
        "csharp-oracle",
        "ruby-oci8",
        "elixir-jamdb",
        "python-snowflake",
        "typescript-snowflake",
        "go-gosnowflake",
        "csharp-snowflake",
    ];

    /// `get_backend(name, engine)` rejects an engine the backend's manifest
    /// doesn't cover (e.g. `go-database-sql` has no PostgreSQL manifest at
    /// all); try engines in order and use whichever one the backend
    /// actually accepts, since this test cares about naming consistency,
    /// not which engine.
    fn backend_for_select_star(name: &str) -> Box<dyn CodegenBackend> {
        const ENGINES: &[&str] = &[
            "postgresql",
            "mysql",
            "sqlite",
            "mssql",
            "mariadb",
            "duckdb",
            "oracle",
            "snowflake",
            "redshift",
        ];
        for engine in ENGINES {
            if let Ok(backend) = get_backend(name, engine) {
                return backend;
            }
        }
        panic!("no known engine works for backend '{name}'");
    }

    fn select_star_query() -> AnalyzedQuery {
        let mut query = make_query(
            "GetAllUsers",
            QueryCommand::Many,
            "SELECT * FROM users",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            vec![],
        );
        query.source_table = Some("users".to_string());
        query
    }

    /// One entry in [`KNOWN_DIVERGENT_BACKENDS`] or [`NOT_APPLICABLE_BACKENDS`]:
    /// a backend name paired with why it is listed, so the list doubles as
    /// its own documentation.
    struct BackendNote {
        backend: &'static str,
        reason: &'static str,
    }

    /// Root cause shared by every entry in [`KNOWN_DIVERGENT_BACKENDS`]
    /// except `kotlin-exposed`: `generate_model_struct`
    /// (`crates/scythe-codegen/src/backends/*.rs`) computes `let name =
    /// to_pascal_case(&singularize(table_name))` -- the correct, unsuffixed
    /// model name -- and then delegates to `self.generate_row_struct(&name,
    /// columns)`, whose contract unconditionally appends
    /// `manifest.naming.row_suffix`. That turns "User" into "UserRow" on
    /// *declaration* while `determine_struct_name` (this file, already
    /// correct) keeps the *reference* at "User". Fix: replace that
    /// delegation with an inline emission under `name` with no further
    /// suffixing, matching what `backends/sqlx.rs` and
    /// `backends/tokio_postgres.rs` already do for their own
    /// `generate_model_struct` -- the two backends that do not appear below.
    const ROW_SUFFIX_DELEGATION_BUG: &str = "generate_model_struct delegates to generate_row_struct, which appends row_suffix onto \
         the already-final, unsuffixed model name (#164)";

    /// Ratcheting allowlist for
    /// [`test_select_star_declares_and_references_the_same_struct_name_across_all_backends`],
    /// matching the pattern established by `scripts/torture-expected-failures.txt`
    /// / `scripts/check-generated-backends.py`: the test fails in *both*
    /// directions --
    /// - a backend not listed here that diverges is a regression: investigate,
    ///   don't just add a line.
    /// - a backend listed here that now agrees means this entry is stale: the
    ///   underlying defect was fixed, delete the line.
    ///
    /// No percentage, no tolerance, no `#[ignore]`: every entry names one
    /// backend and one reason, and the list is forced to shrink to empty as
    /// backends get fixed instead of silently rotting.
    const KNOWN_DIVERGENT_BACKENDS: &[BackendNote] = &[
        BackendNote {
            backend: "python-psycopg3",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "python-asyncpg",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "python-aiomysql",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "python-aiosqlite",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "python-duckdb",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-postgres",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-pg",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-mysql2",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-better-sqlite3",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-duckdb",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-node-sqlite",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-wasm-sqlite",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-kysely",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "go-database-sql",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "go-pgx",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "java-jdbc",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "java-r2dbc",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "kotlin-jdbc",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "kotlin-r2dbc",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "kotlin-exposed",
            reason: "generate_model_struct emits an Exposed `object XTable : IntIdTable(...)` in place \
                      of a row type entirely -- a different, more severe defect than the row_suffix bug \
                      shared by every other entry in this list (#214)",
        },
        BackendNote {
            backend: "csharp-npgsql",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "csharp-mysqlconnector",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "csharp-microsoft-sqlite",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "elixir-postgrex",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "elixir-ecto",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "elixir-myxql",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "elixir-exqlite",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "ruby-pg",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "ruby-mysql2",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "ruby-sqlite3",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "ruby-trilogy",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "php-pdo",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "php-amphp",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "rust-tiberius",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "python-pyodbc",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-mssql",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "csharp-sqlclient",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "ruby-tiny-tds",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "elixir-tds",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "rust-sibyl",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "python-oracledb",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-oracledb",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "go-godror",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "csharp-oracle",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "ruby-oci8",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "elixir-jamdb",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "python-snowflake",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "typescript-snowflake",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "go-gosnowflake",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
        BackendNote {
            backend: "csharp-snowflake",
            reason: ROW_SUFFIX_DELEGATION_BUG,
        },
    ];

    /// Backends this check cannot render a verdict on at all -- distinct
    /// from [`KNOWN_DIVERGENT_BACKENDS`], which lists backends the check
    /// *did* run and found broken. The four `javascript-*` backends type
    /// their generated functions entirely through JSDoc comments (see
    /// `generate_js_typedef_row_struct` / the `js_mode` branches in
    /// `backends/typescript_*.rs`): both the struct name they declare and
    /// the one they reference live only inside comment text, which
    /// [`strip_comments`] deliberately removes before matching (see its own
    /// doc comment for why -- otherwise unrelated prose like "Fetch all User
    /// rows" reads as a match). After stripping there is no runtime
    /// identifier left to check either way, so this check always reports
    /// both sides as absent regardless of whether the row_suffix bug is
    /// fixed. Excluding these four from the pass/fail logic entirely, but
    /// listing them here by name and reason, is what keeps that exclusion
    /// visible instead of reading as a quiet, unexplained pass -- the same
    /// defect shape (a skip that looks like a pass) this whole effort exists
    /// to catch.
    const NOT_APPLICABLE_BACKENDS: &[BackendNote] = &[
        BackendNote {
            backend: "javascript-postgres",
            reason: "js_mode types via JSDoc comments only; no runtime identifier for either side of \
                      the check to find once comments are stripped",
        },
        BackendNote {
            backend: "javascript-pg",
            reason: "js_mode types via JSDoc comments only; no runtime identifier for either side of \
                      the check to find once comments are stripped",
        },
        BackendNote {
            backend: "javascript-mysql2",
            reason: "js_mode types via JSDoc comments only; no runtime identifier for either side of \
                      the check to find once comments are stripped",
        },
        BackendNote {
            backend: "javascript-better-sqlite3",
            reason: "js_mode types via JSDoc comments only; no runtime identifier for either side of \
                      the check to find once comments are stripped",
        },
    ];

    /// Ratcheting guard for #164: `SELECT *` must declare and reference the
    /// same struct name. Every backend not in [`KNOWN_DIVERGENT_BACKENDS`]
    /// or [`NOT_APPLICABLE_BACKENDS`] must agree; every backend in
    /// `KNOWN_DIVERGENT_BACKENDS` must currently disagree (a stale entry
    /// fails the test exactly as loudly as a regression does -- see that
    /// const's doc comment). This makes the list impossible to forget about:
    /// it can only shrink, never quietly rot.
    #[test]
    fn test_select_star_declares_and_references_the_same_struct_name_across_all_backends() {
        let query = select_star_query();

        let mut regressions = Vec::new();
        let mut stale_entries = Vec::new();
        let mut seen_not_applicable: Vec<&str> = Vec::new();

        for &name in ALL_BACKEND_NAMES {
            if let Some(note) = NOT_APPLICABLE_BACKENDS.iter().find(|n| n.backend == name) {
                seen_not_applicable.push(note.backend);
                continue;
            }

            let backend = backend_for_select_star(name);
            let struct_name = determine_struct_name(&query, backend.manifest());

            let agrees = match generate_with_backend(&query, &*backend) {
                Err(_) => false,
                Ok(result) => match (result.model_struct, result.query_fn) {
                    (Some(model_struct), Some(query_fn)) => {
                        let declared = contains_identifier(&strip_comments(&model_struct), &struct_name);
                        let referenced = contains_identifier(&strip_comments(&query_fn), &struct_name);
                        declared && referenced
                    }
                    _ => false,
                },
            };

            let listed = KNOWN_DIVERGENT_BACKENDS.iter().find(|n| n.backend == name);
            match (agrees, listed) {
                (true, Some(_)) => stale_entries.push(name),
                (false, None) => regressions.push(name),
                // Confirmed still divergent for the reason on file -- print
                // it (visible with `--nocapture`) so `reason` is more than
                // documentation nobody's code path ever reads.
                (false, Some(note)) => eprintln!("expected divergence, {name}: {}", note.reason),
                (true, None) => {}
            }
        }

        // NOT_APPLICABLE_BACKENDS naming a backend ALL_BACKEND_NAMES doesn't
        // know about would silently exclude nothing (the `if let` above
        // never matches it) -- catch that the same way a stale
        // KNOWN_DIVERGENT_BACKENDS entry is caught, rather than let the
        // exclusion quietly fail to apply.
        let unknown_not_applicable: Vec<&str> = NOT_APPLICABLE_BACKENDS
            .iter()
            .map(|n| n.backend)
            .filter(|b| !seen_not_applicable.contains(b))
            .collect();

        let mut failures = Vec::new();
        for name in &regressions {
            let reason = "diverges on SELECT * struct naming and is not in KNOWN_DIVERGENT_BACKENDS";
            failures.push(format!(
                "REGRESSION: {name}: {reason} -- investigate before adding a line"
            ));
        }
        for name in &stale_entries {
            failures.push(format!(
                "STALE ALLOWLIST: {name}: listed in KNOWN_DIVERGENT_BACKENDS but now agrees -- delete its entry"
            ));
        }
        for name in &unknown_not_applicable {
            failures.push(format!(
                "STALE ALLOWLIST: {name}: listed in NOT_APPLICABLE_BACKENDS but is not in ALL_BACKEND_NAMES"
            ));
        }

        assert!(
            failures.is_empty(),
            "{} of {} backends need attention:\n{}",
            failures.len(),
            ALL_BACKEND_NAMES.len(),
            failures.join("\n")
        );
    }
}
