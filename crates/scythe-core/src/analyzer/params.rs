use sqlparser::ast::{BinaryOperator, Expr};

use super::helpers::*;
use super::type_conversion::datatype_to_neutral;
use super::types::*;

impl<'a> Analyzer<'a> {
    /// Resolve a placeholder string to a position number.
    /// For `$N` placeholders, returns the parsed number.
    /// For `?` (MySQL positional), auto-increments and returns the next position.
    pub(super) fn resolve_placeholder_position(&mut self, placeholder: &str) -> Option<i64> {
        if let Some(pos) = parse_placeholder(placeholder) {
            Some(pos)
        } else if is_positional_placeholder(placeholder) {
            self.positional_param_counter += 1;
            Some(self.positional_param_counter)
        } else {
            None
        }
    }

    pub(super) fn register_param(
        &mut self,
        position: i64,
        name: Option<String>,
        neutral_type: Option<String>,
        nullable: bool,
    ) {
        if let Some(existing) = self.params.iter_mut().find(|p| p.position == position) {
            if existing.name.is_none() && name.is_some() {
                existing.name = name;
            }
            if existing.neutral_type.is_none() && neutral_type.is_some() {
                existing.neutral_type = neutral_type;
            }
        } else {
            self.params.push(ParamInfo {
                position,
                name,
                neutral_type,
                nullable,
            });
        }
    }

