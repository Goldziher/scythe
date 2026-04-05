use sqlparser::ast::{
    self, BinaryOperator, Expr, ObjectName, SelectItem, SetExpr, Statement, TableFactor, Value,
};

// ---------------------------------------------------------------------------
// Value matching helpers (ValueWithSpan wraps Value)
// ---------------------------------------------------------------------------

pub(super) fn value_is_placeholder(vws: &ast::ValueWithSpan) -> Option<&str> {
    match &vws.value {
        Value::Placeholder(s) => Some(s.as_str()),
        _ => None,
    }
}

pub(super) fn value_is_number(vws: &ast::ValueWithSpan) -> bool {
    matches!(&vws.value, Value::Number(_, _))
}

pub(super) fn value_is_string(vws: &ast::ValueWithSpan) -> bool {
    matches!(
        &vws.value,
        Value::SingleQuotedString(_) | Value::DoubleQuotedString(_)
    )
}

pub(super) fn value_is_boolean(vws: &ast::ValueWithSpan) -> bool {
    matches!(&vws.value, Value::Boolean(_))
}

pub(super) fn value_is_null(vws: &ast::ValueWithSpan) -> bool {
    matches!(&vws.value, Value::Null)
}

// ---------------------------------------------------------------------------
// Free-standing helpers
// ---------------------------------------------------------------------------

pub(super) fn object_name_to_string(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|part| match part {
            ast::ObjectNamePart::Identifier(ident) => ident.value.clone(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

pub(super) fn table_factor_name(tf: &TableFactor) -> String {
    match tf {
        TableFactor::Table { name, .. } => object_name_to_string(name).to_lowercase(),
        _ => String::new(),
    }
}

pub(super) fn assignment_target_name(target: &ast::AssignmentTarget) -> String {
    match target {
        ast::AssignmentTarget::ColumnName(name) => object_name_to_string(name).to_lowercase(),
        ast::AssignmentTarget::Tuple(names) => {
            // Use the first name in the tuple
            names
                .first()
                .map(|n| object_name_to_string(n).to_lowercase())
                .unwrap_or_default()
        }
    }
}

pub(super) fn expr_to_name(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(ident) => {
            if ident.quote_style.is_some() {
                ident.value.clone()
            } else {
                ident.value.to_lowercase()
            }
        }
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .map(|i| i.value.to_lowercase())
            .unwrap_or_else(|| "unknown".to_string()),
        Expr::Function(func) => object_name_to_string(&func.name).to_lowercase(),
        Expr::Cast { expr: inner, .. } => expr_to_name(inner),
        Expr::Nested(inner) => expr_to_name(inner),
        Expr::UnaryOp { expr: inner, .. } => expr_to_name(inner),
        Expr::Case { .. } => "case".to_string(),
        Expr::Subquery(_) => "subquery".to_string(),
        Expr::Value(vws) => {
            if let Some(p) = value_is_placeholder(vws)
                && let Some(pos) = parse_placeholder(p)
            {
                return format!("p{}", pos);
            }
            "unknown".to_string()
        }
        Expr::BinaryOp { left, .. } => expr_to_name(left),
        Expr::CompoundFieldAccess { access_chain, .. } => {
            // Return the last field name, e.g. (home_address).street -> "street"
            if let Some(last) = access_chain.last()
                && let ast::AccessExpr::Dot(inner) = last
            {
                return expr_to_name(inner);
            }
            "unknown".to_string()
        }
        _ => "unknown".to_string(),
    }
}

/// Detect if a statement is a SELECT * from a single table (for model struct reuse)
pub(super) fn detect_select_star_source(stmt: &Statement) -> Option<String> {
    if let Statement::Query(query) = stmt
        && let SetExpr::Select(select) = query.body.as_ref()
    {
        // Check: single wildcard projection, single FROM table, no joins
        let is_star =
            select.projection.len() == 1 && matches!(select.projection[0], SelectItem::Wildcard(_));
        if is_star
            && select.from.len() == 1
            && select.from[0].joins.is_empty()
            && let TableFactor::Table { name, .. } = &select.from[0].relation
        {
            let table_name = object_name_to_string(name).to_lowercase();
            return Some(table_name);
        }
    }
    None
}

pub(super) fn parse_placeholder(s: &str) -> Option<i64> {
    s.strip_prefix('$')?.parse::<i64>().ok()
}

/// Derive a param name from a comparison context.
/// For aggregate functions in HAVING (e.g., COUNT(*) > $1), adds a semantic prefix.
pub(super) fn derive_param_name_from_comparison(
    col_name: &str,
    col_side: &Expr,
    _param_side: &Expr,
    op: Option<&BinaryOperator>,
) -> String {
    // If the column side is an aggregate function, use min_/max_ prefix
    if let Expr::Function(_) = col_side
        && let Some(op) = op
    {
        match op {
            // col > $1 means param is a minimum for col
            BinaryOperator::Gt | BinaryOperator::GtEq => {
                return format!("min_{}", col_name);
            }
            // col < $1 means param is a maximum for col
            BinaryOperator::Lt | BinaryOperator::LtEq => {
                return format!("max_{}", col_name);
            }
            _ => {}
        }
    }
    col_name.to_string()
}

/// Check if a CASE WHEN condition guards the result from being null.
/// e.g., `WHEN bio IS NOT NULL THEN bio` - the IS NOT NULL condition guarantees bio is non-null.
pub(super) fn is_not_null_guard(condition: &Expr, result: &Expr) -> bool {
    match condition {
        Expr::IsNotNull(inner) => expr_to_name(inner) == expr_to_name(result),
        _ => false,
    }
}

pub(super) fn is_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Value(vws) => {
            matches!(
                &vws.value,
                Value::Number(_, _)
                    | Value::SingleQuotedString(_)
                    | Value::DoubleQuotedString(_)
                    | Value::Boolean(_)
            )
        }
        _ => false,
    }
}

