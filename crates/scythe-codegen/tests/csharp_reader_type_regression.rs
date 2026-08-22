//! Regression tests for the C# backends' column readers (issue #155).
//!
//! Every `csharp-*` backend picked its ADO.NET accessor from a hand-written
//! table keyed on the neutral type, ending in `_ => "GetValue"`. The manifests
//! meanwhile declare the *record field* from a second, independent table --
//! `array = "List<{T}>"`, `bytes = "byte[]"`, a composite as its own record --
//! and nothing ever cross-checked the two. `DbDataReader.GetValue` returns
//! `object`, so every neutral type that fell through produced a file the C#
//! compiler rejected outright:
//!
//! ```text
//! error CS1503: Argument 2: cannot convert from 'object' to
//! 'System.Collections.Generic.List<string>'
//! ```
//!
//! Two arms were wrong without even reaching the fallback: `csharp-npgsql.toml`
//! declares `interval = "TimeSpan"` and `inet = "System.Net.IPAddress"` while
//! the reader routed both through `GetString`, which is the same disagreement
//! wearing a different accessor.
//!
//! The fix is a typed reader, not a degraded manifest: `GetFieldValue<T>`
//! returns `T` by signature, and against a live PostgreSQL 17 server through
//! Npgsql 8, `GetFieldValue<List<string>>` really does read a `text[]`,
//! `GetFieldValue<TimeSpan>` an `interval`, `GetFieldValue<IPAddress>` an
//! `inet` and `GetFieldValue<byte[]>` a `bytea`. Rewriting the manifests to say
//! `string` (the route the JVM backends took, and which #158 exists to undo)
//! would have thrown away types the driver can genuinely produce.
//!
//! What makes the sweep at the bottom non-vacuous is that it does not compare
//! against a second hardcoded expectation table. It resolves each neutral type
//! through the manifest, runs the real backend, and then asks what the *static
//! C# type* of the emitted expression is -- an accessor this file does not
//! recognise is a hard failure, not a skip.

use std::collections::BTreeSet;
use std::path::PathBuf;

use scythe_backend::types::resolve_type;
use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_structural, validate_with_tools};
use scythe_codegen::{CodegenBackend, generate_with_backend, get_backend, provenance};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, EnumInfo, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::{QueryCommand, parse_query_with_dialect};

/// Every `csharp-*.toml` on disk, paired with the backend and engine that
/// selects it. The manifest-wide sweep asserts this list and the directory
/// listing describe the same set, in both directions.
const MANIFEST_ROUTES: &[(&str, &str, &str)] = &[
    ("csharp-npgsql.toml", "csharp-npgsql", "postgresql"),
    ("csharp-npgsql.redshift.toml", "csharp-npgsql", "redshift"),
    ("csharp-mysqlconnector.toml", "csharp-mysqlconnector", "mysql"),
    ("csharp-microsoft-sqlite.toml", "csharp-microsoft-sqlite", "sqlite"),
    ("csharp-sqlclient.toml", "csharp-sqlclient", "mssql"),
    ("csharp-oracle.toml", "csharp-oracle", "oracle"),
    ("csharp-snowflake.toml", "csharp-snowflake", "snowflake"),
];

/// The static return type of `reader.<accessor>(ordinal)`.
///
/// `GetFieldValue<T>` is parsed rather than tabulated: `DbDataReader` declares
/// it `public virtual T GetFieldValue<T>(int ordinal)`, so its static type *is*
/// its type argument, which is the whole point of using it. Everything else is
/// a fixed `DbDataReader` member with a fixed return type.
///
/// `None` means "this file does not know what that expression's type is", and
/// every caller treats that as a failure. That is what stops the sweep from
/// quietly passing over an accessor nobody checked -- `GetValue` included.
fn accessor_return_type(accessor: &str) -> Option<&'static str> {
    Some(match accessor {
        "GetBoolean" => "bool",
        "GetInt16" => "short",
        "GetInt32" => "int",
        "GetInt64" => "long",
        "GetFloat" => "float",
        "GetDouble" => "double",
        "GetString" => "string",
        "GetGuid" => "Guid",
        "GetDecimal" => "decimal",
        "GetDateTime" => "DateTime",
        _ => return None,
    })
}

