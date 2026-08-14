//! Docblock rendering shared by the PHP backends.
//!
//! PHP is the one target where a type has to be written twice. The native
//! position -- a promoted property, a parameter, a return type -- accepts no
//! generic syntax, so `[types.containers]` maps `array` to a bare `array`;
//! `public array<string> $tags` is a parse error, not a style question. A
//! PHPStan docblock has the opposite constraint: at level 9 a bare `array` is
//! `array<mixed, mixed>` and every read out of it is an error, so the element
//! type has to appear *somewhere*.
//!
//! The two strings come from the same manifest and the same resolver: the
//! native one from `[types.containers]`, the docblock one from
//! `[types.docblock_containers]` falling back to `[types.containers]`. Nothing
//! here decides what a PHP array is called -- ask the manifest, twice.

use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::composite_type_name;
use scythe_backend::types::{resolve_docblock_type, resolve_type};

use scythe_core::analyzer::CompositeInfo;
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::GeneratedCode;
use crate::backend_trait::{ResolvedColumn, ResolvedParam};

/// Class name of the exception every php-* backend throws for a `:one` query whose database
/// returns no row.
///
/// `:one` means "exactly one row, error if absent" -- unlike `:opt`, which returns `null` for a
/// legitimately absent row. `RuntimeException` is the idiomatic PHP base for an
/// application-specific exception (this project's php-conventions: "specific exceptions
/// extending RuntimeException"), matching the `throw new RuntimeException(...)` the generated
/// integration-test harness (`tools/integration-test-generator/templates/php.php.jinja`)
/// already uses for its own assertion failures.
pub(crate) const RECORD_NOT_FOUND_EXCEPTION_CLASS: &str = "RecordNotFoundException";

/// The `RecordNotFoundException` class declaration, written once per generated file (see
/// `PhpPdoBackend::file_header`/`PhpAmphpBackend::file_header`) alongside the row classes and
/// query methods that reference it.
pub(crate) fn record_not_found_exception_class_def() -> String {
    format!("final class {RECORD_NOT_FOUND_EXCEPTION_CLASS} extends \\RuntimeException {{}}\n")
}

/// The docblock type for a value already rendered as `native` in a native
/// position, or `None` when the two are the same string.
///
/// `None` is the common case, and it is the reason this returns an `Option`
/// rather than a string: emitting `/** @var string */` above `public string
/// $name` is noise that says nothing the native type did not already say, and
/// PHPStan gains nothing from it. Only the positions where the manifest can
/// say more than PHP's syntax allows get a docblock.
fn narrower_docblock_type(
    neutral: &str,
    nullable: bool,
    native: &str,
    name: &str,
    manifest: &BackendManifest,
) -> Result<Option<String>, ScytheError> {
    let rendered = resolve_docblock_type(neutral, manifest, nullable).map_err(|e| {
        ScytheError::new(
            ErrorCode::InternalError,
            format!("docblock type resolution failed for '{name}': {e}"),
        )
    })?;

    // Compared against the *emitted* native string rather than against a
    // freshly resolved one on purpose: that makes this a cross-check of what
    // actually reached the file, so a native type produced some other way
    // (an override, a hand-built `ResolvedColumn`) still gets a docblock when
    // it differs and still gets none when it does not.
    if rendered == native {
        return Ok(None);
    }
    Ok(Some(rendered.into_owned()))
}

/// Write one promoted constructor property, preceded by a `/** @var ... */`
/// line when the docblock type is narrower than the native one.
pub(crate) fn write_promoted_property(
    out: &mut String,
    column: &ResolvedColumn,
    manifest: &BackendManifest,
) -> Result<(), ScytheError> {
    if let Some(docblock) = narrower_docblock_type(
        &column.neutral_type,
        column.nullable,
        &column.full_type,
        &column.name,
        manifest,
    )? {
        let _ = writeln!(out, "        /** @var {} */", docblock);
    }
    let _ = writeln!(out, "        public {} ${},", column.full_type, column.field_name);
    Ok(())
}

/// The type to render in a parameter's `@param` tag: the docblock type where
/// it says more, the native type otherwise.
pub(crate) fn param_docblock_type(param: &ResolvedParam, manifest: &BackendManifest) -> Result<String, ScytheError> {
    let narrower = narrower_docblock_type(
        &param.neutral_type,
        param.nullable,
        &param.full_type,
        &param.name,
        manifest,
    )?;
    Ok(narrower.unwrap_or_else(|| param.full_type.clone()))
}

