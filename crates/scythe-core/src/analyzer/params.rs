use sqlparser::ast::{BinaryOperator, Expr};
use sqlparser::tokenizer::Span;

use super::helpers::*;
use super::type_conversion::datatype_to_neutral;
use super::types::*;

impl<'a> Analyzer<'a> {
    /// Resolve a placeholder string to a position number.
    /// For `$N` placeholders, returns the parsed number -- already idempotent,
    /// so `span` is ignored and the memo is never consulted or updated for
    /// this branch.
    /// For `?` (MySQL positional), the same source-text occurrence can reach
    /// this function twice (`collect_params_from_where` and `infer_expr_type`
    /// both visit the same projection expression -- see `analyze_select`), so
    /// `span` identifies the occurrence: a span already in
    /// `self.resolved_placeholders` returns its previously assigned position
    /// instead of auto-incrementing again (#170). `Span::empty()` -- which
    /// only synthetic/test AST nodes carry -- bypasses the memo entirely and
    /// falls back to the old always-increment behavior, since every such
    /// node would otherwise collapse onto one memo entry.
    /// For any other token (e.g. `:bucket` named placeholders) this is an error:
    /// a message is pushed to `self.type_errors` and `None` is returned so the
    /// caller's no-match path skips param registration. The error surfaces through
    /// `analyze()` which checks `type_errors` before returning.
    /// Note: Oracle `:N` numeric placeholders are converted to `?` by
    /// `preprocess_oracle_sql` before the AST is built, so they never reach here.
    pub(super) fn resolve_placeholder_position(&mut self, placeholder: &str, span: Span) -> Option<i64> {
        if let Some(pos) = parse_placeholder(placeholder) {
            Some(pos)
        } else if is_positional_placeholder(placeholder) {
            let memo_key = (span != Span::empty()).then_some(span);
            if let Some(key) = memo_key
                && let Some(&pos) = self.resolved_placeholders.get(&key)
            {
                return Some(pos);
            }
            self.positional_param_counter += 1;
            let pos = self.positional_param_counter;
            if let Some(key) = memo_key {
                self.resolved_placeholders.insert(key, pos);
            }
            Some(pos)
        } else {
            self.type_errors.push(format!(
                "unsupported placeholder \"{placeholder}\": named placeholders are not supported; \
                 use $N (PostgreSQL) or ? (MySQL/SQLite) instead",
            ));
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
                // A LIKE pattern is not always a bare placeholder literal --
                // `'%' || $1 || '%'` (a BinaryOp concat) is the common
                // idiom for a "contains" search, and was previously only
                // matched when `pattern` was exactly `Expr::Value`, so the
                // placeholder inside the concatenation silently registered
                // nothing (#171). `collect_param_from_expr_with_type`
                // already walks `BinaryOp`/`Cast`/`Nested`/... to find the
                // placeholder wherever it sits -- the same helper
                // `infer_expr_type`'s LIKE arm uses, so both traversals of
                // a LIKE pattern agree on what gets registered.
                let name = expr_to_name(col_expr);
                self.collect_param_from_expr_with_type(pattern, "string", &name);
            }
            Expr::InList {
                expr: col_expr, list, ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                let col_name = expr_to_name(col_expr);
                for item in list {
                    if let Expr::Value(vws) = item
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                    {
                        self.register_param(pos, Some(col_name.clone()), Some(col_ti.neutral_type.clone()), false);
                    }
                }
            }
            // The operand was previously never descended into, so a
            // placeholder used inside it (e.g. `WHERE coalesce(x, $1) IS
            // NULL`) registered nothing and silently dropped a parameter
            // from the signature while `$1` stayed in the emitted SQL
            // (#171). `IS NULL`/`IS NOT NULL` themselves are never
            // placeholders -- there is nothing dialect-specific to bind at
            // this node -- so this only needs to keep walking.
            Expr::IsNull(inner) | Expr::IsNotNull(inner) => self.collect_params_from_where(inner, scope),
            Expr::Nested(inner) => self.collect_params_from_where(inner, scope),
            Expr::UnaryOp { expr: inner, .. } => self.collect_params_from_where(inner, scope),
            Expr::AnyOp { left, right, .. } => {
                let left_ti = self.infer_expr_type(left, scope);
                if let Expr::Value(vws) = right.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
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
                let args = self.get_function_args(func);
                for arg in &args {
                    self.collect_params_from_where(arg, scope);
                }
            }
            Expr::Cast {
                expr: inner, data_type, ..
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
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                {
                    let col_ti = self.infer_expr_type(col_side, scope);
                    let col_name = expr_to_name(col_side);
                    let param_name = derive_param_name_from_comparison(&col_name, col_side, param_side, op);
                    self.register_param(pos, Some(param_name), Some(col_ti.neutral_type), false);
                }
            }
            Expr::Cast {
                expr: inner, data_type, ..
            } => {
                if let Expr::Value(vws) = inner.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                {
                    let neutral = datatype_to_neutral(data_type, self.catalog);
                    let col_name = expr_to_name(col_side);
                    let param_name = derive_param_name_from_comparison(&col_name, col_side, param_side, op);
                    self.register_param(pos, Some(param_name), Some(neutral), false);
                }
            }
            _ => {}
        }
    }

    pub(super) fn collect_param_from_expr(&mut self, expr: &Expr, name: &str, type_str: &str) {
        if let Expr::Value(vws) = expr {
            if let Some(p) = value_is_placeholder(vws)
                && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
            {
                self.register_param(pos, Some(name.to_string()), Some(type_str.to_string()), false);
            }
        } else if let Expr::Cast {
            expr: inner, data_type, ..
        } = expr
            && let Expr::Value(vws) = inner.as_ref()
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
        {
            let neutral = datatype_to_neutral(data_type, self.catalog);
            self.register_param(pos, Some(name.to_string()), Some(neutral), false);
        }
    }

    pub(super) fn collect_param_from_expr_with_type(&mut self, expr: &Expr, type_str: &str, name: &str) {
        self.collect_param_from_expr_with_type_nullable(expr, type_str, name, false);
    }

    pub(super) fn collect_param_from_expr_with_type_nullable(
        &mut self,
        expr: &Expr,
        type_str: &str,
        name: &str,
        nullable: bool,
    ) {
        match expr {
            Expr::Value(vws) => {
                if let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                {
                    self.register_param(pos, Some(name.to_string()), Some(type_str.to_string()), nullable);
                }
            }
            Expr::Cast {
                expr: inner, data_type, ..
            } => {
                if let Expr::Value(vws) = inner.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                {
                    let neutral = datatype_to_neutral(data_type, self.catalog);
                    self.register_param(pos, Some(name.to_string()), Some(neutral), nullable);
                } else {
                    self.collect_param_from_expr_with_type_nullable(inner, type_str, name, nullable);
                }
            }
            Expr::Nested(inner) => {
                self.collect_param_from_expr_with_type_nullable(inner, type_str, name, nullable);
            }
            Expr::UnaryOp { expr: inner, .. } => {
                self.collect_param_from_expr_with_type_nullable(inner, type_str, name, nullable);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.collect_param_from_expr_with_type_nullable(left, type_str, name, nullable);
                self.collect_param_from_expr_with_type_nullable(right, type_str, name, nullable);
            }
            Expr::Function(func) => {
                for arg in self.get_function_args(func) {
                    self.collect_param_from_expr_with_type_nullable(&arg, type_str, name, nullable);
                }
            }
            Expr::Case {
                operand,
                conditions,
                else_result,
                ..
            } => {
                if let Some(op) = operand {
                    self.collect_param_from_expr_with_type_nullable(op, type_str, name, nullable);
                }
                for case_when in conditions {
                    self.collect_param_from_expr_with_type_nullable(&case_when.condition, type_str, name, nullable);
                    self.collect_param_from_expr_with_type_nullable(&case_when.result, type_str, name, nullable);
                }
                if let Some(else_expr) = else_result {
                    self.collect_param_from_expr_with_type_nullable(else_expr, type_str, name, nullable);
                }
            }
            _ => {}
        }
    }

    pub(super) fn collect_param_type_from_cast(&mut self, expr: &Expr, neutral_type: &str) {
        if let Expr::Value(vws) = expr
            && let Some(p) = value_is_placeholder(vws)
            && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
        {
            let name = match neutral_type {
                "interval" => Some("duration".to_string()),
                _ => None,
            };
            self.register_param(pos, name, Some(neutral_type.to_string()), false);
        }
    }

    pub(super) fn collect_param_from_any(&mut self, expr: &Expr, left_ti: &TypeInfo, left_name: &str) {
        match expr {
            Expr::Value(vws) => {
                if let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                {
                    let array_type = format!("array<{}>", left_ti.neutral_type);
                    let name = pluralize(left_name);
                    self.register_param(pos, Some(name), Some(array_type), false);
                }
            }
            Expr::Cast {
                expr: inner, data_type, ..
            } => {
                if let Expr::Value(vws) = inner.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                {
                    let neutral = datatype_to_neutral(data_type, self.catalog);
                    self.register_param(pos, None, Some(neutral), false);
                }
            }
            Expr::Nested(inner) => self.collect_param_from_any(inner, left_ti, left_name),
            Expr::Array(arr) => {
                for (i, elem) in arr.elem.iter().enumerate() {
                    if let Expr::Value(vws) = elem
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                    {
                        let name = format!("{}{}", left_name, i + 1);
                        self.register_param(pos, Some(name), Some(left_ti.neutral_type.clone()), false);
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
    use sqlparser::tokenizer::{Location, Span};

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
            pending_nested: Vec::new(),
            next_nested_id: 0,
            resolved_placeholders: AHashMap::new(),
        }
    }

    fn placeholder_expr(pos: &str) -> Expr {
        placeholder_expr_at(pos, Span::empty())
    }

    fn placeholder_expr_at(pos: &str, span: Span) -> Expr {
        Expr::Value(ValueWithSpan {
            value: Value::Placeholder(pos.to_string()),
            span,
        })
    }

    fn span_at(offset: u64) -> Span {
        // Distinct, non-empty spans standing in for two different
        // occurrences of the same placeholder token in real source text --
        // `Span::empty()` deliberately bypasses the memo (see
        // `resolve_placeholder_position`), so exercising the memo itself
        // needs spans sqlparser would actually hand back.
        Span::new(Location::new(1, offset), Location::new(1, offset + 1))
    }

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
        analyzer.register_param(1, None, None, false);
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].name, None);
        assert_eq!(analyzer.params[0].neutral_type, None);

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
        analyzer.register_param(1, Some("new_name".to_string()), Some("string".to_string()), true);
        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].name, Some("id".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("int32".to_string()));
    }

    #[test]
    fn test_register_multiple_params() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        analyzer.register_param(1, Some("name".to_string()), Some("string".to_string()), false);
        analyzer.register_param(2, Some("age".to_string()), Some("int32".to_string()), false);
        assert_eq!(analyzer.params.len(), 2);
        assert_eq!(analyzer.params[0].position, 1);
        assert_eq!(analyzer.params[1].position, 2);
    }

    #[test]
    fn test_try_bind_param_from_comparison_basic() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE users (id INTEGER NOT NULL, name TEXT NOT NULL);"]).unwrap();
        let mut analyzer = Analyzer {
            catalog: &catalog,
            params: Vec::new(),
            ctes: AHashMap::new(),
            type_errors: Vec::new(),
            positional_param_counter: 0,
            pending_nested: Vec::new(),
            next_nested_id: 0,
            resolved_placeholders: AHashMap::new(),
        };
        let scope = Scope {
            sources: vec![ScopeSource {
                alias: "users".to_string(),
                table_name: "users".to_string(),
                columns: vec![
                    ScopeColumn::new("id", "int32", false),
                    ScopeColumn::new("name", "string", false),
                ],
                nullable_from_join: false,
            }],
        };

        let param_side = placeholder_expr("$1");
        let col_side = Expr::Identifier(Ident::new("id"));
        analyzer.try_bind_param_from_comparison(&param_side, &col_side, &scope, Some(&BinaryOperator::Eq));

        assert_eq!(analyzer.params.len(), 1);
        assert_eq!(analyzer.params[0].position, 1);
        assert_eq!(analyzer.params[0].name, Some("id".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("int32".to_string()));
    }

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
        assert_eq!(analyzer.params[0].neutral_type, Some("interval".to_string()));
    }

    #[test]
    fn test_collect_param_type_from_cast_non_placeholder_ignored() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let expr = Expr::Identifier(Ident::new("x"));
        analyzer.collect_param_type_from_cast(&expr, "int32");
        assert_eq!(analyzer.params.len(), 0, "non-placeholder should not register a param");
    }

    #[test]
    fn test_resolve_placeholder_position_dollar_n() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        assert_eq!(analyzer.resolve_placeholder_position("$1", Span::empty()), Some(1));
        assert_eq!(analyzer.resolve_placeholder_position("$99", Span::empty()), Some(99));
        assert!(analyzer.type_errors.is_empty());
    }

    #[test]
    fn test_resolve_placeholder_position_question_mark() {
        // Two calls with `Span::empty()` -- the identity synthetic/test
        // nodes carry when they don't come from `Parser::parse_sql` --
        // deliberately bypass the memo and keep auto-incrementing, so this
        // still models two genuinely different occurrences.
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        assert_eq!(analyzer.resolve_placeholder_position("?", Span::empty()), Some(1));
        assert_eq!(analyzer.resolve_placeholder_position("?", Span::empty()), Some(2));
        assert!(analyzer.type_errors.is_empty());
    }

    #[test]
    fn test_resolve_placeholder_position_question_mark_same_span_is_idempotent() {
        // The core of #170: the same source-text `?` occurrence, identified
        // by its span, must resolve to the same position on a second visit
        // instead of minting a new one.
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let span = span_at(10);
        assert_eq!(analyzer.resolve_placeholder_position("?", span), Some(1));
        assert_eq!(
            analyzer.resolve_placeholder_position("?", span),
            Some(1),
            "revisiting the same occurrence must not advance the counter"
        );
    }

    #[test]
    fn test_resolve_placeholder_position_question_mark_distinct_spans_differ() {
        // Two real, distinct occurrences (different spans) must still get
        // different positions -- the memo must not collapse unrelated `?`s.
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        assert_eq!(analyzer.resolve_placeholder_position("?", span_at(1)), Some(1));
        assert_eq!(analyzer.resolve_placeholder_position("?", span_at(2)), Some(2));
        assert_eq!(
            analyzer.resolve_placeholder_position("?", span_at(1)),
            Some(1),
            "revisiting the first occurrence must still return its original position"
        );
    }

    #[test]
    fn test_resolve_placeholder_position_named_param_errors() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let result = analyzer.resolve_placeholder_position(":bucket", Span::empty());
        assert_eq!(result, None, "named placeholder must return None");
        assert_eq!(analyzer.type_errors.len(), 1, "one error must be recorded");
        assert!(
            analyzer.type_errors[0].contains(":bucket"),
            "error must name the offending token"
        );
        assert!(
            analyzer.type_errors[0].contains("not supported"),
            "error must explain the problem"
        );
    }

    #[test]
    fn test_collect_params_from_where_between_placeholder_visited_twice_counts_once() {
        // `analyze_select` calls `collect_params_from_where` and then
        // `infer_expr_type` over the same projection expression, so a
        // `Between` node's placeholder is reached by both. Simulating that
        // here by calling `collect_params_from_where` and then
        // `infer_expr_type` on the same `Expr::Between` node reproduces
        // #170: the second visit must resolve to the same position instead
        // of registering a second parameter.
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = Scope { sources: Vec::new() };
        let span = span_at(20);
        let expr = Expr::Between {
            expr: Box::new(Expr::Identifier(Ident::new("age"))),
            negated: false,
            low: Box::new(placeholder_expr_at("?", span)),
            high: Box::new(placeholder_expr_at("$2", Span::empty())),
        };
        analyzer.collect_params_from_where(&expr, &scope);
        let _ = analyzer.infer_expr_type(&expr, &scope);
        assert_eq!(
            analyzer.params.len(),
            2,
            "a BETWEEN with one `?` and one `$2` must report exactly two parameters, not more"
        );
        assert!(
            analyzer.params.iter().any(|p| p.position == 1),
            "the `?` must have resolved to position 1"
        );
        assert!(
            analyzer.params.iter().any(|p| p.position == 2),
            "the `$2` must still resolve to its explicit position"
        );
    }

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

    // Regression tests for #171: a parameter dropped from the generated
    // function signature while the placeholder stays in the emitted SQL.

    #[test]
    fn test_collect_params_from_where_is_null_descends_into_operand() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = Scope { sources: Vec::new() };
        // `$1 IS NULL` is a degenerate stand-in for the realistic shape
        // (`coalesce(x, $1) IS NULL`) that exercises the same operand
        // descent without needing to build a full function-call AST node.
        let expr = Expr::IsNull(Box::new(placeholder_expr("$1")));
        analyzer.collect_params_from_where(&expr, &scope);
        assert_eq!(
            analyzer.params.len(),
            1,
            "a placeholder inside an IS NULL operand must not be dropped"
        );
        assert_eq!(analyzer.params[0].position, 1);
    }

    #[test]
    fn test_collect_params_from_where_is_not_null_descends_into_operand() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = Scope { sources: Vec::new() };
        let expr = Expr::IsNotNull(Box::new(placeholder_expr("$2")));
        analyzer.collect_params_from_where(&expr, &scope);
        assert_eq!(
            analyzer.params.len(),
            1,
            "a placeholder inside an IS NOT NULL operand must not be dropped"
        );
        assert_eq!(analyzer.params[0].position, 2);
    }

    #[test]
    fn test_collect_params_from_where_like_literal_pattern_still_registers() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = Scope { sources: Vec::new() };
        let expr = Expr::Like {
            negated: false,
            any: false,
            expr: Box::new(Expr::Identifier(Ident::new("name"))),
            pattern: Box::new(placeholder_expr("$1")),
            escape_char: None,
        };
        analyzer.collect_params_from_where(&expr, &scope);
        assert_eq!(
            analyzer.params.len(),
            1,
            "the pre-existing literal-pattern case must keep working"
        );
        assert_eq!(analyzer.params[0].position, 1);
        assert_eq!(analyzer.params[0].name, Some("name".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("string".to_string()));
    }

    #[test]
    fn test_collect_params_from_where_like_non_literal_pattern_registers() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = Scope { sources: Vec::new() };
        // `'%' || $1` is not an `Expr::Value`, so the old literal-only
        // check silently registered nothing for it (#171).
        let expr = Expr::Like {
            negated: false,
            any: false,
            expr: Box::new(Expr::Identifier(Ident::new("name"))),
            pattern: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Value(ValueWithSpan {
                    value: Value::SingleQuotedString("%".to_string()),
                    span: Span::empty(),
                })),
                op: BinaryOperator::StringConcat,
                right: Box::new(placeholder_expr("$1")),
            }),
            escape_char: None,
        };
        analyzer.collect_params_from_where(&expr, &scope);
        assert_eq!(
            analyzer.params.len(),
            1,
            "a placeholder inside a non-literal LIKE pattern must not be dropped"
        );
        assert_eq!(analyzer.params[0].position, 1);
        assert_eq!(analyzer.params[0].name, Some("name".to_string()));
        assert_eq!(analyzer.params[0].neutral_type, Some("string".to_string()));
    }
}
