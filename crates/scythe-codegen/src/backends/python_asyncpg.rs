use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_snake_case};
use scythe_backend::types::resolve_type;
use std::collections::HashMap;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

fn asyncpg_param_expr(param: &ResolvedParam, raw: &str) -> String {
    if param.neutral_type.starts_with("composite::") {
        format!("None if {raw} is None else {raw}._to_record()")
    } else {
        raw.to_string()
    }
}

use super::python_common::{PythonRowType, no_rows_exception_def, type_support_imports, write_missing_row_guard};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/python-asyncpg.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/python-asyncpg.redshift.toml");

/// The Python expression converting one composite field's already-decoded `asyncpg.Record`
/// value (`raw`, e.g. `record["street"]`) into the field's declared Python type.
///
/// Unlike psycopg3 (board #204's `python_psycopg3.rs`), asyncpg needs no hand-written text
/// parser here -- verified by reading the vendored driver's own decode path, not from memory
/// (`asyncpg/protocol/codecs/base.pyx`): `DataCodecConfig.add_types` runs automatically off
/// introspection results and, for every `kind == b'c'` (composite) type touched by a query,
/// registers a per-column binary codec via `Codec.new_composite_codec`; `decode_composite`
/// then builds a real `self.record_desc.make_record(asyncpg.Record, ...)` keyed by the
/// composite's own attribute names, decoding each sub-field through *its own* element codec
/// (native `int`/`Decimal`/`datetime.datetime`/`uuid.UUID`/...) and mapping a NULL sub-field
/// (`elem_len == -1`) straight to `None` -- no text escaping to reimplement. An `enum` (`kind
/// == b'e'`) is registered as a plain `TEXTOID` scalar codec ("Enum types are essentially
/// text"), so an enum sub-field still decodes to `str` and needs `T(value)` to become our
/// generated `Enum` subclass; a composite sub-field only needs wrapping into its own generated
/// class, recursing through that class's own `_from_record`.
fn asyncpg_composite_field_from_record(neutral_type: &str, field_type: &str, raw: &str) -> String {
    if neutral_type.starts_with("composite::") {
        return format!("{field_type}._from_record({raw})");
    }
    if neutral_type.starts_with("enum::") {
        return format!("{field_type}({raw})");
    }
    raw.to_string()
}

/// Build the `field=value` expression for one column read from an `asyncpg.Record`
/// (`row`/`r`, both keyed by column name).
///
/// A `composite::` column arrives as asyncpg's own `Record` (see
/// [`asyncpg_composite_field_from_record`]'s doc comment), not our generated
/// dataclass/BaseModel/Struct, so it is routed through that class's `_from_record`
/// classmethod -- null-safe on its own, so no extra guard is needed regardless of
/// `col.nullable`. An `enum::` column decodes to a plain `str`; the generated `Enum`
/// subclass's `str, Enum` base makes `T(value)` the value-lookup constructor, guarded by a
/// `None` check for a nullable column since `T(None)` raises `ValueError` rather than
/// returning `None`.
fn field_assignment_expr(col: &ResolvedColumn, var: &str) -> String {
    let raw = format!("{var}[\"{}\"]", col.name);
    if col.neutral_type.starts_with("composite::") {
        return format!("{}={}._from_record({raw})", col.field_name, col.lang_type);
    }
    if col.neutral_type.starts_with("enum::") {
        return if col.nullable {
            format!("{}=None if {raw} is None else {}({raw})", col.field_name, col.lang_type)
        } else {
            format!("{}={}({raw})", col.field_name, col.lang_type)
        };
    }
    format!("{}={raw}", col.field_name)
}

pub struct PythonAsyncpgBackend {
    manifest: BackendManifest,
    row_type: PythonRowType,
}

impl PythonAsyncpgBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_TOML,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!(
                        "python-asyncpg only supports PostgreSQL/Redshift, got engine '{}'",
                        engine
                    ),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self {
            manifest,
            row_type: PythonRowType::default(),
        })
    }
}

