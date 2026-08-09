use std::borrow::Cow;

use sqlparser::ast::{self, DataType, TimezoneInfo};

use crate::catalog::Catalog;
use crate::dialect::SqlDialect;

use super::helpers::object_name_to_string;

/// Map a DDL type name onto scythe's neutral type vocabulary.
///
/// Re-exported from [`crate::analyzer`] because schema-drift checking has to
/// speak the same type vocabulary as query analysis.  Giving the drift checker
/// its own DDL-to-neutral table would let the two definitions diverge, and a
/// drift checker whose type mapping disagrees with the generator reports
/// mismatches the generated code does not actually have.
pub fn sql_type_to_neutral(sql_type: &str, catalog: &Catalog) -> Cow<'static, str> {
    let lower = sql_type.to_lowercase();
    let normalized = strip_precision(&lower);

    if let Some(neutral) = bit_type_to_neutral(&lower, catalog.dialect()) {
        return neutral;
    }

    if let Some(neutral) = number_type_to_neutral(&lower) {
        return neutral;
    }

    match normalized.as_str() {
        // SQLite's `INTEGER` storage class holds up to 8 bytes; there is no
        // narrower 4-byte integer type. `INT`/`INTEGER` are genuine SQLite
        // spellings so both are widened here. `INT4`/`SERIAL` are PostgreSQL-only
        // spellings that would not appear in real SQLite DDL, so they are left
        // ungated below rather than risk masking a mixed-dialect mistake.
        "integer" | "int" if catalog.dialect() == SqlDialect::SQLite => Cow::Borrowed("int64"),
        // ~keep Snowflake has no dedicated narrow integer storage: every integer type
        // (`SMALLINT`, `TINYINT`, `INT`, `INTEGER`, `BIGINT`) is an alias for
        // `NUMBER(38,0)`, so all of them must resolve to the same `int64` as the
        // bare `"number"` arm below. `INT4`/`SERIAL` are PostgreSQL-only spellings
        // that would not appear in real Snowflake DDL, so they stay ungated.
        "integer" | "int" if catalog.dialect() == SqlDialect::Snowflake => Cow::Borrowed("int64"),
        "integer" | "int" | "int4" | "serial" => Cow::Borrowed("int32"),
        // ~keep Same reasoning as the Snowflake `INT` arm above. Regression test for
        // issue #83: previously ungated, so Snowflake typed the same underlying
        // storage as `int64` via the `NUMBER`/`BIGINT` spellings and `int16` via
        // the `SMALLINT`/`TINYINT` spellings. `BYTEINT` is Snowflake's own synonym
        // for `TINYINT` and belongs here too; sqlparser has no keyword for it, so
        // it survives normalization as a raw custom type and had no arm at all --
        // it reached the backend as the invalid neutral type "byteint" and failed
        // codegen outright.
        "smallint" | "tinyint" | "byteint" if catalog.dialect() == SqlDialect::Snowflake => Cow::Borrowed("int64"),
        "smallint" | "int2" | "smallserial" => Cow::Borrowed("int16"),
        "bigint" | "int8" | "bigserial" => Cow::Borrowed("int64"),
        "tinyint" | "byteint" => Cow::Borrowed("int16"),
        "mediumint" => Cow::Borrowed("int32"),
        // DuckDB's 1-byte `TINYINT` alias. Left ungated: no other supported
        // dialect has an `INT1` spelling, so there is no risk of colliding with a
        // different engine's type. Regression test for issue #83.
        "int1" => Cow::Borrowed("int16"),
        // DuckDB's 128-bit integers. There is no neutral type wide enough to hold
        // the full range losslessly (the widest integer neutral type is `int64`),
        // so both map to `decimal` -- an arbitrary-precision type -- rather than a
        // lossy, silently-truncating `int64`. `UHUGEINT` in particular was
        // previously documented (incorrectly) as `uint64`, which is not a neutral
        // type at all -- the docs are corrected to `decimal` alongside this fix.
        // Left ungated: no other supported dialect has these spellings.
        // Regression test for issue #83.
        "hugeint" | "uhugeint" => Cow::Borrowed("decimal"),
        // ~keep Oracle/Snowflake `NUMBER(p,s)` with `s > 0` is handled scale-aware by
        // `number_type_to_neutral` above, before precision is stripped. This bare
        // arm only ever sees `NUMBER(p,0)` (explicit zero scale), `NUMBER(p)`
        // (implied zero scale per the SQL standard), and truly bare `NUMBER` (no
        // precision or scale at all) -- all indistinguishable from each other by
        // the time this string reaches here, because upstream catalog
        // normalization (`normalize_data_type` in `catalog/type_normalizer.rs`)
        // collapses `NUMBER(p)` to the same bare `"number"` spelling as a
        // parameterless `NUMBER`. Oracle's truly bare `NUMBER` is technically a
        // "floating" type that can hold fractional values, which would argue for
        // `decimal` here too -- but real schemas overwhelmingly use `NUMBER(p)`
        // (e.g. `NUMBER(1)` as an integer/boolean-flag column, as seen in this
        // repo's own Oracle fixtures) far more often than a genuinely
        // unconstrained bare `NUMBER`, and mapping this arm to `decimal` would
        // silently regress every `NUMBER(p)` integer column instead. `int64` is
        // kept as the pragmatic default; a caller that truly needs bare `NUMBER`
        // to preserve fractional precision should declare an explicit scale.
        "number" => Cow::Borrowed("int64"),

        // SQLite's `REAL` storage class is always an 8-byte IEEE float (SQLite has no
        // 4-byte float type), unlike PostgreSQL's 4-byte `real`/`float4`.
        "real" | "float4" if catalog.dialect() == SqlDialect::SQLite => Cow::Borrowed("float64"),
        // Snowflake has no dedicated narrow float storage either: `FLOAT`,
        // `FLOAT4`, `FLOAT8`, `DOUBLE`, and `REAL` are all aliases for the same
        // 8-byte `DOUBLE`. Regression test for issue #83.
        "real" | "float4" if catalog.dialect() == SqlDialect::Snowflake => Cow::Borrowed("float64"),
        "real" | "float4" => Cow::Borrowed("float32"),
        "double precision" | "float8" | "double" => Cow::Borrowed("float64"),
        // ~keep MySQL's bare `FLOAT` (no precision) is a genuine 4-byte type; every other
        // engine here defaults bare `FLOAT` to 8-byte double precision (equivalent
        // to `FLOAT(53)`).
        "float" if catalog.dialect() == SqlDialect::MySQL => Cow::Borrowed("float32"),
        "float" => Cow::Borrowed("float64"),
        "numeric" | "decimal" => Cow::Borrowed("decimal"),
        // MSSQL's `MONEY`/`SMALLMONEY` and PostgreSQL's `MONEY` are all
        // fixed-point currency types; both engines benefit from the same mapping,
        // so this is left ungated rather than split into two dialect-gated arms.
        // Regression test for issue #83.
        "money" | "smallmoney" => Cow::Borrowed("decimal"),

        // ~keep MySQL's `UNSIGNED` numeric qualifier has no dedicated neutral type, so it
        // maps to the same-width signed neutral type. This does not widen to
        // accommodate the extra range (`bigint unsigned` has no wider neutral type
        // to widen to, and widening only the narrower ones would be inconsistent),
        // so callers must be aware that values near the top of the unsigned range
        // (e.g. > i16::MAX for `smallint unsigned`, > i32::MAX for `int unsigned`,
        // > i64::MAX for `bigint unsigned`) require application-level handling.
        "tinyint unsigned" | "smallint unsigned" => Cow::Borrowed("int16"),
        "mediumint unsigned" | "int unsigned" | "integer unsigned" => Cow::Borrowed("int32"),
        "bigint unsigned" => Cow::Borrowed("int64"),

        "text" | "character varying" | "character" | "varchar" | "char" | "varchar2" | "nvarchar2" => {
            Cow::Borrowed("string")
        }
        "nvarchar" | "nchar" | "ntext" => Cow::Borrowed("string"),
        "tinytext" | "mediumtext" | "longtext" | "clob" | "nclob" => Cow::Borrowed("string"),
        "set" => Cow::Borrowed("string"),
        // MSSQL's XML column type. Left ungated: PostgreSQL also has a native
        // `xml`, and `string` is the right neutral type for both -- no driver
        // surfaces it as anything richer. Regression test for issue #83.
        "xml" => Cow::Borrowed("string"),
        // Snowflake's spatial types, which snowflake.md has always documented as
        // `string` while no arm existed -- so they reached the backend as an
        // invalid neutral type. Ungated: PostGIS uses the same two spellings and
        // also surfaces them as text.
        "geography" | "geometry" => Cow::Borrowed("string"),

        "boolean" | "bool" => Cow::Borrowed("bool"),

        "bytea" => Cow::Borrowed("bytes"),
        "blob" | "tinyblob" | "mediumblob" | "longblob" | "binary" | "varbinary" | "bfile" => Cow::Borrowed("bytes"),

        "uuid" | "uniqueidentifier" => Cow::Borrowed("uuid"),

        "date" => Cow::Borrowed("date"),
        "time" | "time without time zone" => Cow::Borrowed("time"),
        "time with time zone" | "timetz" => Cow::Borrowed("time_tz"),
        "timestamp" | "timestamp without time zone" => Cow::Borrowed("datetime"),
        "timestamp with time zone" | "timestamptz" => Cow::Borrowed("datetime_tz"),
        "datetime" | "datetime2" => Cow::Borrowed("datetime"),
        "datetimeoffset" => Cow::Borrowed("datetime_tz"),
        "interval" => Cow::Borrowed("interval"),
        "year" => Cow::Borrowed("int16"),
        "timestamp_ntz" => Cow::Borrowed("datetime"),
        "timestamp_ltz" => Cow::Borrowed("datetime_tz"),
        "timestamp_tz" => Cow::Borrowed("datetime_tz"),

        "json" | "jsonb" | "variant" | "super" => Cow::Borrowed("json"),

        "inet" | "cidr" | "macaddr" => Cow::Borrowed("inet"),

        // ~keep Bare `BIT` (no width) is `BIT(1)` per the SQL standard, which is
        // boolean-ish. `BIT VARYING`/`VARBIT` has no fixed width and is a genuine
        // bit string, so it maps to `bytes` (see `bit_type_to_neutral` for the
        // width-bearing `BIT(n)` cases).
        "bit" => Cow::Borrowed("bool"),
        "bit varying" | "varbit" => Cow::Borrowed("bytes"),

        "integer[]" | "int4[]" | "int[]" => Cow::Borrowed("array<int32>"),
        "text[]" | "character varying[]" | "varchar[]" => Cow::Borrowed("array<string>"),
        "boolean[]" | "bool[]" => Cow::Borrowed("array<bool>"),
        "bigint[]" | "int8[]" => Cow::Borrowed("array<int64>"),
        "smallint[]" | "int2[]" => Cow::Borrowed("array<int16>"),
        "real[]" | "float4[]" => Cow::Borrowed("array<float32>"),
        "double precision[]" | "float8[]" => Cow::Borrowed("array<float64>"),
        "uuid[]" => Cow::Borrowed("array<uuid>"),
        "numeric[]" | "decimal[]" => Cow::Borrowed("array<decimal>"),
        "jsonb[]" | "json[]" => Cow::Borrowed("array<json>"),

        "int4range" => Cow::Borrowed("range<int32>"),
        "int8range" => Cow::Borrowed("range<int64>"),
        "tstzrange" => Cow::Borrowed("range<datetime_tz>"),
        "tsrange" => Cow::Borrowed("range<datetime>"),
        "daterange" => Cow::Borrowed("range<date>"),
        "numrange" => Cow::Borrowed("range<decimal>"),
        _ => {
            if let Some(inner) = normalized.strip_suffix("[]") {
                let inner_neutral = sql_type_to_neutral(inner, catalog);
                return Cow::Owned(format!("array<{}>", inner_neutral));
            }
            if let Some(base_type) = catalog.get_domain_base_type(&normalized) {
                return sql_type_to_neutral(base_type, catalog);
            }
            if catalog.get_enum(&normalized).is_some() {
                return Cow::Owned(format!("enum::{}", normalized));
            }
            if catalog.get_composite(&normalized).is_some() {
                return Cow::Owned(format!("composite::{}", normalized));
            }
            Cow::Owned(normalized.to_string())
        }
    }
}

