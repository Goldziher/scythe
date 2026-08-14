use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, to_pascal_case};
use std::collections::HashMap;
use std::fmt::Write;

use scythe_core::SqlDialect;
use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::backends::php_common::{
    RECORD_NOT_FOUND_EXCEPTION_CLASS, param_docblock_type, record_not_found_exception_class_def,
    write_promoted_property,
};

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/php-amphp.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/php-amphp.mysql.toml");

pub struct PhpAmphpBackend {
    manifest: BackendManifest,
    namespace: String,
    /// Canonical engine string, kept (like `java-jdbc`/`kotlin-jdbc`) so
    /// `generate_query_fn`/`generate_grouped_query_fn` can resolve the
    /// [`SqlDialect`] the SQL-text pipeline needs (board #148) -- `new` only had
    /// this as a local before.
    engine: String,
}

impl PhpAmphpBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" | "mariadb" => DEFAULT_MANIFEST_MYSQL,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("unsupported engine '{}' for php-amphp backend", engine),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self {
            manifest,
            namespace: "App\\Generated".to_string(),
            engine: engine.to_string(),
        })
    }
}

/// Build the PHP expression that converts a raw AMPHP column value to the
/// type the manifest declares for it.
///
/// Keyed on `lang_type` -- the manifest's own declaration for this column --
/// rather than on `neutral_type`. The previous table matched on
/// `neutral_type` and hardcoded `json` and `decimal` into the `(string) `
/// arm, which held only by coincidence: `php-amphp.toml` declares
/// `json = "array"` while `php-amphp.mysql.toml` declares `json = "string"`.
/// A cast keyed on the neutral type alone cannot see that difference -- it
/// produced a `(string) ` cast against a property the same manifest had just
/// declared `array`. See #198.
///
/// Container types (`array<...>`, `range<...>`, `json_typed<...>`) resolve
/// `lang_type` to the same base string a scalar could (a bare PostgreSQL
/// array column and the `json` scalar both resolve to `"array"`), so they are
/// excluded up front by `neutral_type` rather than by `lang_type`: nothing
/// here parses a PostgreSQL array literal or unpacks a manifest-declared
/// struct, so no cast or decode of a container column can be correct in
/// general, and guessing from `lang_type` alone would decode a real array
/// column's `{a,b,c}` literal as JSON and silently get back `null`.
fn php_convert_column(neutral_type: &str, lang_type: &str, value_expr: &str) -> String {
    if neutral_type.contains('<') {
        return value_expr.to_string();
    }
    match lang_type {
        "int" => format!("(int) {value_expr}"),
        "float" => format!("(float) {value_expr}"),
        "bool" => format!("(bool) {value_expr}"),
        "string" => format!("(string) {value_expr}"),
        "array" => format!("json_decode({value_expr}, true)"),
        _ => value_expr.to_string(),
    }
}

/// Return the PHP value expression for decoding a single column from `$row`.
///
/// Used when building the `$parentArgs` dictionary in the grouped-query fold so
/// that parent field values are decoded with the same casts and enum/datetime
/// conversions as the regular `fromRow` factory.
fn php_row_expr(c: &ResolvedColumn) -> String {
    let is_enum = c.neutral_type.starts_with("enum::");
    let is_datetime = matches!(
        c.neutral_type.as_str(),
        "date" | "time" | "time_tz" | "datetime" | "datetime_tz"
    );
    if is_enum {
        if c.nullable {
            format!(
                "$row['{}'] !== null ? {}::from($row['{}']) : null",
                c.name, c.lang_type, c.name
            )
        } else {
            format!("{}::from($row['{}'])", c.lang_type, c.name)
        }
    } else if is_datetime {
        if c.nullable {
            format!(
                "$row['{}'] !== null ? new \\DateTimeImmutable($row['{}']) : null",
                c.name, c.name
            )
        } else {
            format!("new \\DateTimeImmutable($row['{}'])", c.name)
        }
    } else {
        let value_expr = format!("$row['{}']", c.name);
        let converted = php_convert_column(&c.neutral_type, &c.lang_type, &value_expr);
        if c.nullable {
            format!("$row['{}'] !== null ? {} : null", c.name, converted)
        } else {
            converted
        }
    }
}