/// Splits a PostgreSQL composite's text form (`"(a,b,c)"`) into its raw field tokens, honoring
/// its escaping rules -- an empty unquoted field is SQL NULL, and a field containing a comma,
/// paren, quote, backslash, or leading/trailing space (or the empty string) is double-quoted,
/// with an inner `"` **doubled** and an inner `\` backslash-escaped.
///
/// Shared by `php-pdo` and `php-amphp` (both `PDO::FETCH_ASSOC` and `amphp/postgres` hand back a
/// composite column as this exact driver text, unparsed) so there is exactly one copy of the
/// escaping rule to get right -- copying this block per backend is how the doubled-quote defect
/// (board #220) spread across nine other backends in the first place.
const PHP_PARSE_COMPOSITE_FIELDS_METHOD: &str = r#"    /**
     * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
     * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as null); a
     * field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
     * space, or the empty string) is wrapped in double quotes; every other field is unquoted and
     * taken literally. A nested composite's own "(x,y)" text form always contains parens, so it
     * always comes back quoted here, ready for that type's own fromText to parse recursively.
     *
     * Inside a quoted field record_out writes a literal '"' as '""' and a literal '\' as '\\'.
     * Both spellings must be accepted: reading '""' as "closing quote, then a new field" both
     * truncates the value and desynchronizes every field after it. Verified against
     * PostgreSQL 16 -- ROW('he said "hi"', 'back\slash', NULL) renders as
     * ("he said ""hi""","back\\slash",).
     *
     * @return array<int, string|null>
     */
    public static function parseCompositeFields(string $text): array {
        $fields = [];
        $inner = substr($text, 1, strlen($text) - 2);
        $i = 0;
        $n = strlen($inner);
        while (true) {
            $field = '';
            $isNull = false;
            if ($i < $n && $inner[$i] === '"') {
                $i++;
                while ($i < $n) {
                    $c = $inner[$i];
                    if ($c === '\\' && $i + 1 < $n) {
                        $field .= $inner[$i + 1];
                        $i += 2;
                    } elseif ($c === '"' && $i + 1 < $n && $inner[$i + 1] === '"') {
                        $field .= '"';
                        $i += 2;
                    } elseif ($c === '"') {
                        $i++;
                        break;
                    } else {
                        $field .= $c;
                        $i++;
                    }
                }
            } else {
                $start = $i;
                while ($i < $n && $inner[$i] !== ',') {
                    $i++;
                }
                $field = substr($inner, $start, $i - $start);
                $isNull = $field === '';
            }
            $fields[] = $isNull ? null : $field;
            if ($i < $n && $inner[$i] === ',') {
                $i++;
                continue;
            }
            break;
        }
        return $fields;
    }
"#;

/// The name of the file-level class holding [`PHP_PARSE_COMPOSITE_FIELDS_METHOD`].
///
/// One class per generated file rather than one copy of the parser per composite: a file
/// declaring three composites would otherwise carry three byte-identical copies of a ~40-line
/// parser, and `mago` reports each composite class as high-complexity for carrying it. Same
/// shape `go_pgx.rs` settled on for the same reason, gated the same way.
pub(crate) const PHP_COMPOSITE_PARSER_CLASS: &str = "ScytheCompositeText";

/// The shared parser class, emitted once per file by
/// [`composite_parser_class_if_used`].
fn php_composite_parser_class_def() -> String {
    format!("final class {PHP_COMPOSITE_PARSER_CLASS} {{\n{PHP_PARSE_COMPOSITE_FIELDS_METHOD}}}\n")
}

/// The shared composite-parser class, when anything in `generated` actually calls it.
///
/// Gated rather than unconditional so a file with no composite column does not grow a class
/// nothing references -- the same reason `go_pgx.rs` gates its own shared parser on a call-site
/// match rather than on "this backend supports composites".
pub(crate) fn composite_parser_class_if_used(generated: &[GeneratedCode]) -> String {
    let call = format!("{PHP_COMPOSITE_PARSER_CLASS}::parseCompositeFields(");
    let used = generated.iter().any(|code| {
        [
            code.enum_def.as_deref(),
            code.model_struct.as_deref(),
            code.row_struct.as_deref(),
            code.query_fn.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|fragment| fragment.contains(&call))
            || code.nested_struct_defs.iter().any(|def| def.code.contains(&call))
    });
    if used {
        format!("\n{}", php_composite_parser_class_def())
    } else {
        String::new()
    }
}