/// Resolve a width-bearing `BIT(n)` string (as produced by `normalize_data_type`,
/// which preserves the width precisely because `strip_precision` cannot: the
/// parens in a stringified `"bigint(20) unsigned"`-style suffix are not trailing,
/// but a bare `"bit(n)"` string is trailing, so this handles it directly instead).
///
/// MSSQL's `BIT` has no width (`DataType::Bit(None)`) and must keep resolving to
/// `bool` via the ordinary `"bit"` match arm — this function only fires for a
/// parsed width, so MSSQL is never affected. For dialects that do carry a width:
/// `BIT(1)` is boolean-ish; `BIT(n>1)` is a genuine multi-bit value, which MySQL
/// treats as an integer bitfield (up to 64 bits, hence `int64`) and PostgreSQL
/// treats as a true bit string with no numeric interpretation (hence `bytes`).
fn bit_type_to_neutral(lower: &str, dialect: SqlDialect) -> Option<Cow<'static, str>> {
    let inner = lower.strip_prefix("bit(")?.strip_suffix(')')?;
    let width: u64 = inner.trim().parse().ok()?;
    if width <= 1 {
        return Some(Cow::Borrowed("bool"));
    }
    Some(if dialect == SqlDialect::MySQL {
        Cow::Borrowed("int64")
    } else {
        Cow::Borrowed("bytes")
    })
}

/// Resolve a scale-bearing `NUMBER(p,s)` string (Oracle/Snowflake's `NUMBER`
/// type). `strip_precision` discards the `(p,s)` suffix before the main `match`
/// in `sql_type_to_neutral` runs, and the bare `"number"` arm there has no scale
/// handling, so a `NUMBER(10,2)` -- a genuine decimal -- would otherwise fall
/// through to `int64` and silently truncate money-shaped columns
/// (https://github.com/Goldziher/scythe/issues/83 follow-up). This must run
/// before `strip_precision` to see the scale at all.
///
/// Only an *explicit* nonzero scale routes to `decimal`; `NUMBER(p,0)` and
/// `NUMBER(p)` (implied zero scale) return `None` so they fall through to the
/// ordinary bare `"number"` arm, which resolves to `int64` -- see the comment on
/// that arm for why bare `NUMBER` also stays `int64` rather than `decimal`.
/// Left ungated: no other supported dialect has a `NUMBER` spelling, so there is
/// no risk of colliding with a different engine's type.
fn number_type_to_neutral(lower: &str) -> Option<Cow<'static, str>> {
    let inner = lower.strip_prefix("number(")?.strip_suffix(')')?;
    let (_, scale) = inner.split_once(',')?;
    let scale: i64 = scale.trim().parse().ok()?;
    if scale > 0 {
        Some(Cow::Borrowed("decimal"))
    } else {
        None
    }
}

