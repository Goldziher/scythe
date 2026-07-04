use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case};
use scythe_backend::types::resolve_type;
use std::collections::HashMap;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/php-pdo.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/php-pdo.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/php-pdo.sqlite.toml");
const DEFAULT_MANIFEST_MSSQL: &str = include_str!("../../manifests/php-pdo.mssql.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/php-pdo.redshift.toml");
const DEFAULT_MANIFEST_SNOWFLAKE: &str = include_str!("../../manifests/php-pdo.snowflake.toml");

pub struct PhpPdoBackend {
    manifest: BackendManifest,
    namespace: String,
}

impl PhpPdoBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" | "mariadb" => DEFAULT_MANIFEST_MYSQL,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            "mssql" => DEFAULT_MANIFEST_MSSQL,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            "snowflake" => DEFAULT_MANIFEST_SNOWFLAKE,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for php-pdo backend", engine),
                ));
            }
        };
        let manifest = super::load_or_default_manifest("backends/php-pdo/manifest.toml", default_toml)?;
        Ok(Self {
            manifest,
            namespace: "App\\Generated".to_string(),
        })
    }
}

/// Map a neutral type to a PHP cast expression.
fn php_cast(neutral_type: &str) -> &'static str {
    match neutral_type {
        "int16" | "int32" | "int64" => "(int) ",
        "float32" | "float64" => "(float) ",
        "bool" => "(bool) ",
        "string" | "json" | "inet" | "interval" | "uuid" | "decimal" | "bytes" => "(string) ",
        _ => "",
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
        let cast = php_cast(&c.neutral_type);
        if c.nullable {
            format!("$row['{}'] !== null ? {}$row['{}'] : null", c.name, cast, c.name)
        } else {
            format!("{}$row['{}']", cast, c.name)
        }
    }
}

/// Write a `fromRow(array $row): self` static factory method for the given columns.
///
/// The output is byte-identical to what [`PhpPdoBackend::generate_row_struct`] emits
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
            let cast = php_cast(&c.neutral_type);
            if c.nullable {
                let _ = writeln!(
                    out,
                    "            {}: $row['{}'] !== null ? {}{} : null{}",
                    c.field_name,
                    c.name,
                    cast,
                    format_args!("$row['{}']", c.name),
                    sep
                );
            } else {
                let _ = writeln!(out, "            {}: {}$row['{}']{}", c.field_name, cast, c.name, sep);
            }
        }
    }
    let _ = writeln!(out, "        );");
    let _ = writeln!(out, "    }}");
}

impl CodegenBackend for PhpPdoBackend {
    fn name(&self) -> &str {
        "php-pdo"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &[
            "postgresql",
            "mysql",
            "mariadb",
            "sqlite",
            "mssql",
            "redshift",
            "snowflake",
        ]
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        if let Some(ns) = options.get("namespace") {
            self.namespace = ns.clone();
        }
        Ok(())
    }

    fn file_header(&self) -> String {
        let ns = if self.namespace.is_empty() {
            String::new()
        } else {
            format!("namespace {};\n\n", self.namespace)
        };
        format!("<?php\n\ndeclare(strict_types=1);\n\n{ns}// Auto-generated by scythe. Do not edit.\n")
    }

    fn query_class_header(&self) -> String {
        "final class Queries {".to_string()
    }