/// The static C# type of the single column-read expression in `code`.
///
/// Returns `Err` with a human-readable reason when the expression is missing,
/// duplicated, or uses an accessor [`accessor_return_type`] cannot type -- all
/// three are failures, never skips.
fn emitted_read_type(code: &str) -> Result<String, String> {
    // ~keep csharp-mysqlconnector reads a uuid through GetValue followed by
    // ToString, because MySQL has no uuid type and its manifest declares
    // string. object.ToString() is string, so the expression is well-typed
    // even though its accessor alone is not.
    if code.contains("reader.GetValue(0).ToString()!") {
        return Ok("string".to_string());
    }

    // The enum path never consults the accessor table: it parses the text
    // form, so the expression's type is `Enum.TryParse`'s type argument.
    if let Some(after) = code.split("Enum.TryParse<").nth(1)
        && let Some(type_arg) = after.split('>').next()
    {
        return Ok(type_arg.to_string());
    }

    // ~keep board #220: the composite path never consults the accessor table either -- it
    // reads the driver's raw text through `GetFieldValue<string>` and hands it to the
    // composite's own `FromText`, so the expression's *declared* type is `FromText`'s
    // receiver, not the `string` it decodes from.
    if let Some(idx) = code.find(".FromText(reader.GetFieldValue<string>(") {
        let head = &code[..idx];
        let start = head
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|p| p + 1)
            .unwrap_or(0);
        return Ok(head[start..].to_string());
    }

    let mut found: Vec<&str> = Vec::new();
    let mut rest = code;
    while let Some(pos) = rest.find("reader.") {
        rest = &rest[pos + "reader.".len()..];
        let Some(open) = rest.find('(') else { break };
        let accessor = &rest[..open];
        if accessor != "IsDBNull" && accessor != "ReadAsync" {
            found.push(accessor);
        }
    }

    match found.as_slice() {
        [] => Err("no column-read expression at all".to_string()),
        [accessor] => {
            if let Some(inner) = accessor
                .strip_prefix("GetFieldValue<")
                .and_then(|s| s.strip_suffix('>'))
            {
                return Ok(inner.to_string());
            }
            accessor.parse::<Accessor>().map(|a| a.0).map_err(|()| {
                format!(
                    "`reader.{accessor}(...)` has no known static type. `GetValue` returns \
                     `object` and binds to nothing; if this is a new accessor, add it to \
                     accessor_return_type with its real DbDataReader return type"
                )
            })
        }
        many => Err(format!("expected one column read, found {many:?}")),
    }
}

/// Newtype so [`emitted_read_type`] can use `?`-style parsing over the fixed
/// accessor table without leaking a `&'static str` lookup into every caller.
struct Accessor(String);

impl std::str::FromStr for Accessor {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        accessor_return_type(s).map(|t| Self(t.to_string())).ok_or(())
    }
}

/// PostgreSQL is the only engine here with array and composite *columns*, and
/// the only one whose manifest declares `interval`/`inet` as anything but
/// `string` -- so it reaches every shape of the defect at once.
const PG_SCHEMA: &str = "CREATE TYPE widget_status AS ENUM ('active', 'archived');\
    CREATE TYPE widget_address AS (street TEXT, city TEXT);\
    CREATE TABLE widgets (\
        id SERIAL PRIMARY KEY, \
        tags TEXT[] NOT NULL, \
        counts INTEGER[] NOT NULL, \
        statuses widget_status[] NOT NULL, \
        home_address widget_address, \
        span INTERVAL, \
        ip INET, \
        payload BYTEA NOT NULL, \
        maybe_tags TEXT[]\
    );";

const PG_QUERY: &str = "-- @name GetWidget\n-- @returns :one\n\
    SELECT id, tags, counts, statuses, home_address, span, ip, payload, maybe_tags \
    FROM widgets WHERE id = $1;";