pub(super) fn strip_precision(s: &str) -> String {
    if let Some(idx) = s.rfind('(')
        && s.ends_with(')')
    {
        let prefix = s[..idx].trim();
        let inner = &s[idx + 1..s.len() - 1];
        if inner.chars().all(|c| c.is_ascii_digit() || c == ',' || c == ' ') {
            return prefix.to_string();
        }
    }
    s.to_string()
}

pub(super) fn datatype_to_neutral(dt: &DataType, catalog: &Catalog) -> String {
    match dt {
        // See the matching comment in `sql_type_to_neutral`: SQLite's `INTEGER` is
        // always an 8-byte integer; `INT4`/`SERIAL`-style spellings are left ungated.
        DataType::Int(_) | DataType::Integer(_) if catalog.dialect() == SqlDialect::SQLite => "int64".to_string(),
        DataType::Int(_) | DataType::Integer(_) if catalog.dialect() == SqlDialect::Snowflake => "int64".to_string(),
        DataType::Int(_) | DataType::Int4(_) | DataType::Integer(_) => "int32".to_string(),
        // ~keep See the matching comment in `sql_type_to_neutral`: Snowflake stores every
        // integer type as `NUMBER(38,0)`, so `SMALLINT`/`TINYINT` must resolve to
        // `int64` there, not the narrower `int16` other dialects use.
        DataType::SmallInt(_) | DataType::Int2(_) | DataType::TinyInt(_)
            if catalog.dialect() == SqlDialect::Snowflake =>
        {
            "int64".to_string()
        }
        DataType::SmallInt(_) | DataType::Int2(_) => "int16".to_string(),
        DataType::BigInt(_) | DataType::Int8(_) => "int64".to_string(),
        // See the matching comment in `sql_type_to_neutral`: SQLite's `REAL` is always
        // an 8-byte float, unlike PostgreSQL's 4-byte `real`/`float4`.
        DataType::Real | DataType::Float4 if catalog.dialect() == SqlDialect::SQLite => "float64".to_string(),
        // See the matching comment in `sql_type_to_neutral`: Snowflake's `REAL`/
        // `FLOAT4` are aliases for the same 8-byte `DOUBLE` as every other float
        // spelling there.
        DataType::Real | DataType::Float4 if catalog.dialect() == SqlDialect::Snowflake => "float64".to_string(),
        DataType::Real | DataType::Float4 => "float32".to_string(),
        DataType::DoublePrecision | DataType::Float8 => "float64".to_string(),
        DataType::Float(info) => {
            use sqlparser::ast::ExactNumberInfo;
            match info {
                ExactNumberInfo::Precision(p) if *p <= 24 => "float32".to_string(),
                // See the matching comment in `sql_type_to_neutral`: MySQL's bare
                // `FLOAT` is a genuine 4-byte type; every other engine defaults it
                // to 8-byte double precision.
                ExactNumberInfo::None if catalog.dialect() == SqlDialect::MySQL => "float32".to_string(),
                _ => "float64".to_string(),
            }
        }
        // MySQL `UNSIGNED` numeric qualifier: see the matching comment in
        // `sql_type_to_neutral` for the same-width, no-widening rationale.
        DataType::TinyIntUnsigned(_) | DataType::Int2Unsigned(_) | DataType::SmallIntUnsigned(_) => "int16".to_string(),
        DataType::MediumIntUnsigned(_)
        | DataType::IntUnsigned(_)
        | DataType::Int4Unsigned(_)
        | DataType::IntegerUnsigned(_) => "int32".to_string(),
        DataType::BigIntUnsigned(_) | DataType::Int8Unsigned(_) => "int64".to_string(),
        DataType::DecimalUnsigned(_) | DataType::DecUnsigned(_) => "decimal".to_string(),
        DataType::RealUnsigned => "float32".to_string(),
        DataType::FloatUnsigned(info) => {
            use sqlparser::ast::ExactNumberInfo;
            match info {
                ExactNumberInfo::Precision(p) if *p <= 24 => "float32".to_string(),
                ExactNumberInfo::None if catalog.dialect() == SqlDialect::MySQL => "float32".to_string(),
                _ => "float64".to_string(),
            }
        }
        DataType::DoubleUnsigned(_) | DataType::DoublePrecisionUnsigned => "float64".to_string(),
        DataType::Numeric(_) | DataType::Decimal(_) | DataType::Dec(_) => "decimal".to_string(),
        DataType::Varchar(_)
        | DataType::CharVarying(_)
        | DataType::CharacterVarying(_)
        | DataType::Text
        | DataType::Char(_)
        | DataType::Character(_)
        | DataType::Nvarchar(_) => "string".to_string(),
        DataType::Bool | DataType::Boolean => "bool".to_string(),
        DataType::Bytea => "bytes".to_string(),
        DataType::Blob(_) => "bytes".to_string(),
        DataType::TinyBlob => "bytes".to_string(),
        DataType::MediumBlob => "bytes".to_string(),
        DataType::LongBlob => "bytes".to_string(),
        DataType::Binary(_) | DataType::Varbinary(_) => "bytes".to_string(),
        DataType::Uuid => "uuid".to_string(),
        DataType::TinyInt(_) => "int16".to_string(),
        DataType::MediumInt(_) => "int32".to_string(),
        DataType::Datetime(_) => "datetime".to_string(),
        // ~keep MSSQL's `BIT` has no width (`None`) and must keep resolving to `bool`.
        // See `bit_type_to_neutral` for the `BIT(n>1)` rationale (MySQL: `int64`
        // bitfield; PostgreSQL and others: `bytes` bit string).
        DataType::Bit(width) => match width {
            Some(w) if *w > 1 => {
                if catalog.dialect() == SqlDialect::MySQL {
                    "int64".to_string()
                } else {
                    "bytes".to_string()
                }
            }
            _ => "bool".to_string(),
        },
        DataType::BitVarying(_) | DataType::VarBit(_) => "bytes".to_string(),
        DataType::Enum(_, _) => "string".to_string(),
        DataType::Set(_) => "string".to_string(),
        DataType::TinyText => "string".to_string(),
        DataType::MediumText => "string".to_string(),
        DataType::LongText => "string".to_string(),
        DataType::Clob(_) => "string".to_string(),
        DataType::Date => "date".to_string(),
        DataType::Time(_, tz) => match tz {
            TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "time_tz".to_string(),
            _ => "time".to_string(),
        },
        DataType::Timestamp(_, tz) => match tz {
            TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "datetime_tz".to_string(),
            _ => "datetime".to_string(),
        },
        DataType::Interval { .. } => "interval".to_string(),
        DataType::JSON => "json".to_string(),
        DataType::JSONB => "json".to_string(),
        DataType::Array(elem) => {
            let inner = match elem {
                ast::ArrayElemTypeDef::SquareBracket(inner_dt, _) => datatype_to_neutral(inner_dt, catalog),
                ast::ArrayElemTypeDef::AngleBracket(inner_dt) => datatype_to_neutral(inner_dt, catalog),
                ast::ArrayElemTypeDef::Parenthesis(inner_dt) => datatype_to_neutral(inner_dt, catalog),
                ast::ArrayElemTypeDef::None => "unknown".to_string(),
            };
            format!("array<{}>", inner)
        }
        DataType::Custom(name, tokens) => {
            let raw = object_name_to_string(name).to_lowercase();
            match raw.as_str() {
                "timestamptz" => "datetime_tz".to_string(),
                "timetz" => "time_tz".to_string(),
                "serial" | "serial4" => "int32".to_string(),
                "bigserial" | "serial8" => "int64".to_string(),
                "smallserial" | "serial2" => "int16".to_string(),
                "timestamp_ntz" => "datetime".to_string(),
                "timestamp_ltz" => "datetime_tz".to_string(),
                "timestamp_tz" => "datetime_tz".to_string(),
                "variant" => "json".to_string(),
                // ~keep This AST-level Custom arm sees `tokens` directly from the
                // parser -- unlike `sql_type_to_neutral`'s bare `"number"` arm,
                // which only ever receives the already-collapsed `"number"`
                // string (upstream catalog normalization in
                // `catalog/type_normalizer.rs` discards precision-only tokens
                // before storing a column's `sql_type`, so a catalog column can
                // never reach this arm distinguishing bare `NUMBER` from
                // `NUMBER(p)`). This arm is reached instead for direct AST
                // resolution -- e.g. `CAST(x AS NUMBER)` expressions and query
                // parameter type inference -- where the token count is known, so
                // it implements the full three-way split documented on the
                // Oracle database docs page:
                //   - no tokens: bare `NUMBER` is Oracle's "floating" type, which
                //     can hold fractional values, so it maps to `decimal`.
                //   - one token (precision only, e.g. `NUMBER(38)`): implied
                //     scale 0, a true integer, so it maps to `int64`.
                //   - two-plus tokens (explicit scale): scale-aware, matching
                //     `number_type_to_neutral` -- nonzero scale is `decimal`,
                //     zero scale is `int64`. An unparseable scale token defaults
                //     to `decimal` (the safe, non-truncating choice).
                "number" => match tokens.len() {
                    0 => "decimal".to_string(),
                    1 => "int64".to_string(),
                    _ => {
                        let scale_is_nonzero = tokens[1].trim().parse::<i64>().map(|s| s > 0).unwrap_or(true);
                        if scale_is_nonzero {
                            "decimal".to_string()
                        } else {
                            "int64".to_string()
                        }
                    }
                },
                _ => sql_type_to_neutral(&raw, catalog).into_owned(),
            }
        }
        _ => {
            let s = dt.to_string().to_lowercase();
            sql_type_to_neutral(&s, catalog).into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_catalog() -> Catalog {
        Catalog::from_ddl(&[]).unwrap()
    }

    fn sqlite_catalog() -> Catalog {
        Catalog::from_ddl_with_dialect(&[], &SqlDialect::SQLite).unwrap()
    }

    fn mysql_catalog() -> Catalog {
        Catalog::from_ddl_with_dialect(&[], &SqlDialect::MySQL).unwrap()
    }

    fn mssql_catalog() -> Catalog {
        Catalog::from_ddl_with_dialect(&[], &SqlDialect::MsSql).unwrap()
    }

    fn snowflake_catalog() -> Catalog {
        Catalog::from_ddl_with_dialect(&[], &SqlDialect::Snowflake).unwrap()
    }

    fn oracle_catalog() -> Catalog {
        Catalog::from_ddl_with_dialect(&[], &SqlDialect::Oracle).unwrap()
    }

    /// Regression test for issue #83: DuckDB's `INT1` (a 1-byte `TINYINT` alias)
    /// had no matching arm at all and previously fell through to the unknown-type
    /// echo, which would later fail codegen with `BackendError::UnknownType`.
    /// DuckDB has no dedicated `SqlDialect` variant (it collapses to
    /// `SqlDialect::PostgreSQL`), so this is exercised under a plain PostgreSQL
    /// catalog and left ungated in the implementation, since PostgreSQL itself has
    /// no `INT1` spelling to collide with.
    #[test]
    fn test_duckdb_int1() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("int1", &c), "int16");

        let catalog = Catalog::from_ddl_with_dialect(&["CREATE TABLE t (a INT1);"], &SqlDialect::PostgreSQL).unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "int16");
    }

    /// Regression test for issue #83: DuckDB's 128-bit `HUGEINT`/`UHUGEINT` had no
    /// matching arm at all and previously fell through to the unknown-type echo,
    /// eventually failing codegen with `BackendError::UnknownType`. No neutral
    /// type is wide enough to hold the full 128-bit range, so both map to
    /// `decimal` (arbitrary precision) rather than a lossy, silently-truncating
    /// `int64`. `sqlparser` gives `HUGEINT`/`UHUGEINT` dedicated `DataType`
    /// variants (`DataType::HugeInt`/`DataType::UHugeInt`), so this also verifies
    /// the AST entry point, which falls through to `sql_type_to_neutral` via the
    /// catch-all stringify-and-delegate arm.
    #[test]
    fn test_duckdb_hugeint_uhugeint_map_to_decimal() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("hugeint", &c), "decimal");
        assert_eq!(sql_type_to_neutral("uhugeint", &c), "decimal");
        assert_eq!(datatype_to_neutral(&DataType::HugeInt, &c), "decimal");
        assert_eq!(datatype_to_neutral(&DataType::UHugeInt, &c), "decimal");

        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (a HUGEINT, b UHUGEINT);"], &SqlDialect::PostgreSQL)
                .unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "decimal");
        assert_eq!(sql_type_to_neutral(&table.columns[1].sql_type, &catalog), "decimal");
    }

    /// Regression test for issue #83: neither MSSQL's `MONEY`/`SMALLMONEY` nor
    /// PostgreSQL's `MONEY` had a matching arm, both previously falling through to
    /// the unknown-type echo and eventually failing codegen with
    /// `BackendError::UnknownType`. Both are fixed-point currency types, so both
    /// engines share the same ungated `decimal` mapping.
    #[test]
    fn test_money_and_smallmoney_map_to_decimal() {
        let mssql = mssql_catalog();
        assert_eq!(sql_type_to_neutral("money", &mssql), "decimal");
        assert_eq!(sql_type_to_neutral("smallmoney", &mssql), "decimal");

        let postgres = empty_catalog();
        assert_eq!(sql_type_to_neutral("money", &postgres), "decimal");

        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (a MONEY, b SMALLMONEY);"], &SqlDialect::MsSql).unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "decimal");
        assert_eq!(sql_type_to_neutral(&table.columns[1].sql_type, &catalog), "decimal");
    }

    /// Regression test for issue #83: MSSQL's `XML` column type had no matching
    /// arm and previously fell through to the unknown-type echo, eventually
    /// failing codegen with `BackendError::UnknownType`.
    #[test]
    fn test_mssql_xml_maps_to_string() {
        let mssql = mssql_catalog();
        assert_eq!(sql_type_to_neutral("xml", &mssql), "string");

        let catalog = Catalog::from_ddl_with_dialect(&["CREATE TABLE t (a XML);"], &SqlDialect::MsSql).unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "string");
    }

    /// Regression test for the `NUMBER(p,s)` truncation bug (issue #83 follow-up):
    /// a direct `sql_type_to_neutral` call with an explicit-scale Oracle/Snowflake
    /// `NUMBER(10,2)` string previously resolved to `int64` via the bare
    /// `"number"` arm (scale discarded by `strip_precision`), silently truncating
    /// money-shaped columns. It must now resolve to `decimal`.
    ///
    /// Confirmed via `Catalog::from_ddl_with_dialect` that the full DDL pipeline
    /// was NOT affected by this bug for `NUMBER(p,s)` with `s > 0`: upstream
    /// catalog normalization (`normalize_data_type` in
    /// `catalog/type_normalizer.rs`) already rewrites `NUMBER(10,2)` to
    /// `"numeric(10,2)"` before it reaches this module, which the existing
    /// `"numeric" | "decimal"` arm handles correctly. This test still fixes the
    /// direct string-call path, which any future non-DDL caller (e.g. live
    /// catalog introspection) could hit.
    #[test]
    fn test_oracle_number_precision_scale_bug() {
        let oracle = oracle_catalog();
        assert_eq!(sql_type_to_neutral("number(10,2)", &oracle), "decimal");
        assert_eq!(sql_type_to_neutral("number(38,4)", &oracle), "decimal");

        let snowflake = snowflake_catalog();
        assert_eq!(sql_type_to_neutral("number(10,2)", &snowflake), "decimal");

        // ~keep Also verify end-to-end through the DDL path, per the task instructions:
        // this must have been correct already (not a regression fix, a
        // confirmation), since upstream normalization already rewrites this to
        // `numeric(10,2)`.
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (price NUMBER(10,2));"], &SqlDialect::Oracle).unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "decimal");
    }

    /// `NUMBER(p,0)` (explicit zero scale) and bare `NUMBER(p)`/`NUMBER` must stay
    /// `int64`, not `decimal` -- a zero scale is a true integer, and Oracle's own
    /// `NUMBER(1)` idiom (used as a boolean-flag column in this repo's Oracle
    /// fixtures, e.g. `integration_tests/sql/oracle/schema.sql`) must not
    /// silently become a decimal string.
    #[test]
    fn test_oracle_number_zero_scale_and_bare_stay_int64() {
        let oracle = oracle_catalog();
        assert_eq!(sql_type_to_neutral("number(10,0)", &oracle), "int64");
        assert_eq!(sql_type_to_neutral("number(1)", &oracle), "int64");
        assert_eq!(sql_type_to_neutral("number", &oracle), "int64");
    }

    /// The assertions above pass a hand-built string straight to
    /// `sql_type_to_neutral`, which is not the spelling the DDL path produces:
    /// `normalize_data_type` rewrote every two-token `NUMBER(p,s)` to
    /// `numeric(p,s)` regardless of the scale, so an explicit zero scale reached
    /// the decimal arm and a real schema resolved to `decimal` while these unit
    /// assertions stayed green. Drive the whole pipeline instead.
    ///
    /// `NUMBER(38,0)` matters most: it is what Snowflake's `DESCRIBE TABLE`
    /// reports for `INT`, so a schema reverse-engineered from a live table typed
    /// its keys `decimal` while the same table written with `INT` typed them
    /// `int64` -- the spelling-dependent inconsistency issue #83 set out to end.
    #[test]
    fn test_number_zero_scale_through_the_ddl_path_is_int64() {
        let ddl = "CREATE TABLE t (a NUMBER(38,0), b NUMBER(10,0), c NUMBER(1), d NUMBER, e NUMBER(10,2));";
        for dialect in [SqlDialect::Oracle, SqlDialect::Snowflake] {
            let catalog = Catalog::from_ddl_with_dialect(&[ddl], &dialect).unwrap();
            let table = catalog.get_table("t").unwrap();
            let neutral = |i: usize| sql_type_to_neutral(&table.columns[i].sql_type, &catalog).into_owned();

            for (index, column) in [(0, "a"), (1, "b"), (2, "c"), (3, "d")] {
                assert_eq!(neutral(index), "int64", "{dialect:?} column {column} must be int64");
            }
            // A non-zero scale is still a decimal -- the guard must not swallow it.
            assert_eq!(neutral(4), "decimal", "{dialect:?} NUMBER(10,2) must stay decimal");
        }
    }

    /// `snowflake.md` has always documented `GEOGRAPHY`/`GEOMETRY` as `string`,
    /// but no arm existed, so they reached the backend as an invalid neutral type.
    #[test]
    fn test_snowflake_spatial_types_are_string() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (a GEOGRAPHY, b GEOMETRY);"], &SqlDialect::Snowflake)
                .unwrap();
        let table = catalog.get_table("t").unwrap();
        for column in &table.columns {
            assert_eq!(
                sql_type_to_neutral(&column.sql_type, &catalog),
                "string",
                "column {}",
                column.name
            );
        }
    }

    /// `BYTEINT` is Snowflake's synonym for `TINYINT`. sqlparser has no keyword
    /// for it, so it survives as a raw custom type; with no arm it reached the
    /// backend as the invalid neutral type "byteint" and failed codegen outright.
    #[test]
    fn test_snowflake_byteint_is_int64() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (flag BYTEINT);"], &SqlDialect::Snowflake).unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "int64");

        // Everywhere else it keeps the narrow width its name implies.
        let mysql = Catalog::from_ddl_with_dialect(&["CREATE TABLE t (flag BYTEINT);"], &SqlDialect::MySQL).unwrap();
        let table = mysql.get_table("t").unwrap();
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &mysql), "int16");
    }

    /// AST-based entry point (`datatype_to_neutral`'s `DataType::Custom` arm) must
    /// stay consistent with `sql_type_to_neutral`'s scale-aware handling above.
    #[test]
    fn test_oracle_number_datatype_custom_scale_aware() {
        let oracle = oracle_catalog();
        let name = ast::ObjectName(vec![ast::ObjectNamePart::Identifier(ast::Ident::new("NUMBER"))]);
        let with_scale = DataType::Custom(name.clone(), vec!["10".to_string(), "2".to_string()]);
        assert_eq!(datatype_to_neutral(&with_scale, &oracle), "decimal");

        let zero_scale = DataType::Custom(name.clone(), vec!["10".to_string(), "0".to_string()]);
        assert_eq!(datatype_to_neutral(&zero_scale, &oracle), "int64");

        let precision_only = DataType::Custom(name.clone(), vec!["10".to_string()]);
        assert_eq!(datatype_to_neutral(&precision_only, &oracle), "int64");

        // ~keep Bare `NUMBER` (no tokens at all) is Oracle's "floating" type, which can
        // hold fractional values, so it maps to `decimal` -- distinct from the
        // `int64` that `sql_type_to_neutral`'s bare `"number"` string arm gives,
        // because that string-based arm cannot distinguish a genuinely bare
        // `NUMBER` from a `NUMBER(p)` collapsed by upstream catalog
        // normalization (see the comment on that arm). This AST-level arm has
        // the real token count and can make the distinction correctly.
        let bare = DataType::Custom(name, vec![]);
        assert_eq!(datatype_to_neutral(&bare, &oracle), "decimal");
    }

    #[test]
    fn test_integer_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("integer", &c), "int32");
        assert_eq!(sql_type_to_neutral("int", &c), "int32");
        assert_eq!(sql_type_to_neutral("int4", &c), "int32");
        assert_eq!(sql_type_to_neutral("serial", &c), "int32");
        assert_eq!(sql_type_to_neutral("smallint", &c), "int16");
        assert_eq!(sql_type_to_neutral("int2", &c), "int16");
        assert_eq!(sql_type_to_neutral("smallserial", &c), "int16");
        assert_eq!(sql_type_to_neutral("bigint", &c), "int64");
        assert_eq!(sql_type_to_neutral("int8", &c), "int64");
        assert_eq!(sql_type_to_neutral("bigserial", &c), "int64");
    }

    #[test]
    fn test_float_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("real", &c), "float32");
        assert_eq!(sql_type_to_neutral("float4", &c), "float32");
        assert_eq!(sql_type_to_neutral("double precision", &c), "float64");
        assert_eq!(sql_type_to_neutral("float8", &c), "float64");
        assert_eq!(sql_type_to_neutral("numeric", &c), "decimal");
        assert_eq!(sql_type_to_neutral("decimal", &c), "decimal");
    }

    #[test]
    fn test_string_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("text", &c), "string");
        assert_eq!(sql_type_to_neutral("varchar", &c), "string");
        assert_eq!(sql_type_to_neutral("character varying", &c), "string");
        assert_eq!(sql_type_to_neutral("character", &c), "string");
        assert_eq!(sql_type_to_neutral("char", &c), "string");
    }

    #[test]
    fn test_boolean() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("boolean", &c), "bool");
        assert_eq!(sql_type_to_neutral("bool", &c), "bool");
    }

    #[test]
    fn test_temporal_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("timestamp", &c), "datetime");
        assert_eq!(sql_type_to_neutral("timestamp without time zone", &c), "datetime");
        assert_eq!(sql_type_to_neutral("timestamp with time zone", &c), "datetime_tz");
        assert_eq!(sql_type_to_neutral("timestamptz", &c), "datetime_tz");
        assert_eq!(sql_type_to_neutral("date", &c), "date");
        assert_eq!(sql_type_to_neutral("time", &c), "time");
        assert_eq!(sql_type_to_neutral("time without time zone", &c), "time");
        assert_eq!(sql_type_to_neutral("time with time zone", &c), "time_tz");
        assert_eq!(sql_type_to_neutral("timetz", &c), "time_tz");
        assert_eq!(sql_type_to_neutral("interval", &c), "interval");
    }

    #[test]
    fn test_binary_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("bytea", &c), "bytes");
    }

    #[test]
    fn test_json_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("json", &c), "json");
        assert_eq!(sql_type_to_neutral("jsonb", &c), "json");
    }

    #[test]
    fn test_uuid() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("uuid", &c), "uuid");
    }

    #[test]
    fn test_network_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("inet", &c), "inet");
        assert_eq!(sql_type_to_neutral("cidr", &c), "inet");
        assert_eq!(sql_type_to_neutral("macaddr", &c), "inet");
    }

    #[test]
    fn test_array_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("integer[]", &c), "array<int32>");
        assert_eq!(sql_type_to_neutral("int4[]", &c), "array<int32>");
        assert_eq!(sql_type_to_neutral("int[]", &c), "array<int32>");
        assert_eq!(sql_type_to_neutral("text[]", &c), "array<string>");
        assert_eq!(sql_type_to_neutral("boolean[]", &c), "array<bool>");
        assert_eq!(sql_type_to_neutral("bool[]", &c), "array<bool>");
        assert_eq!(sql_type_to_neutral("bigint[]", &c), "array<int64>");
        assert_eq!(sql_type_to_neutral("uuid[]", &c), "array<uuid>");
        assert_eq!(sql_type_to_neutral("jsonb[]", &c), "array<json>");
        assert_eq!(sql_type_to_neutral("numeric[]", &c), "array<decimal>");
    }

    #[test]
    fn test_range_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("int4range", &c), "range<int32>");
        assert_eq!(sql_type_to_neutral("int8range", &c), "range<int64>");
        assert_eq!(sql_type_to_neutral("tstzrange", &c), "range<datetime_tz>");
        assert_eq!(sql_type_to_neutral("tsrange", &c), "range<datetime>");
        assert_eq!(sql_type_to_neutral("daterange", &c), "range<date>");
        assert_eq!(sql_type_to_neutral("numrange", &c), "range<decimal>");
    }

    #[test]
    fn test_unknown_type() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("somecustomtype", &c), "somecustomtype");
        assert_eq!(sql_type_to_neutral("hstore", &c), "hstore");
    }

    #[test]
    fn test_case_insensitive() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("INTEGER", &c), "int32");
        assert_eq!(sql_type_to_neutral("Text", &c), "string");
        assert_eq!(sql_type_to_neutral("BOOLEAN", &c), "bool");
        assert_eq!(sql_type_to_neutral("TIMESTAMP WITH TIME ZONE", &c), "datetime_tz");
    }

    #[test]
    fn test_strip_precision() {
        assert_eq!(
            strip_precision("timestamp with time zone(6)"),
            "timestamp with time zone"
        );
        assert_eq!(strip_precision("numeric(10,2)"), "numeric");
        assert_eq!(strip_precision("varchar(255)"), "varchar");
        assert_eq!(strip_precision("foo(bar)"), "foo(bar)");
        assert_eq!(strip_precision("integer"), "integer");
    }

    #[test]
    fn test_type_with_precision() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("numeric(10,2)", &c), "decimal");
        assert_eq!(sql_type_to_neutral("timestamp with time zone(6)", &c), "datetime_tz");
    }

    #[test]
    fn test_enum_type_lookup() {
        let c = Catalog::from_ddl(&["CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');"]).unwrap();
        assert_eq!(sql_type_to_neutral("mood", &c), "enum::mood");
    }

    #[test]
    fn test_composite_type_lookup() {
        let c = Catalog::from_ddl(&["CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);"]).unwrap();
        assert_eq!(sql_type_to_neutral("address", &c), "composite::address");
    }

    #[test]
    fn test_generic_array_fallback() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("timestamptz[]", &c), "array<datetime_tz>");
    }

    #[test]
    fn test_snowflake_timestamp_types() {
        let c = snowflake_catalog();
        assert_eq!(sql_type_to_neutral("timestamp_ntz", &c), "datetime");
        assert_eq!(sql_type_to_neutral("timestamp_ltz", &c), "datetime_tz");
        assert_eq!(sql_type_to_neutral("timestamp_tz", &c), "datetime_tz");
    }

    #[test]
    fn test_snowflake_variant_type() {
        let c = snowflake_catalog();
        assert_eq!(sql_type_to_neutral("variant", &c), "json");
    }

    /// Regression test for issue #83: Snowflake stores every integer type as
    /// `NUMBER(38,0)`, so `SMALLINT`/`TINYINT` must resolve to `int64` -- the same
    /// neutral type the bare `NUMBER` spelling already resolves to -- rather than
    /// the narrower `int16` other dialects use. Before this fix, Snowflake typed
    /// the identical underlying storage as `int64` via one spelling and `int16`
    /// via the other, with no Snowflake-gated arm at all (the ungated `smallint`/
    /// `tinyint` arms silently applied). `INT`/`INTEGER`/`BIGINT` are unaffected:
    /// they already resolve to `int64`/`int32` consistent with `NUMBER(38,0)` via
    /// the existing ungated arms and are not part of this fix.
    #[test]
    fn test_snowflake_integer_family_is_all_int64() {
        // ~keep Snowflake aliases every integer spelling to NUMBER(38,0), so the whole
        // family must agree. Fixing only SMALLINT/TINYINT would leave INT/INTEGER
        // reporting int32 for the same underlying storage.
        let c = snowflake_catalog();
        for spelling in ["smallint", "tinyint", "int", "integer", "bigint", "number"] {
            assert_eq!(sql_type_to_neutral(spelling, &c), "int64", "snowflake {spelling}");
        }
        assert_eq!(datatype_to_neutral(&DataType::SmallInt(None), &c), "int64");
        assert_eq!(datatype_to_neutral(&DataType::TinyInt(None), &c), "int64");
        assert_eq!(datatype_to_neutral(&DataType::Int(None), &c), "int64");
        assert_eq!(datatype_to_neutral(&DataType::Integer(None), &c), "int64");

        // Other dialects are unaffected: PostgreSQL's SMALLINT and INT genuinely
        // are narrower types.
        let postgres = empty_catalog();
        assert_eq!(sql_type_to_neutral("smallint", &postgres), "int16");
        assert_eq!(sql_type_to_neutral("tinyint", &postgres), "int16");
        assert_eq!(sql_type_to_neutral("int", &postgres), "int32");
        assert_eq!(sql_type_to_neutral("integer", &postgres), "int32");
        assert_eq!(datatype_to_neutral(&DataType::Int(None), &postgres), "int32");

        // SQLite's own INT widening is untouched by the Snowflake arm.
        let sqlite = sqlite_catalog();
        assert_eq!(sql_type_to_neutral("int", &sqlite), "int64");
    }

    /// Regression test for issue #83: Snowflake's `REAL`/`FLOAT4` are aliases for
    /// the same 8-byte `DOUBLE` as every other Snowflake float spelling
    /// (`FLOAT`/`FLOAT8`/`DOUBLE`, which already resolved to `float64` via the
    /// existing ungated arms). Before this fix there was no Snowflake-gated arm,
    /// so `REAL`/`FLOAT4` silently narrowed to `float32`.
    #[test]
    fn test_snowflake_real_float4_are_float64() {
        let c = snowflake_catalog();
        assert_eq!(sql_type_to_neutral("real", &c), "float64");
        assert_eq!(sql_type_to_neutral("float4", &c), "float64");
        assert_eq!(datatype_to_neutral(&DataType::Real, &c), "float64");
        assert_eq!(datatype_to_neutral(&DataType::Float4, &c), "float64");

        // Other dialects are unaffected: PostgreSQL's REAL genuinely is float32.
        let postgres = empty_catalog();
        assert_eq!(sql_type_to_neutral("real", &postgres), "float32");
        assert_eq!(sql_type_to_neutral("float4", &postgres), "float32");
    }

    #[test]
    fn test_mysql_integer_types() {
        let c = mysql_catalog();
        assert_eq!(sql_type_to_neutral("tinyint", &c), "int16");
        assert_eq!(sql_type_to_neutral("mediumint", &c), "int32");
    }

    #[test]
    fn test_mysql_string_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("tinytext", &c), "string");
        assert_eq!(sql_type_to_neutral("mediumtext", &c), "string");
        assert_eq!(sql_type_to_neutral("longtext", &c), "string");
        assert_eq!(sql_type_to_neutral("set", &c), "string");
    }

    #[test]
    fn test_mysql_binary_types() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("blob", &c), "bytes");
        assert_eq!(sql_type_to_neutral("tinyblob", &c), "bytes");
        assert_eq!(sql_type_to_neutral("mediumblob", &c), "bytes");
        assert_eq!(sql_type_to_neutral("longblob", &c), "bytes");
        assert_eq!(sql_type_to_neutral("binary", &c), "bytes");
        assert_eq!(sql_type_to_neutral("varbinary", &c), "bytes");
    }

    #[test]
    fn test_mysql_datetime() {
        let c = empty_catalog();
        assert_eq!(sql_type_to_neutral("datetime", &c), "datetime");
        assert_eq!(sql_type_to_neutral("year", &c), "int16");
    }

    #[test]
    fn test_mysql_float_types() {
        let c = mysql_catalog();
        // MySQL's bare `FLOAT` is genuinely 4-byte (see
        // `test_bare_float_is_float64_except_mysql` for the cross-dialect default).
        assert_eq!(sql_type_to_neutral("float", &c), "float32");
        assert_eq!(sql_type_to_neutral("double", &c), "float64");
    }

    #[test]
    fn test_mysql_bit_type() {
        let c = mysql_catalog();
        assert_eq!(sql_type_to_neutral("bit", &c), "bool");
    }

    /// Regression note: this test previously ran against `empty_catalog()`
    /// (PostgreSQL), which never exercised SQLite's dialect-specific `INTEGER`
    /// handling and would not have caught
    /// https://github.com/Goldziher/scythe/issues/70. Fixing the catalog to
    /// `sqlite_catalog()` changes the expected value for `"integer"` from
    /// `"int32"` to `"int64"` — that value change is the point of the fix, not a
    /// loosened assertion.
    #[test]
    fn test_sqlite_types() {
        let c = sqlite_catalog();
        assert_eq!(sql_type_to_neutral("integer", &c), "int64");
        assert_eq!(sql_type_to_neutral("text", &c), "string");
        assert_eq!(sql_type_to_neutral("blob", &c), "bytes");
        assert_eq!(sql_type_to_neutral("numeric", &c), "decimal");
        assert_eq!(sql_type_to_neutral("clob", &c), "string");
    }

    /// SQLite's `REAL` storage class is always an 8-byte IEEE float (SQLite has no
    /// 4-byte float type) and must resolve to `float64`, while PostgreSQL's `real`
    /// (aka `float4`) is genuinely 4 bytes and must stay `float32`. Regression test
    /// for https://github.com/Goldziher/scythe/issues/70.
    #[test]
    fn test_sqlite_real_is_float64_postgres_real_stays_float32() {
        let sqlite = Catalog::from_ddl_with_dialect(&[], &SqlDialect::SQLite).unwrap();
        assert_eq!(sql_type_to_neutral("real", &sqlite), "float64");
        assert_eq!(sql_type_to_neutral("float4", &sqlite), "float64");
        assert_eq!(datatype_to_neutral(&DataType::Real, &sqlite), "float64");
        assert_eq!(datatype_to_neutral(&DataType::Float4, &sqlite), "float64");

        let postgres = empty_catalog();
        assert_eq!(sql_type_to_neutral("real", &postgres), "float32");
        assert_eq!(sql_type_to_neutral("float4", &postgres), "float32");
        assert_eq!(datatype_to_neutral(&DataType::Real, &postgres), "float32");
        assert_eq!(datatype_to_neutral(&DataType::Float4, &postgres), "float32");
    }

    /// SQLite's `INTEGER` storage class always holds up to 8 bytes; there is no
    /// narrower 4-byte integer type distinguishable from it. `INT`/`INTEGER` are
    /// genuine SQLite spellings and must resolve to `int64`, while PostgreSQL's
    /// `integer`/`int` genuinely is 4 bytes and must stay `int32`. Regression test
    /// for https://github.com/Goldziher/scythe/issues/70.
    #[test]
    fn test_sqlite_integer_is_int64_postgres_integer_stays_int32() {
        let sqlite = sqlite_catalog();
        assert_eq!(sql_type_to_neutral("integer", &sqlite), "int64");
        assert_eq!(sql_type_to_neutral("int", &sqlite), "int64");
        assert_eq!(datatype_to_neutral(&DataType::Integer(None), &sqlite), "int64");
        assert_eq!(datatype_to_neutral(&DataType::Int(None), &sqlite), "int64");

        let postgres = empty_catalog();
        assert_eq!(sql_type_to_neutral("integer", &postgres), "int32");
        assert_eq!(sql_type_to_neutral("int", &postgres), "int32");
        assert_eq!(datatype_to_neutral(&DataType::Integer(None), &postgres), "int32");
        assert_eq!(datatype_to_neutral(&DataType::Int(None), &postgres), "int32");
    }

    /// `int4`/`serial` are PostgreSQL-only spellings that would not appear in
    /// genuine SQLite DDL; they intentionally stay `int32` even under a SQLite
    /// catalog rather than risk masking a mixed-dialect mistake.
    #[test]
    fn test_sqlite_int4_and_serial_spellings_stay_int32() {
        let sqlite = sqlite_catalog();
        assert_eq!(sql_type_to_neutral("int4", &sqlite), "int32");
        assert_eq!(sql_type_to_neutral("serial", &sqlite), "int32");
        assert_eq!(datatype_to_neutral(&DataType::Int4(None), &sqlite), "int32");
    }

    /// A bare `FLOAT` (no precision) is 8-byte double precision on every engine
    /// except MySQL, where it is genuinely 4-byte. Regression test for
    /// https://github.com/Goldziher/scythe/issues/73 (the issue's own diagnosis
    /// about `FLOAT(53)` narrowing was incorrect — that path was already correct;
    /// the real bugs were the bare-`FLOAT` default here and in
    /// `normalize_data_type`).
    #[test]
    fn test_bare_float_is_float64_except_mysql() {
        let postgres = empty_catalog();
        assert_eq!(sql_type_to_neutral("float", &postgres), "float64");
        assert_eq!(
            datatype_to_neutral(&DataType::Float(sqlparser::ast::ExactNumberInfo::None), &postgres),
            "float64"
        );

        let mssql = mssql_catalog();
        assert_eq!(sql_type_to_neutral("float", &mssql), "float64");

        let mysql = mysql_catalog();
        assert_eq!(sql_type_to_neutral("float", &mysql), "float32");
        assert_eq!(
            datatype_to_neutral(&DataType::Float(sqlparser::ast::ExactNumberInfo::None), &mysql),
            "float32"
        );
    }

    /// MySQL's `UNSIGNED` numeric qualifier has no dedicated neutral type and
    /// previously had no matching arm at all, causing codegen to fail with
    /// `BackendError::UnknownType` on an ordinary MySQL schema. It now maps to the
    /// same-width signed neutral type. Regression test for
    /// https://github.com/Goldziher/scythe/issues/74.
    #[test]
    fn test_mysql_unsigned_integer_types_map_to_signed_neutral() {
        let mysql = mysql_catalog();
        assert_eq!(sql_type_to_neutral("tinyint unsigned", &mysql), "int16");
        assert_eq!(sql_type_to_neutral("smallint unsigned", &mysql), "int16");
        assert_eq!(sql_type_to_neutral("mediumint unsigned", &mysql), "int32");
        assert_eq!(sql_type_to_neutral("int unsigned", &mysql), "int32");
        assert_eq!(sql_type_to_neutral("bigint unsigned", &mysql), "int64");

        assert_eq!(datatype_to_neutral(&DataType::TinyIntUnsigned(None), &mysql), "int16");
        assert_eq!(datatype_to_neutral(&DataType::SmallIntUnsigned(None), &mysql), "int16");
        assert_eq!(datatype_to_neutral(&DataType::MediumIntUnsigned(None), &mysql), "int32");
        assert_eq!(datatype_to_neutral(&DataType::IntUnsigned(None), &mysql), "int32");
        assert_eq!(datatype_to_neutral(&DataType::BigIntUnsigned(None), &mysql), "int64");
    }

    /// End-to-end regression through the catalog: a MySQL `BIGINT(20) UNSIGNED`
    /// column previously normalized to the unclean string `"bigint(20) unsigned"`
    /// (because `strip_precision` cannot rescue a width that isn't in a trailing
    /// paren group) and had no matching neutral-type arm either way, dying as
    /// `BackendError::UnknownType` downstream. It must now normalize to the clean
    /// `"bigint unsigned"` string and resolve to `int64`. Regression test for
    /// https://github.com/Goldziher/scythe/issues/74.
    #[test]
    fn test_mysql_unsigned_bigint_with_display_width_does_not_error() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (a BIGINT(20) UNSIGNED);"], &SqlDialect::MySQL).unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].sql_type, "bigint unsigned");
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "int64");
    }

    /// `BIT` semantics differ sharply by engine: MSSQL's `BIT` has no width and is
    /// always a 1-byte boolean (must not change — 11 downstream harnesses depend on
    /// it); PostgreSQL's `BIT(n)`/`BIT VARYING` are true bit strings; MySQL's
    /// `BIT(n)` with n>1 is an integer bitfield. Only `BIT`/`BIT(1)` is
    /// boolean-ish. Regression test for
    /// https://github.com/Goldziher/scythe/issues/75.
    #[test]
    fn test_bit_type_by_dialect_and_width() {
        let mssql = mssql_catalog();
        assert_eq!(sql_type_to_neutral("bit", &mssql), "bool");
        assert_eq!(datatype_to_neutral(&DataType::Bit(None), &mssql), "bool");

        let mysql = mysql_catalog();
        assert_eq!(sql_type_to_neutral("bit(1)", &mysql), "bool");
        assert_eq!(sql_type_to_neutral("bit(8)", &mysql), "int64");
        assert_eq!(datatype_to_neutral(&DataType::Bit(Some(8)), &mysql), "int64");

        let postgres = empty_catalog();
        assert_eq!(sql_type_to_neutral("bit(1)", &postgres), "bool");
        assert_eq!(sql_type_to_neutral("bit(8)", &postgres), "bytes");
        assert_eq!(datatype_to_neutral(&DataType::Bit(Some(8)), &postgres), "bytes");

        assert_eq!(sql_type_to_neutral("bit varying", &postgres), "bytes");
        assert_eq!(sql_type_to_neutral("varbit", &postgres), "bytes");
        assert_eq!(datatype_to_neutral(&DataType::BitVarying(None), &postgres), "bytes");
        assert_eq!(datatype_to_neutral(&DataType::VarBit(None), &postgres), "bytes");
    }

    /// End-to-end regression through the catalog: MSSQL's `BIT` column (as used by
    /// `integration_tests/sql/mssql/schema.sql`, which feeds 11 downstream
    /// harnesses) must keep resolving to `bool` — the width-sensitive
    /// PostgreSQL/MySQL `BIT(n)` handling must not affect it.
    #[test]
    fn test_mssql_bit_ddl_stays_bool() {
        let catalog =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE t (active BIT NOT NULL DEFAULT 1);"], &SqlDialect::MsSql)
                .unwrap();
        let table = catalog.get_table("t").unwrap();
        assert_eq!(table.columns[0].sql_type, "bit");
        assert_eq!(sql_type_to_neutral(&table.columns[0].sql_type, &catalog), "bool");
    }
}