/// The PHP expression converting one composite field's raw text token (`raw`, a `string`
/// already unescaped by `parseCompositeFields`) into the field's declared PHP type -- the
/// inverse of what PostgreSQL's composite output function wrote for that field.
///
/// A field's own declared type is always non-nullable (`php_composite_def` resolves every field
/// with `nullable: false` -- composite fields carry no per-field nullability, matching
/// `java_jdbc.rs`'s identical note), so a genuinely NULL sub-field converted through a
/// non-string arm (`(int) null`, ...) silently becomes `0` rather than throwing. That is a
/// pre-existing gap in what `CompositeFieldInfo` tracks, not one this fix introduces or can
/// close from here.
fn php_composite_field_from_text(
    neutral_type: &str,
    field_type: &str,
    raw: &str,
    manifest: &BackendManifest,
) -> String {
    if let Some(sql_name) = neutral_type.strip_prefix("composite::") {
        return format!("{}::fromText({raw})", composite_type_name(sql_name, &manifest.naming));
    }
    if neutral_type.starts_with("enum::") {
        return format!("{field_type}::from({raw})");
    }
    match neutral_type {
        "bool" => format!("{raw} === 't'"),
        "int16" | "int32" | "int64" => format!("(int) {raw}"),
        "float32" | "float64" => format!("(float) {raw}"),
        // ~keep PostgreSQL's default `bytea` text output is hex ("\x48656c6c6f"); decode the
        // digits after the "\x" prefix back into bytes.
        "bytes" => format!("hex2bin(substr({raw}, 2))"),
        "date" | "time" | "time_tz" | "datetime" | "datetime_tz" => format!("new \\DateTimeImmutable({raw})"),
        "json" if field_type == "array" => format!("json_decode({raw}, true)"),
        // ~keep "string"/"uuid"/"decimal"/"interval"/"inet"/"json" (when declared "string") all
        // resolve to PHP `string`, so the already-parsed text needs only the cast PHPStan wants.
        // Any neutral type not named above (e.g. an array-typed composite field) falls through
        // here too; passing the raw text through is the least-wrong fallback available at
        // generate time rather than a hard error.
        _ => format!("(string) {raw}"),
    }
}

/// Build a composite type's PHP class: a `readonly class` with one promoted property per field,
/// a `fromText(?string $text): ?self` factory that parses the driver's composite text form, and
/// the private `parseCompositeFields` helper `fromText` depends on.
///
/// Shared by `php-pdo` and `php-amphp`: both hand a composite column back as PostgreSQL's
/// `record_out` text (see `PHP_PARSE_COMPOSITE_FIELDS_METHOD`'s doc comment), so the class this
/// produces is identical for either driver -- only the call site that invokes `fromText` differs
/// (`$row['col']` from `PDO::FETCH_ASSOC` vs. from an AMPHP row), and that stays per-backend.
pub(crate) fn generate_composite_def(
    composite: &CompositeInfo,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let name = composite_type_name(&composite.sql_name, &manifest.naming);
    let mut out = String::new();
    // ~keep board #220: a composite with zero fields cannot exist in PostgreSQL
    // (`CREATE TYPE ... AS ()` is rejected), so there is no reachable runtime value that would
    // need `fromText` here. Left as the bare class it always was.
    if composite.fields.is_empty() {
        let _ = writeln!(out, "readonly class {} {{}}", name);
        return Ok(out);
    }
    let field_types: Vec<String> = composite
        .fields
        .iter()
        .map(|f| {
            resolve_type(&f.neutral_type, manifest, false)
                .map(|t| t.into_owned())
                .unwrap_or_else(|_| "mixed".to_string())
        })
        .collect();
    let _ = writeln!(out, "readonly class {} {{", name);
    let _ = writeln!(out, "    public function __construct(");
    for (field, field_type) in composite.fields.iter().zip(&field_types) {
        let _ = writeln!(out, "        public {} ${},", field_type, field.name);
    }
    let _ = writeln!(out, "    ) {{}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public static function fromText(?string $text): ?self {{");
    let _ = writeln!(out, "        if ($text === null) {{");
    let _ = writeln!(out, "            return null;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(
        out,
        "        $f = {PHP_COMPOSITE_PARSER_CLASS}::parseCompositeFields($text);"
    );
    let _ = writeln!(out, "        return new self(");
    for (i, (field, field_type)) in composite.fields.iter().zip(&field_types).enumerate() {
        let raw = format!("$f[{}]", i);
        let value_expr = php_composite_field_from_text(&field.neutral_type, field_type, &raw, manifest);
        let _ = writeln!(out, "            {},", value_expr);
    }
    let _ = writeln!(out, "        );");
    let _ = writeln!(out, "    }}");
    let _ = write!(out, "}}");
    Ok(out)
}