/// The typed reads `PG_SCHEMA` must produce. Each one is a column whose
/// manifest declaration `GetValue` (or, for the last two, `GetString`) could
/// not bind to. `home_address` is the odd one out: board #220 found that
/// `reader.GetFieldValue<WidgetAddress>` -- what this list used to pin -- compiles but throws
/// `InvalidCastException` at runtime, because Npgsql has no binary decoder for a composite
/// unless the caller registers one. The fix parses the driver's text form instead of asking
/// the reader for the composite type directly.
const PG_EXPECTED_READS: &[&str] = &[
    "reader.GetFieldValue<List<string>>(1)",
    "reader.GetFieldValue<List<int>>(2)",
    "reader.GetFieldValue<List<WidgetStatus>>(3)",
    "WidgetAddress.FromText(reader.GetFieldValue<string>(4))",
    "reader.GetFieldValue<TimeSpan>(5)",
    "reader.GetFieldValue<System.Net.IPAddress>(6)",
    "reader.GetFieldValue<byte[]>(7)",
    "reader.GetFieldValue<List<string>>(8)",
];

/// Engines with no array or composite type still reach the fallback through
/// `bytes`, which every C# manifest declares `byte[]`.
fn bytes_fixture(dialect: &SqlDialect) -> (&'static str, &'static str) {
    match dialect {
        SqlDialect::MySQL => (
            "CREATE TABLE blobs (id INT PRIMARY KEY, payload BLOB NOT NULL, maybe_payload BLOB);",
            "-- @name GetBlob\n-- @returns :one\nSELECT id, payload, maybe_payload FROM blobs WHERE id = ?;",
        ),
        SqlDialect::SQLite => (
            "CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload BLOB NOT NULL, maybe_payload BLOB);",
            "-- @name GetBlob\n-- @returns :one\nSELECT id, payload, maybe_payload FROM blobs WHERE id = ?;",
        ),
        SqlDialect::MsSql => (
            "CREATE TABLE blobs (id INT PRIMARY KEY, payload VARBINARY(512) NOT NULL, maybe_payload VARBINARY(512));",
            "-- @name GetBlob\n-- @returns :one\nSELECT id, payload, maybe_payload FROM blobs WHERE id = @p1;",
        ),
        _ => (
            "CREATE TABLE blobs (id INT PRIMARY KEY, payload BLOB NOT NULL, maybe_payload BLOB);",
            "-- @name GetBlob\n-- @returns :one\nSELECT id, payload, maybe_payload FROM blobs WHERE id = $1;",
        ),
    }
}

/// Assemble the exact bytes `scythe generate` would write, provenance header
/// included, so the assertions below see a whole file rather than a fragment.
fn generate_full_file(backend_name: &str, engine: &str, dialect: &SqlDialect, schema: &str, query: &str) -> String {
    let backend = get_backend(backend_name, engine).expect("backend must support engine");
    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, dialect).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");

    let all_codes = vec![code];
    let mut full = backend.file_header_for_results(&all_codes);
    full.push('\n');
    for code in &all_codes {
        for section in [&code.enum_def, &code.model_struct, &code.row_struct]
            .into_iter()
            .flatten()
        {
            full.push_str(section);
            full.push('\n');
        }
    }
    let class_header = backend.query_class_header();
    if !class_header.is_empty() {
        full.push_str(&class_header);
        full.push('\n');
    }
    for code in &all_codes {
        if let Some(ref query_fn) = code.query_fn {
            full.push_str(query_fn);
            full.push('\n');
        }
    }
    let footer = backend.file_footer();
    if !footer.is_empty() {
        full.push_str(&footer);
        full.push('\n');
    }
    let post = backend.post_footer();
    if !post.is_empty() {
        full.push('\n');
        full.push_str(&post);
        full.push('\n');
    }

    provenance::assemble_file(
        &backend.file_preamble(),
        &provenance::header_line(
            &*backend,
            env!("CARGO_PKG_VERSION"),
            engine,
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &full,
    )
}