/// Write a `fromRow(array $row): self` static factory method for the given columns.
///
/// The output is byte-identical to what [`PhpAmphpBackend::generate_row_struct`] emits
/// for the same columns, keeping child struct `fromRow` consistent with regular row structs.
fn write_php_from_row_method(out: &mut String, columns: &[ResolvedColumn]) {
    let _ = writeln!(out, "    public static function fromRow(array $row): self {{");
    let _ = writeln!(out, "        return new self(");
    for c in columns.iter() {
        let sep = ",";
        let is_enum = c.neutral_type.starts_with("enum::");
        let is_datetime = matches!(
            c.neutral_type.as_str(),
            "date" | "time" | "time_tz" | "datetime" | "datetime_tz"
        );
        if is_enum {
            let enum_type = &c.lang_type;
            if c.nullable {
                let _ = writeln!(
                    out,
                    "            {}: $row['{}'] !== null ? {}::from($row['{}']) : null{}",
                    c.field_name, c.name, enum_type, c.name, sep
                );
            } else {
                let _ = writeln!(
                    out,
                    "            {}: {}::from($row['{}']){}",
                    c.field_name, enum_type, c.name, sep
                );
            }
        } else if is_datetime {
            if c.nullable {
                let _ = writeln!(
                    out,
                    "            {}: $row['{}'] !== null ? new \\DateTimeImmutable($row['{}']) : null{}",
                    c.field_name, c.name, c.name, sep
                );
            } else {
                let _ = writeln!(
                    out,
                    "            {}: new \\DateTimeImmutable($row['{}']){}",
                    c.field_name, c.name, sep
                );
            }
        } else {
            let value_expr = format!("$row['{}']", c.name);
            let converted = php_convert_column(&c.neutral_type, &c.lang_type, &value_expr);
            if c.nullable {
                let _ = writeln!(
                    out,
                    "            {}: $row['{}'] !== null ? {} : null{}",
                    c.field_name, c.name, converted, sep
                );
            } else {
                let _ = writeln!(out, "            {}: {}{}", c.field_name, converted, sep);
            }
        }
    }
    let _ = writeln!(out, "        );");
    let _ = writeln!(out, "    }}");
}

impl CodegenBackend for PhpAmphpBackend {
    fn name(&self) -> &str {
        "php-amphp"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb"]
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["namespace"], options)?;