/// Simple pluralization: add 's' to a name
pub(super) fn pluralize(name: &str) -> String {
    if name.ends_with('s') || name.ends_with('x') || name.ends_with("sh") || name.ends_with("ch") {
        format!("{}es", name)
    } else if name.ends_with('y')
        && !name.ends_with("ey")
        && !name.ends_with("ay")
        && !name.ends_with("oy")
    {
        format!("{}ies", &name[..name.len() - 1])
    } else {
        format!("{}s", name)
    }
}

pub(super) fn is_integer_type(t: &str) -> bool {
    matches!(t, "int16" | "int32" | "int64")
}

pub(super) fn is_comparable_types(a: &str, b: &str) -> bool {
    // Numeric types are comparable with each other
    let numeric = ["int16", "int32", "int64", "float32", "float64", "decimal"];
    if numeric.contains(&a) && numeric.contains(&b) {
        return true;
    }
    // String types
    if a == "string" && b == "string" {
        return true;
    }
    // Temporal types
    let temporal = ["date", "datetime", "datetime_tz", "time", "time_tz"];
    if temporal.contains(&a) && temporal.contains(&b) {
        return true;
    }
    // Enums are comparable with strings (PG implicit cast) and with themselves
    if (a.starts_with("enum::") && b == "string") || (b.starts_with("enum::") && a == "string") {
        return true;
    }
    if a.starts_with("enum::") && b.starts_with("enum::") {
        return true;
    }
    false
}

pub(super) fn neutral_to_sql_label(neutral: &str) -> &str {
    match neutral {
        "int16" => "smallint",
        "int32" => "integer",
        "int64" => "bigint",
        "float32" => "real",
        "float64" => "double precision",
        "decimal" => "numeric",
        "string" => "text",
        "bool" => "boolean",
        "bytes" => "bytea",
        "date" => "date",
        "time" => "time",
        "time_tz" => "timetz",
        "datetime" => "timestamp",
        "datetime_tz" => "timestamptz",
        "interval" => "interval",
        "json" => "json",
        "uuid" => "uuid",
        _ => neutral,
    }
}