/// `reader.GetValue(...)` is the accessor that started this: `object` binds to
/// nothing the manifests declare. The one permitted spelling is
/// `csharp-mysqlconnector`'s uuid read, which immediately narrows it with
/// `.ToString()!` and so is a `string` expression, not an `object` one.
fn assert_no_untyped_read(backend_name: &str, code: &str) {
    let untyped = code
        .match_indices("reader.GetValue(")
        .filter(|(index, _)| {
            let tail = &code[*index..];
            let end = tail.find('\n').unwrap_or(tail.len());
            !tail[..end].contains(".ToString()!")
        })
        .count();
    assert_eq!(
        untyped, 0,
        "{backend_name} still reads a column with the untyped `reader.GetValue(...)`, whose \
         `object` static type does not bind to the type the manifest declares:\n{code}"
    );
}

/// Structural validation always runs. `validate_with_tools` has no C# arm --
/// every `csharp-*` backend references a NuGet-only driver, so there is no
/// stub-free path through `dotnet build` from a bare generated file (see
/// `validate_with_tools`' own comment and the `NO_TOOL_VALIDATOR` inventory in
/// `tool_validation.rs`).
///
/// The real C# compiler still sees this code, just not from here:
/// `scripts/check-generated-syntax.sh` runs `dotnet build` over every
/// committed `integration_tests/*/generated/*.cs`, and
/// `scripts/check-generated-backends.py` regenerates every PostgreSQL-engine
/// backend against `sql/torture/schema.sql` -- whose `tags TEXT[]`,
/// `statuses torture_status[]` and `home_address torture_address` columns are
/// exactly the shapes asserted above -- and builds that too. `csharp-npgsql`
/// passes that gate as of this fix, and is no longer on the expected-failure
/// allowlist, so a reintroduced `GetValue` fails a real `dotnet build` in CI
/// and not only the assertions here.
///
/// The `Unsupported ||` disjunction is the same shape `tool_validation.rs`
/// uses: it accepts today's gap by name while still failing the day a C#
/// validator lands but does not actually run.
fn assert_csharp_file_is_valid(backend_name: &str, code: &str) {
    let structural_errors = validate_structural(code, backend_name);
    assert!(
        structural_errors.is_empty(),
        "{backend_name} structural: {structural_errors:?}\n\n{code}"
    );

    let validation = validate_with_tools(code, backend_name);
    if strict_mode_enabled() {
        assert!(
            matches!(validation, ToolValidation::Unsupported) || validation.fully_checked(),
            "{backend_name} has a C# validator that reported no checker actually ran"
        );
    }
    if let Err(tool_errors) = validation.into_result() {
        panic!("{backend_name} tool validation: {tool_errors:?}\n\n{code}");
    }
}

#[test]
fn csharp_npgsql_reads_every_non_scalar_column_with_a_typed_accessor() {
    let code = generate_full_file(
        "csharp-npgsql",
        "postgresql",
        &SqlDialect::PostgreSQL,
        PG_SCHEMA,
        PG_QUERY,
    );
    assert_no_untyped_read("csharp-npgsql", &code);
    for expected in PG_EXPECTED_READS {
        assert!(
            code.contains(expected),
            "csharp-npgsql: expected `{expected}`; got:\n{code}"
        );
    }
    assert!(
        code.contains("List<string> Tags") && code.contains("List<string>? MaybeTags"),
        "csharp-npgsql: the manifest's `List<{{T}}>` array declaration must survive the fix -- \
         degrading it to `string` is the regression #158 exists to undo:\n{code}"
    );
    assert_csharp_file_is_valid("csharp-npgsql", &code);
}

#[test]
fn csharp_npgsql_redshift_reads_every_non_scalar_column_with_a_typed_accessor() {
    let code = generate_full_file(
        "csharp-npgsql",
        "redshift",
        &SqlDialect::PostgreSQL,
        PG_SCHEMA,
        PG_QUERY,
    );
    assert_no_untyped_read("csharp-npgsql", &code);
    for expected in PG_EXPECTED_READS {
        assert!(
            code.contains(expected),
            "csharp-npgsql/redshift: expected `{expected}`; got:\n{code}"
        );
    }
    assert_csharp_file_is_valid("csharp-npgsql", &code);
}