    fn file_footer(&self) -> String {
        "}".to_string()
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();

        // Readonly class with constructor
        let _ = writeln!(out, "readonly class {} {{", struct_name);
        let _ = writeln!(out, "    public function __construct(");
        for c in columns.iter() {
            let sep = ",";
            let _ = writeln!(out, "        public {} ${}{}", c.full_type, c.field_name, sep);
        }
        let _ = writeln!(out, "    ) {{}}");
        let _ = writeln!(out);

        // fromRow factory method
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
                // Enum columns: convert DB string to PHP backed enum via ::from()
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
                // DateTime columns: PDO returns strings, wrap in DateTimeImmutable
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
                let cast = php_cast(&c.neutral_type);
                if c.nullable {
                    let _ = writeln!(
                        out,
                        "            {}: $row['{}'] !== null ? {}{} : null{}",
                        c.field_name,
                        c.name,
                        cast,
                        format_args!("$row['{}']", c.name),
                        sep
                    );
                } else {
                    let _ = writeln!(out, "            {}: {}$row['{}']{}", c.field_name, cast, c.name, sep);
                }
            }
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let name = to_pascal_case(table_name);
        self.generate_row_struct(&name, columns)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!(":p{n}"),
        );
        let mut out = String::new();

        // Handle :batch separately
        if matches!(analyzed.command, QueryCommand::Batch) {
            let batch_fn_name = format!("{}Batch", func_name);
            // PHPDoc for batch function
            let _ = writeln!(out, "    /**");
            let _ = writeln!(out, "     * @param \\PDO $pdo");
            let _ = writeln!(out, "     * @param array<int, array<int, mixed>> $items");
            let _ = writeln!(out, "     * @return void");
            let _ = writeln!(out, "     */");
            let _ = writeln!(
                out,
                "    public static function {}(\\PDO $pdo, array $items): void {{",
                batch_fn_name
            );
            let _ = writeln!(out, "        $stmt = $pdo->prepare(\"{}\");", sql);
            let _ = writeln!(out, "        $pdo->beginTransaction();");
            let _ = writeln!(out, "        try {{");
            let _ = writeln!(out, "            foreach ($items as $item) {{");
            if params.is_empty() {
                let _ = writeln!(out, "                $stmt->execute();");
            } else {
                let use_positional = sql.contains('?');
                if use_positional {
                    let _ = writeln!(out, "                $stmt->execute($item);");
                } else {
                    // Named params — build mapping from item array
                    let bindings = params
                        .iter()
                        .enumerate()
                        .map(|(i, _p)| format!("\"p{}\" => $item[{}]", i + 1, i))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "                $stmt->execute([{}]);", bindings);
                }
            }
            let _ = writeln!(out, "            }}");
            let _ = writeln!(out, "            $pdo->commit();");
            let _ = writeln!(out, "        }} catch (\\Throwable $e) {{");
            let _ = writeln!(out, "            $pdo->rollBack();");
            let _ = writeln!(out, "            throw $e;");
            let _ = writeln!(out, "        }}");
            let _ = write!(out, "    }}");
            return Ok(out);
        }

        // Build PHP parameter list
        let param_list = params
            .iter()
            .map(|p| format!("{} ${}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // Return type depends on command
        let return_type = match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => format!("?{}", struct_name),
            QueryCommand::Many => "\\Generator".to_string(),
            QueryCommand::Exec => "void".to_string(),
            QueryCommand::ExecResult | QueryCommand::ExecRows => "int".to_string(),
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        };

        // PHPDoc block
        let _ = writeln!(out, "    /**");
        let _ = writeln!(out, "     * @param \\PDO $pdo");
        for p in params {
            let _ = writeln!(out, "     * @param {} ${}", p.full_type, p.field_name);
        }
        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
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
            "    public static function {}(\\PDO $pdo{}{}): {} {{",
            func_name, sep, param_list, return_type
        );

        // Prepare statement
        let _ = writeln!(out, "        $stmt = $pdo->prepare(\"{}\");", sql);

        // Build execute params
        // If the SQL contains `?` placeholders (MySQL/SQLite), use positional array.
        // If it contains `:pN` placeholders (PostgreSQL), use named array.
        if params.is_empty() {
            let _ = writeln!(out, "        $stmt->execute();");
        } else {
            let use_positional = sql.contains('?');
            let bindings = params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let value = if p.neutral_type.starts_with("enum::") {
                        format!("${}->value", p.field_name)
                    } else {
                        format!("${}", p.field_name)
                    };
                    if use_positional {
                        value
                    } else {
                        format!("\"p{}\" => {}", i + 1, value)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "        $stmt->execute([{}]);", bindings);
        }

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "        $row = $stmt->fetch(\\PDO::FETCH_ASSOC);");
                let _ = writeln!(out, "        return $row ? {}::fromRow($row) : null;", struct_name);
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "        while ($row = $stmt->fetch(\\PDO::FETCH_ASSOC)) {{");
                let _ = writeln!(out, "            yield {}::fromRow($row);", struct_name);
                let _ = writeln!(out, "        }}");
            }
            QueryCommand::Exec => {
                // nothing else needed
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "        return $stmt->rowCount();");
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

        // Child class — defined first so the parent's `@var Child[]` annotation resolves.
        let _ = writeln!(out, "readonly class {} {{", child_struct_name);
        let _ = writeln!(out, "    public function __construct(");
        for c in child_columns.iter() {
            let _ = writeln!(out, "        public {} ${},", c.full_type, c.field_name);
        }
        let _ = writeln!(out, "    ) {{}}");
        let _ = writeln!(out);
        write_php_from_row_method(&mut out, child_columns);
        let _ = write!(out, "}}");

        let _ = writeln!(out);
        let _ = writeln!(out);

        // Parent class — all parent_columns plus a readonly `array $children`.
        let _ = writeln!(out, "readonly class {} {{", parent_struct_name);
        let _ = writeln!(out, "    public function __construct(");
        for c in parent_columns.iter() {
            let _ = writeln!(out, "        public {} ${},", c.full_type, c.field_name);
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
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!(":p{n}"),
        );
        let mut out = String::new();

        // Build PHP parameter list (same pattern as generate_query_fn)
        let param_list = params
            .iter()
            .map(|p| format!("{} ${}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // PHPDoc block
        let _ = writeln!(out, "    /**");
        let _ = writeln!(out, "     * @param \\PDO $pdo");
        for p in params {
            let _ = writeln!(out, "     * @param {} ${}", p.full_type, p.field_name);
        }
        let _ = writeln!(out, "     * @return {}[]", parent_struct_name);
        let _ = writeln!(out, "     */");

        let _ = writeln!(
            out,
            "    public static function {}(\\PDO $pdo{}{}): array {{",
            func_name, sep, param_list
        );

        // Prepare statement
        let _ = writeln!(out, "        $stmt = $pdo->prepare(\"{}\");", sql);

        // Execute with params
        if params.is_empty() {
            let _ = writeln!(out, "        $stmt->execute();");
        } else {
            let use_positional = sql.contains('?');
            let bindings = params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let value = if p.neutral_type.starts_with("enum::") {
                        format!("${}->value", p.field_name)
                    } else {
                        format!("${}", p.field_name)
                    };
                    if use_positional {
                        value
                    } else {
                        format!("\"p{}\" => {}", i + 1, value)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "        $stmt->execute([{}]);", bindings);
        }

        // Fold state: index maps key → position, parentArgs stores decoded parent fields,
        // childrenMap accumulates children per group. Parallel arrays iterate in insertion order.
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

        // Row-by-row fold
        let _ = writeln!(out, "        while ($row = $stmt->fetch(\\PDO::FETCH_ASSOC)) {{");
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

        // Build result array. Named-argument spread (...$args) unpacks the parent fields
        // dict into constructor parameters; readonly class requires PHP 8.1+.
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
            // empty constructor
        } else {
            for field in &composite.fields {
                let field_type = resolve_type(&field.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|_| "mixed".to_string());
                let _ = writeln!(out, "        public {} ${},", field_type, field.name);
            }
        }
        let _ = writeln!(out, "    ) {{}}");
        let _ = write!(out, "}}");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::PhpPdoBackend;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    fn make_grouped_query() -> AnalyzedQuery {
        let parent_cols = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
            },
            AnalyzedColumn {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            },
            AnalyzedColumn {
                name: "email".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            },
        ];
        let child_cols = vec![
            AnalyzedColumn {
                name: "order_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
            },
            AnalyzedColumn {
                name: "total".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
            },
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
            },
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
        AnalyzedQuery {
            name: "GetUsersWithOrders".to_string(),
            command: QueryCommand::Grouped,
            sql: "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id".to_string(),
            columns: all_cols,
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            }),
            custom: vec![],
        }
    }

    #[test]
    fn test_grouped_php_pdo_structs() {
        let backend = PhpPdoBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        // Child class must appear before parent class
        assert!(
            row_struct.contains("readonly class GetUsersWithOrdersChildRow"),
            "missing child class; got:\n{row_struct}"
        );
        // Child must have fromRow factory with correct column conversions
        assert!(
            row_struct.contains("public static function fromRow"),
            "child missing fromRow; got:\n{row_struct}"
        );
        // Parent class must include children array
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
        // Ordering: child must be defined before parent
        let child_pos = row_struct.find("readonly class GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("readonly class GetUsersWithOrdersRow").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent; got:\n{row_struct}");
    }

    #[test]
    fn test_grouped_php_pdo_query_fn() {
        let backend = PhpPdoBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        // Correct function name and PDO parameter
        assert!(
            query_fn.contains("getUsersWithOrders"),
            "missing function name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("\\PDO $pdo"),
            "missing PDO parameter; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("): array"),
            "wrong return type (expected array); got:\n{query_fn}"
        );
        // Fold data structures
        assert!(
            query_fn.contains("$parentIndex"),
            "missing fold index; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("$childrenMap"),
            "missing children map; got:\n{query_fn}"
        );
        // Children folded via child struct's fromRow
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow::fromRow($row)"),
            "must fold children via fromRow; got:\n{query_fn}"
        );
        // Parent constructed with named-argument spread (PHP 8.1+)
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