/// Widen two numeric types to the wider one for UNION type widening
pub(super) fn widen_type(a: &str, b: &str) -> String {
    if a == b {
        return a.to_string();
    }
    // Integer widening
    let int_rank = |t: &str| -> Option<u8> {
        match t {
            "int16" => Some(0),
            "int32" => Some(1),
            "int64" => Some(2),
            _ => None,
        }
    };
    if let (Some(ra), Some(rb)) = (int_rank(a), int_rank(b)) {
        return if ra >= rb {
            a.to_string()
        } else {
            b.to_string()
        };
    }
    // Float widening
    let float_rank = |t: &str| -> Option<u8> {
        match t {
            "float32" => Some(0),
            "float64" => Some(1),
            _ => None,
        }
    };
    if let (Some(ra), Some(rb)) = (float_rank(a), float_rank(b)) {
        return if ra >= rb {
            a.to_string()
        } else {
            b.to_string()
        };
    }
    // Int + float -> float64
    if int_rank(a).is_some() && float_rank(b).is_some() {
        return "float64".to_string();
    }
    if float_rank(a).is_some() && int_rank(b).is_some() {
        return "float64".to_string();
    }
    // Default: use left side
    a.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::ast::{Ident, ObjectNamePart, ValueWithSpan};
    use sqlparser::tokenizer::Span;

    // ---- widen_type ----
    #[test]
    fn test_widen_type_same() {
        assert_eq!(widen_type("int32", "int32"), "int32");
        assert_eq!(widen_type("float64", "float64"), "float64");
        assert_eq!(widen_type("string", "string"), "string");
    }

    #[test]
    fn test_widen_type_integer_widening() {
        assert_eq!(widen_type("int16", "int32"), "int32");
        assert_eq!(widen_type("int16", "int64"), "int64");
        assert_eq!(widen_type("int32", "int64"), "int64");
        assert_eq!(widen_type("int64", "int16"), "int64");
        assert_eq!(widen_type("int32", "int16"), "int32");
    }

    #[test]
    fn test_widen_type_float_widening() {
        assert_eq!(widen_type("float32", "float64"), "float64");
        assert_eq!(widen_type("float64", "float32"), "float64");
    }

    #[test]
    fn test_widen_type_int_float_mix() {
        assert_eq!(widen_type("int32", "float64"), "float64");
        assert_eq!(widen_type("int64", "float32"), "float64");
        assert_eq!(widen_type("float32", "int16"), "float64");
        assert_eq!(widen_type("float64", "int64"), "float64");
    }

    #[test]
    fn test_widen_type_default_fallback() {
        // Non-numeric: returns left side
        assert_eq!(widen_type("string", "int32"), "string");
        assert_eq!(widen_type("bool", "string"), "bool");
    }

    // ---- is_comparable_types ----
    #[test]
    fn test_is_comparable_types_numeric() {
        assert!(is_comparable_types("int16", "int32"));
        assert!(is_comparable_types("int32", "int64"));
        assert!(is_comparable_types("int64", "float64"));
        assert!(is_comparable_types("float32", "decimal"));
        assert!(is_comparable_types("decimal", "int16"));
    }

    #[test]
    fn test_is_comparable_types_string() {
        assert!(is_comparable_types("string", "string"));
    }

    #[test]
    fn test_is_comparable_types_temporal() {
        assert!(is_comparable_types("date", "datetime"));
        assert!(is_comparable_types("datetime", "datetime_tz"));
        assert!(is_comparable_types("time", "time_tz"));
    }

    #[test]
    fn test_is_comparable_types_incompatible() {
        assert!(!is_comparable_types("string", "int32"));
        assert!(!is_comparable_types("bool", "int64"));
        assert!(!is_comparable_types("date", "string"));
        assert!(!is_comparable_types("json", "string"));
    }

    // ---- parse_placeholder ----
    #[test]
    fn test_parse_placeholder_valid() {
        assert_eq!(parse_placeholder("$1"), Some(1));
        assert_eq!(parse_placeholder("$99"), Some(99));
        assert_eq!(parse_placeholder("$0"), Some(0));
    }

    #[test]
    fn test_parse_placeholder_invalid() {
        assert_eq!(parse_placeholder("1"), None);
        assert_eq!(parse_placeholder("$abc"), None);
        assert_eq!(parse_placeholder(""), None);
        assert_eq!(parse_placeholder("$$"), None);
    }

    // ---- expr_to_name ----
    #[test]
    fn test_expr_to_name_identifier() {
        let expr = Expr::Identifier(Ident::new("my_column"));
        assert_eq!(expr_to_name(&expr), "my_column");
    }

    #[test]
    fn test_expr_to_name_compound() {
        let expr = Expr::CompoundIdentifier(vec![Ident::new("t"), Ident::new("my_col")]);
        assert_eq!(expr_to_name(&expr), "my_col");
    }

    #[test]
    fn test_expr_to_name_function() {
        use sqlparser::ast::{Function, FunctionArguments};
        let func = Function {
            name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new("count"))]),
            args: FunctionArguments::None,
            filter: None,
            over: None,
            null_treatment: None,
            within_group: Vec::new(),
            parameters: FunctionArguments::None,
            uses_odbc_syntax: false,
        };
        let expr = Expr::Function(func);
        assert_eq!(expr_to_name(&expr), "count");
    }

    #[test]
    fn test_expr_to_name_nested() {
        let inner = Expr::Identifier(Ident::new("x"));
        let expr = Expr::Nested(Box::new(inner));
        assert_eq!(expr_to_name(&expr), "x");
    }

    #[test]
    fn test_expr_to_name_placeholder() {
        let vws = ValueWithSpan {
            value: Value::Placeholder("$3".to_string()),
            span: Span::empty(),
        };
        let expr = Expr::Value(vws);
        assert_eq!(expr_to_name(&expr), "p3");
    }

    #[test]
    fn test_expr_to_name_unknown_fallback() {
        let vws = ValueWithSpan {
            value: Value::Null,
            span: Span::empty(),
        };
        let expr = Expr::Value(vws);
        assert_eq!(expr_to_name(&expr), "unknown");
    }

    // ---- pluralize ----
    #[test]
    fn test_pluralize_regular() {
        assert_eq!(pluralize("user"), "users");
        assert_eq!(pluralize("post"), "posts");
        assert_eq!(pluralize("comment"), "comments");
    }

    #[test]
    fn test_pluralize_s_x_sh_ch() {
        assert_eq!(pluralize("status"), "statuses");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("wish"), "wishes");
        assert_eq!(pluralize("match"), "matches");
    }

    #[test]
    fn test_pluralize_y_ending() {
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("city"), "cities");
        // "ey", "ay", "oy" endings should just add 's'
        assert_eq!(pluralize("key"), "keys");
        assert_eq!(pluralize("day"), "days");
        assert_eq!(pluralize("boy"), "boys");
    }

    // ---- is_integer_type ----
    #[test]
    fn test_is_integer_type() {
        assert!(is_integer_type("int16"));
        assert!(is_integer_type("int32"));
        assert!(is_integer_type("int64"));
        assert!(!is_integer_type("float32"));
        assert!(!is_integer_type("string"));
    }

    // ---- neutral_to_sql_label ----
    #[test]
    fn test_neutral_to_sql_label() {
        assert_eq!(neutral_to_sql_label("int32"), "integer");
        assert_eq!(neutral_to_sql_label("int64"), "bigint");
        assert_eq!(neutral_to_sql_label("string"), "text");
        assert_eq!(neutral_to_sql_label("bool"), "boolean");
        assert_eq!(neutral_to_sql_label("datetime_tz"), "timestamptz");
        assert_eq!(neutral_to_sql_label("uuid"), "uuid");
        // Unknown type returns as-is
        assert_eq!(neutral_to_sql_label("custom_type"), "custom_type");
    }

    // ---- derive_param_name_from_comparison ----
    #[test]
    fn test_derive_param_name_from_comparison_no_function() {
        let col_side = Expr::Identifier(Ident::new("age"));
        let param_side = Expr::Identifier(Ident::new("dummy"));
        // Not a function expression, so just returns col_name
        assert_eq!(
            derive_param_name_from_comparison(
                "age",
                &col_side,
                &param_side,
                Some(&BinaryOperator::Gt)
            ),
            "age"
        );
    }

    #[test]
    fn test_derive_param_name_from_comparison_function_gt() {
        use sqlparser::ast::{Function, FunctionArguments};
        let func = Function {
            name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new("count"))]),
            args: FunctionArguments::None,
            filter: None,
            over: None,
            null_treatment: None,
            within_group: Vec::new(),
            parameters: FunctionArguments::None,
            uses_odbc_syntax: false,
        };
        let col_side = Expr::Function(func);
        let param_side = Expr::Identifier(Ident::new("dummy"));
        assert_eq!(
            derive_param_name_from_comparison(
                "count",
                &col_side,
                &param_side,
                Some(&BinaryOperator::Gt)
            ),
            "min_count"
        );
        assert_eq!(
            derive_param_name_from_comparison(
                "count",
                &col_side,
                &param_side,
                Some(&BinaryOperator::GtEq)
            ),
            "min_count"
        );
    }

    #[test]
    fn test_derive_param_name_from_comparison_function_lt() {
        use sqlparser::ast::{Function, FunctionArguments};
        let func = Function {
            name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new("count"))]),
            args: FunctionArguments::None,
            filter: None,
            over: None,
            null_treatment: None,
            within_group: Vec::new(),
            parameters: FunctionArguments::None,
            uses_odbc_syntax: false,
        };
        let col_side = Expr::Function(func);
        let param_side = Expr::Identifier(Ident::new("dummy"));
        assert_eq!(
            derive_param_name_from_comparison(
                "count",
                &col_side,
                &param_side,
                Some(&BinaryOperator::Lt)
            ),
            "max_count"
        );
        assert_eq!(
            derive_param_name_from_comparison(
                "count",
                &col_side,
                &param_side,
                Some(&BinaryOperator::LtEq)
            ),
            "max_count"
        );
    }

    #[test]
    fn test_derive_param_name_from_comparison_function_eq() {
        use sqlparser::ast::{Function, FunctionArguments};
        let func = Function {
            name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new("count"))]),
            args: FunctionArguments::None,
            filter: None,
            over: None,
            null_treatment: None,
            within_group: Vec::new(),
            parameters: FunctionArguments::None,
            uses_odbc_syntax: false,
        };
        let col_side = Expr::Function(func);
        let param_side = Expr::Identifier(Ident::new("dummy"));
        // Eq does not add prefix even for functions
        assert_eq!(
            derive_param_name_from_comparison(
                "count",
                &col_side,
                &param_side,
                Some(&BinaryOperator::Eq)
            ),
            "count"
        );
    }

    // ---- is_not_null_guard ----
    #[test]
    fn test_is_not_null_guard_matching() {
        let inner = Expr::Identifier(Ident::new("bio"));
        let condition = Expr::IsNotNull(Box::new(inner));
        let result = Expr::Identifier(Ident::new("bio"));
        assert!(is_not_null_guard(&condition, &result));
    }

    #[test]
    fn test_is_not_null_guard_non_matching() {
        let inner = Expr::Identifier(Ident::new("bio"));
        let condition = Expr::IsNotNull(Box::new(inner));
        let result = Expr::Identifier(Ident::new("name"));
        assert!(!is_not_null_guard(&condition, &result));
    }

    #[test]
    fn test_is_not_null_guard_not_is_not_null() {
        let condition = Expr::Identifier(Ident::new("bio"));
        let result = Expr::Identifier(Ident::new("bio"));
        assert!(!is_not_null_guard(&condition, &result));
    }

    // ---- is_literal ----
    #[test]
    fn test_is_literal() {
        let num = Expr::Value(ValueWithSpan {
            value: Value::Number("42".to_string(), false),
            span: Span::empty(),
        });
        assert!(is_literal(&num));

        let s = Expr::Value(ValueWithSpan {
            value: Value::SingleQuotedString("hello".to_string()),
            span: Span::empty(),
        });
        assert!(is_literal(&s));

        let b = Expr::Value(ValueWithSpan {
            value: Value::Boolean(true),
            span: Span::empty(),
        });
        assert!(is_literal(&b));

        let n = Expr::Value(ValueWithSpan {
            value: Value::Null,
            span: Span::empty(),
        });
        assert!(!is_literal(&n));

        let ident = Expr::Identifier(Ident::new("x"));
        assert!(!is_literal(&ident));
    }

    // ---- object_name_to_string ----
    #[test]
    fn test_object_name_to_string_single() {
        let name = ObjectName(vec![ObjectNamePart::Identifier(Ident::new("users"))]);
        assert_eq!(object_name_to_string(&name), "users");
    }

    #[test]
    fn test_object_name_to_string_qualified() {
        let name = ObjectName(vec![
            ObjectNamePart::Identifier(Ident::new("public")),
            ObjectNamePart::Identifier(Ident::new("users")),
        ]);
        assert_eq!(object_name_to_string(&name), "public.users");
    }

    // ---- value helpers ----
    #[test]
    fn test_value_is_number() {
        let vws = ValueWithSpan {
            value: Value::Number("42".to_string(), false),
            span: Span::empty(),
        };
        assert!(value_is_number(&vws));
        let vws2 = ValueWithSpan {
            value: Value::SingleQuotedString("42".to_string()),
            span: Span::empty(),
        };
        assert!(!value_is_number(&vws2));
    }

    #[test]
    fn test_value_is_string() {
        let vws = ValueWithSpan {
            value: Value::SingleQuotedString("hello".to_string()),
            span: Span::empty(),
        };
        assert!(value_is_string(&vws));
    }

    #[test]
    fn test_value_is_boolean() {
        let vws = ValueWithSpan {
            value: Value::Boolean(true),
            span: Span::empty(),
        };
        assert!(value_is_boolean(&vws));
    }

    #[test]
    fn test_value_is_null() {
        let vws = ValueWithSpan {
            value: Value::Null,
            span: Span::empty(),
        };
        assert!(value_is_null(&vws));
    }

    #[test]
    fn test_value_is_placeholder() {
        let vws = ValueWithSpan {
            value: Value::Placeholder("$1".to_string()),
            span: Span::empty(),
        };
        assert_eq!(value_is_placeholder(&vws), Some("$1"));
        let vws2 = ValueWithSpan {
            value: Value::Number("1".to_string(), false),
            span: Span::empty(),
        };
        assert_eq!(value_is_placeholder(&vws2), None);
    }
}
