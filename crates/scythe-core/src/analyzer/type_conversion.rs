use std::borrow::Cow;

use sqlparser::ast::{self, DataType, TimezoneInfo};

use crate::catalog::Catalog;
use crate::dialect::SqlDialect;

use super::helpers::object_name_to_string;

pub(super) fn sql_type_to_neutral(sql_type: &str, catalog: &Catalog) -> Cow<'static, str> {
    let lower = sql_type.to_lowercase();
    let normalized = strip_precision(&lower);

    if let Some(neutral) = bit_type_to_neutral(&lower, catalog.dialect()) {
        return neutral;
    }

    match normalized.as_str() {
        // SQLite's `INTEGER` storage class holds up to 8 bytes; there is no
        // narrower 4-byte integer type. `INT`/`INTEGER` are genuine SQLite
        // spellings so both are widened here. `INT4`/`SERIAL` are PostgreSQL-only
        // spellings that would not appear in real SQLite DDL, so they are left
        // ungated below rather than risk masking a mixed-dialect mistake.
        "integer" | "int" if catalog.dialect() == SqlDialect::SQLite => Cow::Borrowed("int64"),
        "integer" | "int" | "int4" | "serial" => Cow::Borrowed("int32"),
        "smallint" | "int2" | "smallserial" => Cow::Borrowed("int16"),
        "bigint" | "int8" | "bigserial" => Cow::Borrowed("int64"),
        "tinyint" => Cow::Borrowed("int16"),
        "mediumint" => Cow::Borrowed("int32"),
        "number" => Cow::Borrowed("int64"),

        // SQLite's `REAL` storage class is always an 8-byte IEEE float (SQLite has no
        // 4-byte float type), unlike PostgreSQL's 4-byte `real`/`float4`.
        "real" | "float4" if catalog.dialect() == SqlDialect::SQLite => Cow::Borrowed("float64"),
        "real" | "float4" => Cow::Borrowed("float32"),
        "double precision" | "float8" | "double" => Cow::Borrowed("float64"),
        // MySQL's bare `FLOAT` (no precision) is a genuine 4-byte type; every other
        // engine here defaults bare `FLOAT` to 8-byte double precision (equivalent
        // to `FLOAT(53)`).
        "float" if catalog.dialect() == SqlDialect::MySQL => Cow::Borrowed("float32"),
        "float" => Cow::Borrowed("float64"),
        "numeric" | "decimal" => Cow::Borrowed("decimal"),

        // MySQL's `UNSIGNED` numeric qualifier has no dedicated neutral type, so it
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

        // Bare `BIT` (no width) is `BIT(1)` per the SQL standard, which is
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
        DataType::Int(_) | DataType::Int4(_) | DataType::Integer(_) => "int32".to_string(),
        DataType::SmallInt(_) | DataType::Int2(_) => "int16".to_string(),
        DataType::BigInt(_) | DataType::Int8(_) => "int64".to_string(),
        // See the matching comment in `sql_type_to_neutral`: SQLite's `REAL` is always
        // an 8-byte float, unlike PostgreSQL's 4-byte `real`/`float4`.
        DataType::Real | DataType::Float4 if catalog.dialect() == SqlDialect::SQLite => "float64".to_string(),
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
        // MSSQL's `BIT` has no width (`None`) and must keep resolving to `bool`.
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
                "number" if tokens.len() >= 2 => "decimal".to_string(),
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