impl CodegenBackend for PythonAsyncpgBackend {
    fn name(&self) -> &str {
        "python-asyncpg"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "redshift"]
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["row_type"], options)?;

        if let Some(rt) = options.get("row_type") {
            self.row_type = PythonRowType::from_option(rt)?;
        }
        Ok(())
    }

    fn file_header(&self) -> String {
        let import_line = self.row_type.import_line();
        let (needs_uuid, needs_any) = type_support_imports(&self.manifest);
        let uuid_line = if needs_uuid { "import uuid  # noqa: F401\n" } else { "" };
        let any_line = if needs_any {
            "from typing import Any  # noqa: F401\n"
        } else {
            ""
        };
        let header = if self.row_type.is_stdlib_import() {
            format!(
                "import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 {uuid_line}\
                 {import_line}\n\
                 from enum import Enum  # noqa: F401\n\
                 {any_line}\
                 \n\
                 from asyncpg import Connection  # noqa: F401\n\
                 \n",
            )
        } else {
            let third_party = self
                .row_type
                .sorted_third_party_imports("from asyncpg import Connection  # noqa: F401");
            format!(
                "import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 {uuid_line}\
                 from enum import Enum  # noqa: F401\n\
                 {any_line}\
                 \n\
                 {third_party}\n\
                 \n",
            )
        };
        format!("{header}{}", no_rows_exception_def())
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();
        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(struct_name));
        let _ = writeln!(out, "    \"\"\"Row type for {} query.\"\"\"", query_name);
        if columns.is_empty() {
            let _ = writeln!(out, "    pass");
        } else {
            let _ = writeln!(out);
            for col in columns {
                let _ = writeln!(out, "    {}: {}", col.field_name, col.full_type);
            }
        }
        Ok(out)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let kw_sep = if param_list.is_empty() { "" } else { ", *, " };

        let sql = crate::sql_literal::escape_python_triple_double(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let is_one = matches!(analyzed.command, QueryCommand::One);
                let ret_type = if is_one {
                    struct_name.to_string()
                } else {
                    format!("{struct_name} | None")
                };
                let _ = writeln!(
                    out,
                    "async def {}(conn: Connection{}{}) -> {}:",
                    func_name, kw_sep, param_list, ret_type
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    row = await conn.fetchrow(");
                let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|p| asyncpg_param_expr(p, &p.field_name)).collect();
                    let _ = writeln!(out, "        {},", args.join(", "));
                }
                let _ = writeln!(out, "    )");
                write_missing_row_guard(&mut out, "    ", "row", is_one, &analyzed.name);
                let field_assignments: Vec<String> =
                    columns.iter().map(|col| field_assignment_expr(col, "row")).collect();
                let oneliner = format!("    return {}({})", struct_name, field_assignments.join(", "));
                if oneliner.len() <= 88 {
                    let _ = writeln!(out, "{}", oneliner);
                } else {
                    let _ = writeln!(out, "    return {}(", struct_name);
                    for fa in &field_assignments {
                        let _ = writeln!(out, "        {},", fa);
                    }
                    let _ = writeln!(out, "    )");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "async def {}(conn: Connection{}{}) -> list[{}]:",
                    func_name, kw_sep, param_list, struct_name
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    rows = await conn.fetch(");
                let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|p| asyncpg_param_expr(p, &p.field_name)).collect();
                    let _ = writeln!(out, "        {},", args.join(", "));
                }
                let _ = writeln!(out, "    )");
                let field_assignments: Vec<String> =
                    columns.iter().map(|col| field_assignment_expr(col, "r")).collect();
                let oneliner = format!(
                    "    return [{}({}) for r in rows]",
                    struct_name,
                    field_assignments.join(", ")
                );
                if oneliner.len() <= 88 {
                    let _ = writeln!(out, "{}", oneliner);
                } else {
                    let _ = writeln!(out, "    return [");
                    let _ = writeln!(out, "        {}(", struct_name);
                    for fa in &field_assignments {
                        let _ = writeln!(out, "            {},", fa);
                    }
                    let _ = writeln!(out, "        )");
                    let _ = writeln!(out, "        for r in rows");
                    let _ = writeln!(out, "    ]");
                }
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let items_type = if params.len() > 1 {
                    let tuple_types: Vec<String> = params.iter().map(|p| p.full_type.clone()).collect();
                    format!("list[tuple[{}]]", tuple_types.join(", "))
                } else if params.len() == 1 {
                    format!("list[{}]", params[0].full_type)
                } else {
                    "int".to_string()
                };
                let param_name = if params.is_empty() { "count" } else { "items" };
                let _ = writeln!(
                    out,
                    "async def {}(conn: Connection, *, {}: {}) -> None:",
                    batch_fn_name, param_name, items_type
                );
                let _ = writeln!(
                    out,
                    "    \"\"\"Execute {} query for each item in the batch.\"\"\"",
                    analyzed.name
                );
                if params.is_empty() {
                    let _ = writeln!(out, "    for _ in range(count):");
                    let _ = writeln!(out, "        await conn.execute(");
                    let _ = writeln!(out, "            \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        )");
                } else {
                    if params.len() == 1 {
                        let item = asyncpg_param_expr(&params[0], "item");
                        let _ = writeln!(out, "    args = [({},) for item in items]", item);
                    } else {
                        let item_fields = params
                            .iter()
                            .enumerate()
                            .map(|(index, param)| asyncpg_param_expr(param, &format!("item[{index}]")))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let _ = writeln!(out, "    args = [({},) for item in items]", item_fields);
                    }
                    let _ = writeln!(out, "    await conn.executemany(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        args,");
                    let _ = writeln!(out, "    )");
                }
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "async def {}(conn: Connection{}{}) -> None:",
                    func_name, kw_sep, param_list
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    await conn.execute(");
                let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|p| asyncpg_param_expr(p, &p.field_name)).collect();
                    let _ = writeln!(out, "        {},", args.join(", "));
                }
                let _ = writeln!(out, "    )");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "async def {}(conn: Connection{}{}) -> int:",
                    func_name, kw_sep, param_list
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    result = await conn.execute(");
                let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|p| asyncpg_param_expr(p, &p.field_name)).collect();
                    let _ = writeln!(out, "        {},", args.join(", "));
                }
                let _ = writeln!(out, "    )");
                let _ = writeln!(out, "    return int(result.split()[-1])");
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        }

        Ok(out)
    }

    fn generate_grouped_structs(
        &self,
        parent_struct_name: &str,
        child_struct_name: &str,
        parent_columns: &[crate::backend_trait::ResolvedColumn],
        child_columns: &[crate::backend_trait::ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(child_struct_name));
        let _ = writeln!(out, "    \"\"\"Child row type for grouped query.\"\"\"");
        if child_columns.is_empty() {
            let _ = writeln!(out, "    pass");
        } else {
            let _ = writeln!(out);
            for col in child_columns {
                let _ = writeln!(out, "    {}: {}", col.field_name, col.full_type);
            }
        }

        let _ = writeln!(out);

        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(parent_struct_name));
        let _ = writeln!(out, "    \"\"\"Parent row type for grouped query.\"\"\"");
        let _ = writeln!(out);
        for col in parent_columns {
            let _ = writeln!(out, "    {}: {}", col.field_name, col.full_type);
        }
        let _ = writeln!(out, "    children: list[{child_struct_name}]");

        Ok(out)
    }

    fn generate_grouped_query_fn(
        &self,
        request: &crate::backend_trait::GroupedQueryFn<'_>,
    ) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let key_field = to_snake_case(key_column);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let kw_sep = if param_list.is_empty() { "" } else { ", *, " };

        let sql = crate::sql_literal::escape_python_triple_double(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let _ = writeln!(
            out,
            "async def {func_name}(conn: Connection{kw_sep}{param_list}) -> list[{parent_struct_name}]:"
        );
        let _ = writeln!(out, "    \"\"\"Execute {} grouped query.\"\"\"", analyzed.name);
        let _ = writeln!(out, "    rows = await conn.fetch(");
        let _ = writeln!(out, "        \"\"\"{sql}\"\"\",");
        if !params.is_empty() {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let _ = writeln!(out, "        {},", args.join(", "));
        }
        let _ = writeln!(out, "    )");

        let _ = writeln!(out, "    _index: dict = {{}}");
        let _ = writeln!(out, "    _entries: list = []");
        let _ = writeln!(out, "    for row in rows:");
        let _ = writeln!(out, "        key = row[\"{key_field}\"]");
        let _ = writeln!(out, "        if key not in _index:");
        let _ = writeln!(out, "            _index[key] = len(_entries)");

        let _ = writeln!(out, "            _entries.append((");
        let _ = writeln!(out, "                {{");
        for col in parent_columns {
            let raw = format!("row[\"{}\"]", col.name);
            let value_expr = if col.neutral_type.starts_with("composite::") {
                format!("{}._from_record({raw})", col.lang_type)
            } else if col.neutral_type.starts_with("enum::") {
                if col.nullable {
                    format!("None if {raw} is None else {}({raw})", col.lang_type)
                } else {
                    format!("{}({raw})", col.lang_type)
                }
            } else {
                raw
            };
            let _ = writeln!(out, "                    \"{}\": {value_expr},", col.field_name);
        }
        let _ = writeln!(out, "                }},");
        let _ = writeln!(out, "                [],");
        let _ = writeln!(out, "            ))");

        let _ = writeln!(out, "        _entries[_index[key]][1].append({child_struct_name}(");
        for col in child_columns {
            let _ = writeln!(out, "            {},", field_assignment_expr(col, "row"));
        }
        let _ = writeln!(out, "        ))");

        let _ = writeln!(out, "    return [");
        let _ = writeln!(out, "        {parent_struct_name}(**parent_kwargs, children=children)");
        let _ = writeln!(out, "        for parent_kwargs, children in _entries");
        let _ = write!(out, "    ]");

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "class {}(str, Enum):", type_name);
        let _ = writeln!(out, "    \"\"\"Database enum type {}.\"\"\"", enum_info.sql_name);
        if enum_info.values.is_empty() {
            let _ = writeln!(out, "    pass");
        } else {
            let _ = writeln!(out);
            for value in &enum_info.values {
                let variant = enum_variant_name(value, &self.manifest.naming);
                let _ = writeln!(out, "    {} = \"{}\"", variant, value);
            }
        }
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(&name));
        let _ = writeln!(out, "    \"\"\"Composite type {}.\"\"\"", composite.sql_name);
        // ~keep board #204: a composite with zero fields cannot exist in PostgreSQL
        // (`CREATE TYPE ... AS ()` is rejected), so there is no reachable runtime value
        // that would need `_from_record` here. Left as the bare stub it always was.
        if composite.fields.is_empty() {
            let _ = writeln!(out, "    pass");
            return Ok(out);
        }
        let _ = writeln!(out);
        let mut field_types: Vec<String> = Vec::with_capacity(composite.fields.len());
        for field in &composite.fields {
            let py_type = resolve_type(&field.neutral_type, &self.manifest, false)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                })?;
            let _ = writeln!(out, "    {}: {}", to_snake_case(&field.name), py_type);
            field_types.push(py_type);
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "    @classmethod");
        // ~keep `Any`, not `asyncpg.Record`: the generated module does not import asyncpg (the
        // connection arrives as a parameter), and pyrefly rejects a bare unannotated parameter
        // outright -- the "Validate: Generated Type Checking" job holds every python backend to
        // zero errors.
        let _ = writeln!(out, "    def _from_record(cls, record: Any) -> \"{} | None\":", name);
        let _ = writeln!(
            out,
            "        \"\"\"~keep board #204: asyncpg decodes a composite column to its own"
        );
        let _ = writeln!(
            out,
            "        `Record` (tuple-like, not our declared type) -- wrap it into this class.\"\"\""
        );
        let _ = writeln!(out, "        if record is None:");
        let _ = writeln!(out, "            return None");
        let _ = writeln!(out, "        return cls(");
        for (field, field_type) in composite.fields.iter().zip(&field_types) {
            let raw = format!("record[\"{}\"]", field.name);
            let value_expr = asyncpg_composite_field_from_record(&field.neutral_type, field_type, &raw);
            let _ = writeln!(out, "            {}={value_expr},", to_snake_case(&field.name));
        }
        let _ = writeln!(out, "        )");
        let encoded_fields = composite
            .fields
            .iter()
            .map(|field| {
                let field_name = to_snake_case(&field.name);
                if field.neutral_type.starts_with("composite::") {
                    format!("None if self.{field_name} is None else self.{field_name}._to_record()")
                } else {
                    format!("self.{field_name}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let tuple_suffix = if composite.fields.len() == 1 { "," } else { "" };
        let _ = writeln!(out);
        let _ = writeln!(out, "    def _to_record(self) -> tuple[Any, ...]:");
        let _ = writeln!(out, "        return ({encoded_fields}{tuple_suffix})");
        Ok(out)
    }
}
