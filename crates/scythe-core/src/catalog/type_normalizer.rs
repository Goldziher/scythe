use ahash::AHashMap;
use sqlparser::ast::{ArrayElemTypeDef, DataType, ObjectName, TimezoneInfo};

use super::DomainDef;

/// Normalize a sqlparser DataType into a lowercase PostgreSQL type string.
/// Returns (type_string, is_serial).
pub(crate) fn normalize_data_type(
    dt: &DataType,
    domains: &AHashMap<String, DomainDef>,
) -> (String, bool) {
    match dt {
        // Custom types (includes serial, timestamptz, and user-defined types)
        DataType::Custom(name, _tokens) => {
            let raw = object_name_to_key(name);
            match raw.as_str() {
                "serial" | "serial4" => return ("integer".to_string(), true),
                "bigserial" | "serial8" => return ("bigint".to_string(), true),
                "smallserial" | "serial2" => return ("smallint".to_string(), true),
                "timestamptz" => return ("timestamptz".to_string(), false),
                "timetz" => return ("timetz".to_string(), false),
                _ => {}
            }
            // Check if it's a domain
            if let Some(domain) = domains.get(&raw) {
                return (domain.base_type.clone(), domain.not_null);
            }
            (raw, false)
        }

        // Integer family
        DataType::Int(_) | DataType::Int4(_) | DataType::Integer(_) => {
            ("integer".to_string(), false)
        }
        DataType::SmallInt(_) | DataType::Int2(_) => ("smallint".to_string(), false),
        DataType::BigInt(_) | DataType::Int8(_) => ("bigint".to_string(), false),

        // Boolean
        DataType::Bool | DataType::Boolean => ("boolean".to_string(), false),

        // Float family
        DataType::Real | DataType::Float4 => ("real".to_string(), false),
        DataType::DoublePrecision | DataType::Float8 => ("double precision".to_string(), false),
        DataType::Float(info) => {
            use sqlparser::ast::ExactNumberInfo;
            match info {
                ExactNumberInfo::Precision(p) if *p <= 24 => ("real".to_string(), false),
                ExactNumberInfo::Precision(_) | ExactNumberInfo::PrecisionAndScale(_, _) => {
                    ("double precision".to_string(), false)
                }
                ExactNumberInfo::None => ("double precision".to_string(), false),
            }
        }

        // Character types
        DataType::Varchar(len) | DataType::CharVarying(len) | DataType::CharacterVarying(len) => {
            match len {
                Some(sqlparser::ast::CharacterLength::IntegerLength { length, .. }) => {
                    (format!("varchar({})", length), false)
                }
                _ => ("text".to_string(), false),
            }
        }
        DataType::Char(len) | DataType::Character(len) => match len {
            Some(sqlparser::ast::CharacterLength::IntegerLength { length, .. }) => {
                (format!("char({})", length), false)
            }
            _ => ("char(1)".to_string(), false),
        },
        DataType::Text => ("text".to_string(), false),

        // Numeric
        DataType::Numeric(info) | DataType::Decimal(info) | DataType::Dec(info) => {
            use sqlparser::ast::ExactNumberInfo;
            match info {
                ExactNumberInfo::PrecisionAndScale(p, s) => {
                    (format!("numeric({},{})", p, s), false)
                }
                ExactNumberInfo::Precision(p) => (format!("numeric({})", p), false),
                ExactNumberInfo::None => ("numeric".to_string(), false),
            }
        }

        // Date/time
        DataType::Date => ("date".to_string(), false),
        DataType::Time(prec, tz) => {
            let base = match tz {
                TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "timetz",
                TimezoneInfo::WithoutTimeZone | TimezoneInfo::None => "time",
            };
            match prec {
                Some(p) => (format!("{}({})", base, p), false),
                None => (base.to_string(), false),
            }
        }
        DataType::Timestamp(prec, tz) => {
            let base = match tz {
                TimezoneInfo::WithTimeZone | TimezoneInfo::Tz => "timestamptz",
                TimezoneInfo::WithoutTimeZone | TimezoneInfo::None => "timestamp",
            };
            match prec {
                Some(p) => (format!("{}({})", base, p), false),
                None => (base.to_string(), false),
            }
        }
        DataType::Interval { .. } => ("interval".to_string(), false),

        // JSON
        DataType::JSON => ("json".to_string(), false),
        DataType::JSONB => ("jsonb".to_string(), false),

        // Binary
        DataType::Bytea => ("bytea".to_string(), false),

        // UUID
        DataType::Uuid => ("uuid".to_string(), false),

        // Array types
        DataType::Array(elem) => {
            let inner = match elem {
                ArrayElemTypeDef::SquareBracket(inner_dt, _) => {
                    let (inner_type, _) = normalize_data_type(inner_dt, domains);
                    inner_type
                }
                ArrayElemTypeDef::AngleBracket(inner_dt) => {
                    let (inner_type, _) = normalize_data_type(inner_dt, domains);
                    inner_type
                }
                ArrayElemTypeDef::Parenthesis(inner_dt) => {
                    let (inner_type, _) = normalize_data_type(inner_dt, domains);
                    inner_type
                }
                ArrayElemTypeDef::None => "unknown".to_string(),
            };
            // Use short forms for common array element types
            let short = match inner.as_str() {
                "integer" => "int",
                "character varying" => "text",
                _ => &inner,
            };
            (format!("{}[]", short), false)
        }

        // Bit types
        DataType::Bit(_) => ("bit".to_string(), false),
        DataType::BitVarying(_) | DataType::VarBit(_) => ("bit varying".to_string(), false),

        // Fallback: use the Display impl and lowercase it
        other => (other.to_string().to_lowercase(), false),
    }
}

pub(crate) fn object_name_to_key(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|part| match part {
            sqlparser::ast::ObjectNamePart::Identifier(ident) => ident.value.to_lowercase(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn ident_to_lower(ident: &sqlparser::ast::Ident) -> String {
    // Preserve case for double-quoted identifiers
    if ident.quote_style.is_some() {
        ident.value.clone()
    } else {
        ident.value.to_lowercase()
    }
}

pub(crate) fn bare_name(key: &str) -> &str {
    key.rsplit_once('.').map_or(key, |(_, name)| name)
}