/// `bytes` is the one fallback type every engine can express, so it is the
/// route to the same defect on the five non-PostgreSQL backends.
#[test]
fn every_csharp_backend_reads_a_binary_column_as_a_byte_array() {
    let cases: &[(&str, &str, SqlDialect)] = &[
        ("csharp-npgsql", "postgresql", SqlDialect::PostgreSQL),
        ("csharp-npgsql", "redshift", SqlDialect::PostgreSQL),
        ("csharp-mysqlconnector", "mysql", SqlDialect::MySQL),
        ("csharp-mysqlconnector", "mariadb", SqlDialect::MySQL),
        ("csharp-microsoft-sqlite", "sqlite", SqlDialect::SQLite),
        ("csharp-sqlclient", "mssql", SqlDialect::MsSql),
        ("csharp-oracle", "oracle", SqlDialect::PostgreSQL),
        ("csharp-snowflake", "snowflake", SqlDialect::PostgreSQL),
    ];

    for (backend_name, engine, dialect) in cases {
        let (schema, query) = bytes_fixture(dialect);
        let code = generate_full_file(backend_name, engine, dialect, schema, query);
        assert!(
            code.contains("reader.GetFieldValue<byte[]>(1)"),
            "{backend_name}/{engine}: a BLOB column is declared `byte[]` but not read as one; \
             got:\n{code}"
        );
        assert!(
            code.contains("reader.IsDBNull(2) ? null : reader.GetFieldValue<byte[]>(2)"),
            "{backend_name}/{engine}: the nullable BLOB column must keep its null guard around \
             the typed read; got:\n{code}"
        );
        assert_no_untyped_read(backend_name, &code);
        assert_csharp_file_is_valid(backend_name, &code);
    }
}

/// The enum path is deliberately untouched: it never consulted the accessor
/// table, and rerouting it through `GetFieldValue<TEnum>` would need the enum
/// registered with the driver's type mapper. A test that only asserted "no
/// GetValue" would happily accept that rewrite, so pin the parse form.
#[test]
fn enum_columns_are_still_read_through_enum_try_parse() {
    let code = generate_full_file(
        "csharp-npgsql",
        "postgresql",
        &SqlDialect::PostgreSQL,
        "CREATE TYPE widget_status AS ENUM ('active', 'archived');\
         CREATE TABLE widgets (id SERIAL PRIMARY KEY, status widget_status NOT NULL);",
        "-- @name GetWidget\n-- @returns :one\nSELECT id, status FROM widgets WHERE id = $1;",
    );
    assert!(
        code.contains("Enum.TryParse<WidgetStatus>(reader.GetString(1)"),
        "csharp-npgsql: enum columns must still parse the text form; got:\n{code}"
    );
}

/// Build a one-column `:one` query carrying `neutral_type`, so the generated
/// query fn contains exactly one column read to inspect.
fn one_column_query(neutral_type: &str, nullable: bool) -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
        query.name = "ReadOne".to_string();
        query.command = QueryCommand::One;
        query.sql = "SELECT value FROM t".to_string();
        query.columns = vec![AnalyzedColumn {
            name: "value".to_string(),
            neutral_type: neutral_type.to_string(),
            nullable,
            sql_type: neutral_type.to_string(),
            ..Default::default()
        }];
        query.composites = vec![CompositeInfo {
            sql_name: "sweep_address".to_string(),
            fields: vec![CompositeFieldInfo {
                name: "street".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            }],
        }];
        query.enums = vec![EnumInfo {
            sql_name: "sweep_status".to_string(),
            values: vec!["active".to_string()],
        }];
    })
}

/// Every neutral type a manifest can put in front of the reader: its declared
/// scalars, one instance of each declared container (except `nullable`, which
/// is a wrapper rather than a type), and the two named-type prefixes.
fn neutral_types_for(backend: &dyn CodegenBackend) -> Vec<String> {
    let manifest = backend.manifest();
    let mut types: BTreeSet<String> = manifest.types.scalars.keys().cloned().collect();
    for container in manifest.types.containers.keys() {
        if container == "nullable" {
            continue;
        }
        types.insert(format!("{container}<string>"));
        types.insert(format!("{container}<int32>"));
    }
    types.insert("composite::sweep_address".to_string());
    types.insert("enum::sweep_status".to_string());
    types.into_iter().collect()
}

