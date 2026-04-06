use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case,
};

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../../../backends/php-pdo/manifest.toml");

pub struct PhpPdoBackend {
    manifest: BackendManifest,
}

impl PhpPdoBackend {
    pub fn new() -> Result<Self, ScytheError> {
        let manifest_path = Path::new("backends/php-pdo/manifest.toml");
        let manifest = if manifest_path.exists() {
            load_manifest(manifest_path)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        } else {
            toml::from_str(DEFAULT_MANIFEST_TOML)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        };
        Ok(Self { manifest })
    }

    pub fn manifest(&self) -> &BackendManifest {
        &self.manifest
    }
}

/// Strip SQL comments, trailing semicolons, and excess whitespace.
fn clean_sql(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Rewrite $1, $2, ... to :p1, :p2, ...
fn rewrite_params(sql: &str) -> String {
    let mut result = sql.to_string();
    // Replace from highest number down to avoid $1 matching inside $10
    for i in (1..=99).rev() {
        let from = format!("${}", i);
        let to = format!(":p{}", i);
        result = result.replace(&from, &to);
    }
    result
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

impl CodegenBackend for PhpPdoBackend {
    fn name(&self) -> &str {
        "php-pdo"
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();

        // Readonly class with constructor
        let _ = writeln!(out, "readonly class {} {{", struct_name);
        let _ = writeln!(out, "    public function __construct(");
        for (i, c) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "," };
            let _ = writeln!(
                out,
                "        public {} ${}{}",
                c.full_type, c.field_name, sep
            );
        }
        let _ = writeln!(out, "    ) {{}}");
        let _ = writeln!(out);

        // fromRow factory method
        let _ = writeln!(
            out,
            "    public static function fromRow(array $row): self {{"
        );
        let _ = writeln!(out, "        return new self(");
        for (i, c) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "," };
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
                let _ = writeln!(
                    out,
                    "            {}: {}$row['{}']{}",
                    c.field_name, cast, c.name, sep
                );
            }
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_model_struct(
        &self,
        table_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
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
        let sql = rewrite_params(&clean_sql(&analyzed.sql));
        let mut out = String::new();

        // Build PHP parameter list
        let param_list = params
            .iter()
            .map(|p| format!("{} ${}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // Return type depends on command
        let return_type = match &analyzed.command {
            QueryCommand::One => format!("?{}", struct_name),
            QueryCommand::Many | QueryCommand::Batch => "array".to_string(),
            QueryCommand::Exec => "void".to_string(),
            QueryCommand::ExecResult | QueryCommand::ExecRows => "int".to_string(),
        };

        let _ = writeln!(
            out,
            "function {}(PDO $pdo{}{}): {} {{",
            func_name, sep, param_list, return_type
        );

        // Prepare statement
        let _ = writeln!(out, "    $stmt = $pdo->prepare(\"{}\");", sql);

        // Build execute params
        if params.is_empty() {
            let _ = writeln!(out, "    $stmt->execute();");
        } else {
            let bindings = params
                .iter()
                .enumerate()
                .map(|(i, p)| format!("\"p{}\" => ${}", i + 1, p.field_name))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "    $stmt->execute([{}]);", bindings);
        }

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "    $row = $stmt->fetch(PDO::FETCH_ASSOC);");
                let _ = writeln!(
                    out,
                    "    return $row ? {}::fromRow($row) : null;",
                    struct_name
                );
            }
            QueryCommand::Many | QueryCommand::Batch => {
                let _ = writeln!(out, "    $rows = $stmt->fetchAll(PDO::FETCH_ASSOC);");
                let _ = writeln!(
                    out,
                    "    return array_map([{}::class, 'fromRow'], $rows);",
                    struct_name
                );
            }
            QueryCommand::Exec => {
                // nothing else needed
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "    return $stmt->rowCount();");
            }
        }

        let _ = write!(out, "}}");
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
        let _ = writeln!(out, "    public function __construct() {{}}");
        let _ = write!(out, "}}");
        Ok(out)
    }
}