    pub(super) fn collect_params_from_where(&mut self, expr: &Expr, scope: &Scope) {
        match expr {
            Expr::BinaryOp { left, op, right } => match op {
                BinaryOperator::Eq
                | BinaryOperator::NotEq
                | BinaryOperator::Lt
                | BinaryOperator::LtEq
                | BinaryOperator::Gt
                | BinaryOperator::GtEq => {
                    self.try_bind_param_from_comparison(left, right, scope, Some(op));
                    self.try_bind_param_from_comparison(right, left, scope, Some(op));
                    // Type mismatch detection: store for validation
                    let left_ti = self.infer_expr_type(left, scope);
                    let right_ti = self.infer_expr_type(right, scope);
                    if left_ti.neutral_type != "unknown"
                        && right_ti.neutral_type != "unknown"
                        && !left_ti.neutral_type.starts_with("__")
                        && !right_ti.neutral_type.starts_with("__")
                        && left_ti.neutral_type != right_ti.neutral_type
                        && !is_comparable_types(&left_ti.neutral_type, &right_ti.neutral_type)
                    {
                        let left_sql = neutral_to_sql_label(&left_ti.neutral_type);
                        let right_sql = neutral_to_sql_label(&right_ti.neutral_type);
                        let op_str = format!("{}", op);
                        self.type_errors.push(format!(
                            "operator does not exist: {} {} {}",
                            left_sql, op_str, right_sql
                        ));
                    }
                }
                BinaryOperator::And | BinaryOperator::Or => {
                    self.collect_params_from_where(left, scope);
                    self.collect_params_from_where(right, scope);
                }
                _ => {
                    self.collect_params_from_where(left, scope);
                    self.collect_params_from_where(right, scope);
                }
            },
            Expr::Between {
                expr: col_expr,
                low,
                high,
                ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                self.collect_param_from_expr_with_type(low, &col_ti.neutral_type, "start");
                self.collect_param_from_expr_with_type(high, &col_ti.neutral_type, "end");
            }
            Expr::Like {
                expr: col_expr,
                pattern,
                ..
            }
            | Expr::ILike {
                expr: col_expr,
                pattern,
                ..
            } => {
                if let Expr::Value(vws) = pattern.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p)
                {
                    let name = expr_to_name(col_expr);
                    self.register_param(pos, Some(name), Some("string".to_string()), false);
                }
            }
            Expr::InList {
                expr: col_expr,
                list,
                ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                let col_name = expr_to_name(col_expr);
                for item in list {
                    if let Expr::Value(vws) = item
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p)
                    {
                        self.register_param(
                            pos,
                            Some(col_name.clone()),
                            Some(col_ti.neutral_type.clone()),
                            false,
                        );
                    }
                }
            }
            Expr::IsNull(_) | Expr::IsNotNull(_) => {}
            Expr::Nested(inner) => self.collect_params_from_where(inner, scope),
            Expr::UnaryOp { expr: inner, .. } => self.collect_params_from_where(inner, scope),
            Expr::AnyOp { left, right, .. } => {
                let left_ti = self.infer_expr_type(left, scope);
                if let Expr::Value(vws) = right.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p)
                {
                    let array_type = format!("array<{}>", left_ti.neutral_type);
                    let name = pluralize(&expr_to_name(left));
                    self.register_param(pos, Some(name), Some(array_type), false);
                }
                self.collect_param_from_any(right, &left_ti, &expr_to_name(left));
            }
            Expr::InSubquery { subquery, .. } => {
                let _ = self.analyze_query(subquery);
            }
            Expr::Exists { subquery, .. } => {
                let _ = self.analyze_query(subquery);
            }
            Expr::Subquery(subquery) => {
                let _ = self.analyze_query(subquery);
            }
            Expr::Case {
                conditions,
                else_result,
                ..
            } => {
                for case_when in conditions {
                    self.collect_params_from_where(&case_when.condition, scope);
                    let _ = self.infer_expr_type(&case_when.result, scope);
                }
                if let Some(else_expr) = else_result {
                    let _ = self.infer_expr_type(else_expr, scope);
                }
            }
            Expr::Function(func) => {
                let _ = self.infer_function_type(func, scope);
                // Also recurse into function args to collect params
                // (e.g., SUM(CASE WHEN col >= $1 THEN ...))
                let args = self.get_function_args(func);
                for arg in &args {
                    self.collect_params_from_where(arg, scope);
                }
            }
            Expr::Cast {
                expr: inner,
                data_type,
                ..
            } => {
                let neutral = datatype_to_neutral(data_type, self.catalog);
                self.collect_param_type_from_cast(inner, &neutral);
                self.collect_params_from_where(inner, scope);
            }
            _ => {
                let _ = self.infer_expr_type(expr, scope);
            }
        }
    }

    pub(super) fn try_bind_param_from_comparison(
        &mut self,
        param_side: &Expr,
        col_side: &Expr,
        scope: &Scope,
        op: Option<&BinaryOperator>,
    ) {
        match param_side {
            Expr::Value(vws) => {
                if let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p)
                {
                    let col_ti = self.infer_expr_type(col_side, scope);
                    let col_name = expr_to_name(col_side);
                    // Add prefix for aggregate comparisons in HAVING
                    let param_name =
                        derive_param_name_from_comparison(&col_name, col_side, param_side, op);
                    self.register_param(pos, Some(param_name), Some(col_ti.neutral_type), false);
                }
            }
            Expr::Cast {
                expr: inner,
                data_type,
                ..
            } => {
                if let Expr::Value(vws) = inner.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p)
                {
                    let neutral = datatype_to_neutral(data_type, self.catalog);
                    let col_name = expr_to_name(col_side);
                    let param_name =
                        derive_param_name_from_comparison(&col_name, col_side, param_side, op);
                    self.register_param(pos, Some(param_name), Some(neutral), false);
                }
            }
            _ => {}
        }
    }

    pub(super) fn collect_param_from_expr(&mut self, expr: &Expr, name: &str, type_str: &str) {
        if let Expr::Value(vws) = expr {
            if let Some(p) = value_is_placeholder(vws)
                && let Some(pos) = self.resolve_placeholder_position(p)
            {
                self.register_param(
                    pos,
                    Some(name.to_string()),
                    Some(type_str.to_string()),
                    false,
                );
            }
        } else if let Expr::Cast {
            expr: inner,
            data_type,
            ..
        } = expr
            && let Expr::Value(vws) = inner.as_ref()
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = self.resolve_placeholder_position(p)
        {
            let neutral = datatype_to_neutral(data_type, self.catalog);
            self.register_param(pos, Some(name.to_string()), Some(neutral), false);
        }
    }

    pub(super) fn collect_param_from_expr_with_type(
        &mut self,
        expr: &Expr,
        type_str: &str,
        name: &str,
    ) {
        self.collect_param_from_expr_with_type_nullable(expr, type_str, name, false);
    }

    pub(super) fn collect_param_from_expr_with_type_nullable(
        &mut self,
        expr: &Expr,
        type_str: &str,
        name: &str,
        nullable: bool,
    ) {
        if let Expr::Value(vws) = expr {
            if let Some(p) = value_is_placeholder(vws)
                && let Some(pos) = self.resolve_placeholder_position(p)
            {
                self.register_param(
                    pos,
                    Some(name.to_string()),
                    Some(type_str.to_string()),
                    nullable,
                );
            }
        } else if let Expr::Cast {
            expr: inner,
            data_type,
            ..
        } = expr
            && let Expr::Value(vws) = inner.as_ref()
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = self.resolve_placeholder_position(p)
        {
            let neutral = datatype_to_neutral(data_type, self.catalog);
            self.register_param(pos, Some(name.to_string()), Some(neutral), nullable);
        }
    }

    pub(super) fn collect_param_type_from_cast(&mut self, expr: &Expr, neutral_type: &str) {
        if let Expr::Value(vws) = expr
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = self.resolve_placeholder_position(p)
        {
            // Give semantic names for certain types
            let name = match neutral_type {
                "interval" => Some("duration".to_string()),
                _ => None,
            };
            self.register_param(pos, name, Some(neutral_type.to_string()), false);
        }
    }

    pub(super) fn collect_param_from_any(
        &mut self,
        expr: &Expr,
        left_ti: &TypeInfo,
        left_name: &str,
    ) {
        match expr {
            Expr::Value(vws) => {
                if let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p)
                {
                    let array_type = format!("array<{}>", left_ti.neutral_type);
                    let name = pluralize(left_name);
                    self.register_param(pos, Some(name), Some(array_type), false);
                }
            }
            Expr::Cast {
                expr: inner,
                data_type,
                ..
            } => {
                if let Expr::Value(vws) = inner.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p)
                {
                    let neutral = datatype_to_neutral(data_type, self.catalog);
                    self.register_param(pos, None, Some(neutral), false);
                }
            }
            Expr::Nested(inner) => self.collect_param_from_any(inner, left_ti, left_name),
            Expr::Array(arr) => {
                // Handle ARRAY[$1, $2, $3] - extract params from array elements
                for (i, elem) in arr.elem.iter().enumerate() {
                    if let Expr::Value(vws) = elem
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p)
                    {
                        let name = format!("{}{}", left_name, i + 1);
                        self.register_param(
                            pos,
                            Some(name),
                            Some(left_ti.neutral_type.clone()),
                            false,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use ahash::AHashMap;
    use sqlparser::ast::{Ident, Value, ValueWithSpan};
    use sqlparser::tokenizer::Span;

    fn empty_catalog() -> Catalog {
        Catalog::from_ddl(&[]).unwrap()
    }

    fn make_analyzer(catalog: &Catalog) -> Analyzer<'_> {
        Analyzer {
            catalog,
            params: Vec::new(),
            ctes: AHashMap::new(),
            type_errors: Vec::new(),
            positional_param_counter: 0,
        }
    }

    fn placeholder_expr(pos: &str) -> Expr {
        Expr::Value(ValueWithSpan {
            value: Value::Placeholder(pos.to_string()),
            span: Span::empty(),
        })
    }

    // ---- register_param ----
    #[test]
    fn test_register_param_new() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        analyzer.register_param(1, Some("id".to_string()), Some("int32".to_string()), false);
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].position, 1);
        assert_eq!(analyzer.params[0].name, Some("id".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("int32".to_string()));
        assert!(!analyzer.params[0].nullable);
    }

    #[test]
    fn test_register_param_dedup_fills_missing() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        // First registration: no name or type
        analyzer.register_param(1, None, None, false);
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].name, None);
        assert_eq!(analyzer.params[0].neutral_type, None);

        // Second registration with the same position: fills in name and type
        analyzer.register_param(1, Some("id".to_string()), Some("int32".to_string()), false);
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].name, Some("id".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("int32".to_string()));
    }

    #[test]
    fn test_register_param_does_not_overwrite_existing() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        analyzer.register_param(1, Some("id".to_string()), Some("int32".to_string()), false);
        // Re-register with different name/type should NOT overwrite
        analyzer.register_param(
            1,
            Some("new_name".to_string()),
            Some("string".to_string()),
            true,
        );
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].name, Some("id".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("int32".to_string()));
    }

    #[test]
    fn test_register_multiple_params() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        analyzer.register_param(
            1,
            Some("name".to_string()),
            Some("string".to_string()),
            false,
        );
        analyzer.register_param(2, Some("age".to_string()), Some("int32".to_string()), false);
        assert_eq!(analyzer.params.len(), 2);
        assert_eq!(analyzer.params[0].position, 1);
        assert_eq!(analyzer.params[1].position, 2);
    }

    // ---- try_bind_param_from_comparison ----
    #[test]
    fn test_try_bind_param_from_comparison_basic() {
        let catalog =
            Catalog::from_ddl(&["CREATE TABLE users (id INTEGER NOT NULL, name TEXT NOT NULL);"])
                .unwrap();
        let mut analyzer = Analyzer {
            catalog: &catalog,
            params: Vec::new(),
            ctes: AHashMap::new(),
            type_errors: Vec::new(),
            positional_param_counter: 0,
        };
        let scope = Scope {
            sources: vec![ScopeSource {
                alias: "users".to_string(),
                table_name: "users".to_string(),
                columns: vec![
                    ScopeColumn {
                        name: "id".to_string(),
                        neutral_type: "int32".to_string(),
                        base_nullable: false,
                    },
                    ScopeColumn {
                        name: "name".to_string(),
                        neutral_type: "string".to_string(),
                        base_nullable: false,
                    },
                ],
                nullable_from_join: false,
            }],
        };

        let param_side = placeholder_expr("$1");
        let col_side = Expr::Identifier(Ident::new("id"));
        analyzer.try_bind_param_from_comparison(
            &param_side,
            &col_side,
            &scope,
            Some(&BinaryOperator::Eq),
        );

        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].position, 1);
        assert_eq!(analyzer.params[0].name, Some("id".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("int32".to_string()));
    }

    // ---- collect_param_type_from_cast ----
    #[test]
    fn test_collect_param_type_from_cast_placeholder() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let expr = placeholder_expr("$1");
        analyzer.collect_param_type_from_cast(&expr, "int32");
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].neutral_type, Some("int32".to_string()));
    }

    #[test]
    fn test_collect_param_type_from_cast_interval_name() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let expr = placeholder_expr("$1");
        analyzer.collect_param_type_from_cast(&expr, "interval");
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].name, Some("duration".to_string()));
        assert_eq!(
            analyzer.params[0].neutral_type,
            Some("interval".to_string())
        );
    }

    #[test]
    fn test_collect_param_type_from_cast_non_placeholder_ignored() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let expr = Expr::Identifier(Ident::new("x"));
        analyzer.collect_param_type_from_cast(&expr, "int32");
        assert_eq!(
            analyzer.params.len(),
            0,
            "non-placeholder should not register a param"
        );
    }

    // ---- collect_param_from_expr ----
    #[test]
    fn test_collect_param_from_expr_placeholder() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let expr = placeholder_expr("$2");
        analyzer.collect_param_from_expr(&expr, "email", "string");
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].position, 2);
        assert_eq!(analyzer.params[0].name, Some("email".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("string".to_string()));
    }
}