/// The invariant this whole file exists for, checked once per (manifest,
/// neutral type) pair: whatever the manifest declares the record field to be,
/// the reader must produce an expression of exactly that static type.
///
/// Both halves come from the code under test -- the declaration from
/// `resolve_type` against the real manifest, the reader from real codegen --
/// so there is no third expectation table to drift.
fn assert_declaration_and_reader_agree(manifest_file: &str, backend_name: &str, engine: &str) -> usize {
    let backend = get_backend(backend_name, engine).expect("backend must support engine");
    let mut checked = 0usize;

    for neutral_type in neutral_types_for(&*backend) {
        for nullable in [false, true] {
            let declared = resolve_type(&neutral_type, backend.manifest(), false)
                .unwrap_or_else(|error| panic!("{manifest_file}: {neutral_type} does not resolve: {error}"));

            let query = one_column_query(&neutral_type, nullable);
            let generated = generate_with_backend(&query, &*backend)
                .unwrap_or_else(|error| panic!("{manifest_file}: codegen failed for {neutral_type}: {error}"));
            let query_fn = generated
                .query_fn
                .unwrap_or_else(|| panic!("{manifest_file}: {neutral_type} produced no query fn"));

            let emitted = emitted_read_type(&query_fn).unwrap_or_else(|reason| {
                panic!("{manifest_file} ({backend_name}/{engine}), {neutral_type}: {reason}\n\n{query_fn}")
            });

            assert_eq!(
                emitted, *declared,
                "{manifest_file} ({backend_name}/{engine}): column of neutral type \
                 `{neutral_type}` is declared `{declared}` but read as `{emitted}`. A record \
                 field only binds to a reader expression of its own type -- this is a CS1503 \
                 in the generated file.\n\n{query_fn}"
            );
            checked += 1;
        }
    }

    checked
}

#[test]
fn every_csharp_manifest_declares_only_types_its_reader_can_produce() {
    let manifests_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("manifests");

    let on_disk: BTreeSet<String> = std::fs::read_dir(&manifests_dir)
        .expect("manifests dir must exist")
        .map(|entry| entry.expect("readable dir entry").file_name())
        .filter_map(|name| name.to_str().map(str::to_string))
        .filter(|name| name.starts_with("csharp-") && name.ends_with(".toml"))
        .collect();

    let routed: BTreeSet<String> = MANIFEST_ROUTES.iter().map(|(file, _, _)| (*file).to_string()).collect();

    // Both directions: a manifest nothing routes to is unswept, and a route to
    // a manifest that no longer exists is a stale entry. Either way the sweep
    // below is no longer checking what it claims to.
    assert_eq!(
        on_disk,
        routed,
        "the csharp-*.toml files under {} and MANIFEST_ROUTES have diverged",
        manifests_dir.display()
    );

    // A glob that matches nothing must fail rather than pass. Seven manifests
    // exist today; this is the floor, not the count.
    assert!(
        on_disk.len() >= 7,
        "expected at least 7 csharp-*.toml manifests under {}, found {} -- the glob has gone \
         stale and this test is no longer checking anything",
        manifests_dir.display(),
        on_disk.len()
    );

    let mut checked = 0usize;
    for (manifest_file, backend_name, engine) in MANIFEST_ROUTES {
        checked += assert_declaration_and_reader_agree(manifest_file, backend_name, engine);
    }

    // ~keep Seven manifests, each contributing at least eighteen scalars plus
    // three containers at two element types plus the two named-type prefixes,
    // doubled over nullable and non-nullable. The floor is deliberately far
    // below the real number so that adding a type never has to touch it, while
    // a manifest whose type tables emptied out still fails here.
    assert!(
        checked >= 7 * 20 * 2,
        "only {checked} (manifest, type, nullability) combinations were checked -- the manifests' \
         type tables have shrunk and this sweep is no longer covering them"
    );
}