        if let Some(ns) = options.get("namespace") {
            self.namespace = ns.clone();
        }
        Ok(())
    }

    fn file_preamble(&self) -> String {
        "<?php\n".to_string()
    }

    fn file_header(&self) -> String {
        let ns = if self.namespace.is_empty() {
            String::new()
        } else {
            format!("namespace {};\n\n", self.namespace)
        };
        format!(
            "declare(strict_types=1);\n\n{ns}{}",
            record_not_found_exception_class_def()
        )
    }

    fn query_class_header(&self) -> String {
        "final class Queries {".to_string()
    }

    fn file_footer(&self) -> String {
        "}".to_string()
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let _ = writeln!(out, "readonly class {} {{", struct_name);
        let _ = writeln!(out, "    public function __construct(");
        for c in columns.iter() {
            write_promoted_property(&mut out, c, &self.manifest)?;
        }
        let _ = writeln!(out, "    ) {{}}");
        let _ = writeln!(out);

        let _ = writeln!(out, "    public static function fromRow(array $row): self {{");
        let _ = writeln!(out, "        return new self(");
        for c in columns.iter() {
            let sep = ",";
            let is_enum = c.neutral_type.starts_with("enum::");
            let is_datetime = matches!(
                c.neutral_type.as_str(),
                "date" | "time" | "time_tz" | "datetime" | "datetime_tz"
            );
            if is_enum {
                let enum_type = &c.lang_type;
                if c.nullable {
                    let _ = writeln!(
                        out,
                        "            {}: $row['{}'] !== null ? {}::from($row['{}']) : null{}",
                        c.field_name, c.name, enum_type, c.name, sep
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "            {}: {}::from($row['{}']){}",
                        c.field_name, enum_type, c.name, sep
                    );
                }
            } else if is_datetime {
                if c.nullable {
                    let _ = writeln!(
                        out,
                        "            {}: $row['{}'] !== null ? new \\DateTimeImmutable($row['{}']) : null{}",
                        c.field_name, c.name, c.name, sep
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "            {}: new \\DateTimeImmutable($row['{}']){}",
                        c.field_name, c.name, sep
                    );
                }
            } else {
                let value_expr = format!("$row['{}']", c.name);
                let converted = php_convert_column(&c.neutral_type, &c.lang_type, &value_expr);
                if c.nullable {
                    let _ = writeln!(
                        out,
                        "            {}: $row['{}'] !== null ? {} : null{}",
                        c.field_name, c.name, converted, sep
                    );
                } else {
                    let _ = writeln!(out, "            {}: {}{}", c.field_name, converted, sep);
                }
            }
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let dialect = SqlDialect::from_str(&self.engine).unwrap_or_default();
        let cleaned_sql = super::clean_sql_oneline_with_optional_dialect(
            &analyzed.sql,
            dialect,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let (rewritten_sql, occurrences) =
            super::rewrite_placeholders_indexed(&cleaned_sql, dialect, |_| "?".to_string());
        let sql = crate::sql_literal::escape_php_single_quoted(&rewritten_sql);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{} ${}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        if matches!(analyzed.command, QueryCommand::Batch) {
            let batch_fn_name = format!("{}Batch", func_name);
            let _ = writeln!(out, "    /**");
            let _ = writeln!(out, "     * @param \\Amp\\Sql\\SqlConnectionPool $pool");
            let _ = writeln!(out, "     * @param array<int, array<int, mixed>> $items");
            let _ = writeln!(out, "     * @return void");
            let _ = writeln!(out, "     */");
            let _ = writeln!(
                out,
                "    public static function {}(\\Amp\\Sql\\SqlConnectionPool $pool, array $items): void {{",
                batch_fn_name
            );
            let _ = writeln!(out, "        $transaction = $pool->beginTransaction();");
            let _ = writeln!(out, "        try {{");
            let _ = writeln!(out, "            $stmt = $transaction->prepare('{}');", sql);
            let _ = writeln!(out, "            foreach ($items as $item) {{");
            if params.is_empty() {
                let _ = writeln!(out, "                $stmt->execute([]);");
            } else {
                let _ = writeln!(out, "                $stmt->execute($item);");
            }
            let _ = writeln!(out, "            }}");
            let _ = writeln!(out, "            $transaction->commit();");
            let _ = writeln!(out, "        }} catch (\\Throwable $e) {{");
            let _ = writeln!(out, "            $transaction->rollback();");
            let _ = writeln!(out, "            throw $e;");
            let _ = writeln!(out, "        }}");
            let _ = write!(out, "    }}");
            return Ok(out);
        }

        let return_type = match &analyzed.command {
            // `:one` throws `RecordNotFoundException` instead of returning `null` on a
            // missing row (see the body match below), so its declared return type is
            // non-nullable -- see `php_pdo.rs`'s identical arm for why.
            QueryCommand::One => struct_name.to_string(),
            QueryCommand::Opt => format!("?{}", struct_name),
            QueryCommand::Many => "\\Generator".to_string(),
            QueryCommand::Exec => "void".to_string(),
            QueryCommand::ExecResult | QueryCommand::ExecRows => "int".to_string(),
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        };

        let _ = writeln!(out, "    /**");
        let _ = writeln!(out, "     * @param \\Amp\\Sql\\SqlConnectionPool $pool");
        for p in params {
            let _ = writeln!(
                out,
                "     * @param {} ${}",
                param_docblock_type(p, &self.manifest)?,
                p.field_name
            );
        }
        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "     * @return {}", struct_name);
                let _ = writeln!(out, "     * @throws {}", RECORD_NOT_FOUND_EXCEPTION_CLASS);
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "     * @return {}|null", struct_name);
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "     * @return \\Generator<int, {}, mixed, void>", struct_name);
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "     * @return void");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "     * @return int");
            }
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        }
        let _ = writeln!(out, "     */");

        let _ = writeln!(
            out,
            "    public static function {}(\\Amp\\Sql\\SqlConnectionPool $pool{}{}): {} {{",
            func_name, sep, param_list, return_type
        );

        // NOTE: This prepares the statement on every call for simplicity.
        if occurrences.is_empty() {
            let _ = writeln!(out, "        $result = $pool->prepare('{}')->execute([]);", sql);
        } else {
            let bindings = occurrences
                .iter()
                .map(|&position| {
                    let p = super::resolved_param_for_position(&analyzed.params, params, position);
                    if p.neutral_type.starts_with("enum::") {
                        format!("${}->value", p.field_name)
                    } else {
                        format!("${}", p.field_name)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "        $result = $pool->prepare('{}')->execute([{}]);",
                sql, bindings
            );
        }

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "        foreach ($result as $row) {{");
                let _ = writeln!(out, "            return {}::fromRow($row);", struct_name);
                let _ = writeln!(out, "        }}");
                let _ = writeln!(
                    out,
                    "        throw new {}('{}: no row found');",
                    RECORD_NOT_FOUND_EXCEPTION_CLASS, func_name
                );
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "        foreach ($result as $row) {{");
                let _ = writeln!(out, "            return {}::fromRow($row);", struct_name);
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "        return null;");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "        foreach ($result as $row) {{");
                let _ = writeln!(out, "            yield {}::fromRow($row);", struct_name);
                let _ = writeln!(out, "        }}");
            }
            QueryCommand::Exec => {}
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "        return $result->getRowCount();");
            }
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        }

        let _ = write!(out, "    }}");
        Ok(out)
    }

    fn generate_grouped_structs(
        &self,
        parent_struct_name: &str,
        child_struct_name: &str,
        parent_columns: &[ResolvedColumn],
        child_columns: &[ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let _ = writeln!(out, "readonly class {} {{", child_struct_name);
        let _ = writeln!(out, "    public function __construct(");
        for c in child_columns.iter() {
            write_promoted_property(&mut out, c, &self.manifest)?;
        }
        let _ = writeln!(out, "    ) {{}}");
        let _ = writeln!(out);
        write_php_from_row_method(&mut out, child_columns);
        let _ = write!(out, "}}");

        let _ = writeln!(out);
        let _ = writeln!(out);

        let _ = writeln!(out, "readonly class {} {{", parent_struct_name);
        let _ = writeln!(out, "    public function __construct(");
        for c in parent_columns.iter() {
            write_promoted_property(&mut out, c, &self.manifest)?;
        }
        let _ = writeln!(out, "        /** @var {}[] */", child_struct_name);
        let _ = writeln!(out, "        public array $children,");
        let _ = writeln!(out, "    ) {{}}");
        let _ = write!(out, "}}");

        Ok(out)
    }

    fn generate_grouped_query_fn(&self, request: &GroupedQueryFn<'_>) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let parent_columns = request.parent_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let dialect = SqlDialect::from_str(&self.engine).unwrap_or_default();
        let cleaned_sql = super::clean_sql_oneline_with_optional_dialect(
            &analyzed.sql,
            dialect,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let (rewritten_sql, occurrences) =
            super::rewrite_placeholders_indexed(&cleaned_sql, dialect, |_| "?".to_string());
        let sql = crate::sql_literal::escape_php_single_quoted(&rewritten_sql);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{} ${}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let _ = writeln!(out, "    /**");
        let _ = writeln!(out, "     * @param \\Amp\\Sql\\SqlConnectionPool $pool");
        for p in params {
            let _ = writeln!(
                out,
                "     * @param {} ${}",
                param_docblock_type(p, &self.manifest)?,
                p.field_name
            );
        }
        let _ = writeln!(out, "     * @return {}[]", parent_struct_name);
        let _ = writeln!(out, "     */");

        let _ = writeln!(
            out,
            "    public static function {}(\\Amp\\Sql\\SqlConnectionPool $pool{}{}): array {{",
            func_name, sep, param_list
        );

        if occurrences.is_empty() {
            let _ = writeln!(out, "        $resultSet = $pool->prepare('{}')->execute([]);", sql);
        } else {
            let bindings = occurrences
                .iter()
                .map(|&position| {
                    let p = super::resolved_param_for_position(&analyzed.params, params, position);
                    if p.neutral_type.starts_with("enum::") {
                        format!("${}->value", p.field_name)
                    } else {
                        format!("${}", p.field_name)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "        $resultSet = $pool->prepare('{}')->execute([{}]);",
                sql, bindings
            );
        }

        let _ = writeln!(out, "        /** @var array<int|string, int> $parentIndex */");
        let _ = writeln!(out, "        $parentIndex = [];");
        let _ = writeln!(out, "        /** @var array<int, array<string, mixed>> $parentArgs */");
        let _ = writeln!(out, "        $parentArgs = [];");
        let _ = writeln!(
            out,
            "        /** @var array<int, {}[]> $childrenMap */",
            child_struct_name
        );
        let _ = writeln!(out, "        $childrenMap = [];");

        let _ = writeln!(out, "        foreach ($resultSet as $row) {{");
        let _ = writeln!(out, "            $key = $row['{}'];", key_column);
        let _ = writeln!(out, "            if (!isset($parentIndex[$key])) {{");
        let _ = writeln!(out, "                $pos = count($parentArgs);");
        let _ = writeln!(out, "                $parentIndex[$key] = $pos;");
        let _ = writeln!(out, "                $parentArgs[] = [");
        for c in parent_columns {
            let expr = php_row_expr(c);
            let _ = writeln!(out, "                    '{}' => {},", c.field_name, expr);
        }
        let _ = writeln!(out, "                ];");
        let _ = writeln!(out, "                $childrenMap[] = [];");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "            $pos = $parentIndex[$key];");
        let _ = writeln!(
            out,
            "            $childrenMap[$pos][] = {}::fromRow($row);",
            child_struct_name
        );
        let _ = writeln!(out, "        }}");

        let _ = writeln!(out, "        $result = [];");
        let _ = writeln!(out, "        foreach ($parentArgs as $pos => $args) {{");
        let _ = writeln!(
            out,
            "            $result[] = new {}(...$args, children: $childrenMap[$pos]);",
            parent_struct_name
        );
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "        return $result;");
        let _ = write!(out, "    }}");

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "enum {}: string {{", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "    case {} = \"{}\";", variant, value);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "readonly class {} {{", name);
        let _ = writeln!(out, "    public function __construct(");
        if composite.fields.is_empty() {
        } else {
            for field in &composite.fields {
                let _ = writeln!(out, "        public mixed ${},", field.name);
            }
        }
        let _ = writeln!(out, "    ) {{}}");
        let _ = write!(out, "}}");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::PhpAmphpBackend;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

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
            aq.sql = "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id".to_string();
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
    fn test_grouped_php_amphp_structs() {
        let backend = PhpAmphpBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("readonly class GetUsersWithOrdersChildRow"),
            "missing child class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("public static function fromRow"),
            "child missing fromRow; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("readonly class GetUsersWithOrdersRow"),
            "missing parent class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("public array $children"),
            "parent missing children field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("@var GetUsersWithOrdersChildRow[]"),
            "parent missing @var annotation for children; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("readonly class GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("readonly class GetUsersWithOrdersRow").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent; got:\n{row_struct}");
    }

    #[test]
    fn test_grouped_php_amphp_query_fn() {
        let backend = PhpAmphpBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("getUsersWithOrders"),
            "missing function name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("\\Amp\\Sql\\SqlConnectionPool $pool"),
            "missing Amp pool parameter; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("): array"),
            "wrong return type (expected array); got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("$parentIndex"),
            "missing fold index; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("$childrenMap"),
            "missing children map; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("foreach ($resultSet as $row)"),
            "must use foreach over resultSet; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow::fromRow($row)"),
            "must fold children via fromRow; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("new GetUsersWithOrdersRow(...$args, children: $childrenMap[$pos])"),
            "must build parent with named-arg spread; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return $result"),
            "must return result array; got:\n{query_fn}"
        );
    }
}
