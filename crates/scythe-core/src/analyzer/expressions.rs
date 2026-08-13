use sqlparser::ast::{self, BinaryOperator, Expr, FunctionArg, FunctionArgExpr, UnaryOperator};

use crate::dialect::SqlDialect;

use super::helpers::*;
use super::type_conversion::{datatype_to_neutral, sql_type_to_neutral};
use super::types::*;

impl<'a> Analyzer<'a> {
    pub(super) fn infer_expr_type(&mut self, expr: &Expr, scope: &Scope) -> TypeInfo {
        match expr {
            Expr::Identifier(ident) => {
                let col_name = if ident.quote_style.is_some() {
                    ident.value.clone()
                } else {
                    ident.value.to_lowercase()
                };
                self.resolve_column_in_scope(&col_name, None, scope)
            }

            Expr::CompoundIdentifier(parts) => {
                if parts.len() == 2 {
                    let qualifier = parts[0].value.to_lowercase();
                    let col_name = parts[1].value.to_lowercase();
                    self.resolve_column_in_scope(&col_name, Some(&qualifier), scope)
                } else if parts.len() >= 3 {
                    let qualifier = parts[parts.len() - 2].value.to_lowercase();
                    let col_name = parts[parts.len() - 1].value.to_lowercase();
                    self.resolve_column_in_scope(&col_name, Some(&qualifier), scope)
                } else {
                    TypeInfo::unknown()
                }
            }

            Expr::Value(vws) => {
                if let ast::Value::Number(text, _) = &vws.value {
                    // A fractional or exponent literal is not an integer --
                    // see #122. `literal_number_neutral_type` decides
                    // between int64/decimal/float64 from the literal's raw
                    // text.
                    TypeInfo::new(literal_number_neutral_type(text), false)
                } else if value_is_string(vws) {
                    // A string literal is non-nullable everywhere except
                    // Oracle's `''`, which the engine stores as NULL --
                    // see `value_is_null_in_dialect`.
                    TypeInfo::new("string", value_is_null_in_dialect(vws, self.catalog.dialect()))
                } else if value_is_boolean(vws) {
                    TypeInfo::new("bool", false)
                } else if value_is_null(vws) {
                    TypeInfo::new("unknown", true)
                } else if let Some(p) = value_is_placeholder(vws) {
                    if let Some(pos) = parse_placeholder(p) {
                        self.register_param(pos, None, None, false);
                    }
                    TypeInfo::unknown()
                } else {
                    TypeInfo::new("string", false)
                }
            }

            Expr::Cast {
                expr: inner,
                data_type,
                kind,
                ..
            } => {
                let inner_ti = self.infer_expr_type(inner, scope);
                let neutral = datatype_to_neutral(data_type, self.catalog);
                self.collect_param_type_from_cast(inner, &neutral);
                // TRY_CAST/SAFE_CAST return NULL on a conversion failure
                // instead of erroring -- that is the entire point of the
                // syntax -- so the result is nullable even when the
                // operand is proven non-null (#120).
                let nullable = match kind {
                    ast::CastKind::TryCast | ast::CastKind::SafeCast => true,
                    ast::CastKind::Cast | ast::CastKind::DoubleColon => inner_ti.nullable,
                };
                TypeInfo::new(neutral, nullable)
            }

            Expr::Function(func) => self.infer_function_type(func, scope),

            Expr::BinaryOp { left, op, right } => {
                let left_ti = self.infer_expr_type(left, scope);
                let right_ti = self.infer_expr_type(right, scope);

                match op {
                    BinaryOperator::StringConcat => TypeInfo::new("string", left_ti.nullable || right_ti.nullable),
                    BinaryOperator::Plus
                    | BinaryOperator::Minus
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo => {
                        // Widen to the wider operand type instead of always
                        // taking the left one -- see #121.
                        widen_type_info(&left_ti, &right_ti)
                    }
                    BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
                    | BinaryOperator::And
                    | BinaryOperator::Or => {
                        // SQL is three-valued: a comparison (or AND/OR) with
                        // a NULL operand yields NULL, not `false` (#119).
                        // `false AND NULL = false` is a stricter true
                        // semantics this deliberately doesn't model -- see
                        // the issue's "not worth the complexity" note.
                        TypeInfo::new("bool", left_ti.nullable || right_ti.nullable)
                    }
                    BinaryOperator::Arrow => TypeInfo::new("json", true),
                    BinaryOperator::LongArrow => TypeInfo::new("string", true),
                    BinaryOperator::HashArrow => TypeInfo::new("json", true),
                    BinaryOperator::HashLongArrow => TypeInfo::new("string", true),
                    _ => TypeInfo::new(left_ti.neutral_type, left_ti.nullable || right_ti.nullable),
                }
            }

            Expr::UnaryOp { op, expr: inner } => {
                let ti = self.infer_expr_type(inner, scope);
                match op {
                    UnaryOperator::Not => TypeInfo::new("bool", ti.nullable),
                    UnaryOperator::Minus | UnaryOperator::Plus => ti,
                    _ => ti,
                }
            }

            Expr::Nested(inner) => self.infer_expr_type(inner, scope),

            Expr::IsNull(_) | Expr::IsNotNull(_) => TypeInfo::new("bool", false),

            Expr::IsTrue(_)
            | Expr::IsFalse(_)
            | Expr::IsNotTrue(_)
            | Expr::IsNotFalse(_)
            | Expr::IsUnknown(_)
            | Expr::IsNotUnknown(_) => TypeInfo::new("bool", false),

            Expr::InList {
                expr: col_expr, list, ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                for item in list {
                    if let Expr::Value(vws) = item
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                    {
                        let col_name = expr_to_name(col_expr);
                        self.register_param(pos, Some(col_name), Some(col_ti.neutral_type.clone()), false);
                    }
                }
                // `NULL IN (1, 2)` is NULL, not false (#119).
                TypeInfo::new("bool", col_ti.nullable)
            }

            Expr::InSubquery { expr: col_expr, .. } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                TypeInfo::new("bool", col_ti.nullable)
            }

            Expr::Between {
                expr: col_expr,
                low,
                high,
                ..
            } => {
                let col_ti = self.infer_expr_type(col_expr, scope);
                let _col_name = expr_to_name(col_expr);
                self.collect_param_from_expr_with_type(low, &col_ti.neutral_type, "start");
                self.collect_param_from_expr_with_type(high, &col_ti.neutral_type, "end");
                let low_ti = self.infer_expr_type(low, scope);
                let high_ti = self.infer_expr_type(high, scope);
                // `NULL BETWEEN 1 AND 5` is NULL, not false (#119).
                TypeInfo::new("bool", col_ti.nullable || low_ti.nullable || high_ti.nullable)
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
                let col_ti = self.infer_expr_type(col_expr, scope);
                self.collect_param_from_expr_with_type(pattern, "string", &expr_to_name(col_expr));
                let pattern_ti = self.infer_expr_type(pattern, scope);
                // `NULL LIKE 'A%'` is NULL, not false (#119, #163).
                TypeInfo::new("bool", col_ti.nullable || pattern_ti.nullable)
            }

            Expr::Case {
                operand,
                conditions,
                else_result,
                ..
            } => {
                // Simple CASE (`CASE operand WHEN x THEN ...`) puts the
                // compared value in `condition`, not a boolean -- unlike
                // searched CASE (`CASE WHEN cond THEN ...`), where
                // `condition` really is boolean. A placeholder in
                // `condition` must be typed against the operand, not `bool`
                // (#125).
                let operand_ti = operand.as_ref().map(|op| self.infer_expr_type(op, scope));

                let mut result_type = "unknown".to_string();
                let mut any_nullable = false;

                for case_when in conditions {
                    let _ = self.infer_expr_type(&case_when.condition, scope);
                    if let Expr::Value(vws) = &case_when.condition
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                    {
                        match &operand_ti {
                            Some(op_ti) => {
                                let name = operand.as_ref().map(|op| expr_to_name(op));
                                self.register_param(pos, name, Some(op_ti.neutral_type.clone()), false);
                            }
                            None => {
                                self.register_param(pos, Some("flag".to_string()), Some("bool".to_string()), false);
                            }
                        }
                    }

                    let ti = self.infer_expr_type(&case_when.result, scope);
                    // Widen across every arm instead of keeping only the
                    // first non-unknown one (#121).
                    result_type = widen_neutral_type(&result_type, &ti.neutral_type);
                    let guarded = is_not_null_guard(&case_when.condition, &case_when.result, scope);
                    if ti.nullable && !guarded {
                        any_nullable = true;
                    }
                }

                if let Some(else_expr) = else_result {
                    let ti = self.infer_expr_type(else_expr, scope);
                    result_type = widen_neutral_type(&result_type, &ti.neutral_type);
                    if ti.nullable {
                        any_nullable = true;
                    }
                } else {
                    any_nullable = true;
                }

                TypeInfo::new(result_type, any_nullable)
            }

            Expr::Subquery(query) => {
                if let Ok(cols) = self.analyze_query(query)
                    && let Some(first) = cols.first()
                {
                    // A scalar subquery evaluates to SQL NULL when it matches
                    // zero rows, regardless of the projected column's own
                    // nullability — unless the query is guaranteed to return
                    // exactly one row (an ungrouped aggregate), in which case
                    // the aggregate's own nullability (already correct per
                    // function) is what determines the result.
                    let nullable = if is_single_row_aggregate_query(query) {
                        first.nullable
                    } else {
                        true
                    };
                    return TypeInfo::new(first.neutral_type.clone(), nullable);
                }
                TypeInfo::unknown()
            }

            Expr::Exists { .. } => TypeInfo::new("bool", false),

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
                TypeInfo::new("bool", false)
            }

            Expr::AllOp { left, right, .. } => {
                let left_ti = self.infer_expr_type(left, scope);
                if let Expr::Value(vws) = right.as_ref()
                    && let Some(p) = value_is_placeholder(vws)
                    && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                {
                    let array_type = format!("array<{}>", left_ti.neutral_type);
                    let name = pluralize(&expr_to_name(left));
                    self.register_param(pos, Some(name), Some(array_type), false);
                }
                TypeInfo::new("bool", false)
            }

            Expr::Array(arr) => {
                if let Some(first) = arr.elem.first() {
                    let ti = self.infer_expr_type(first, scope);
                    TypeInfo::new(format!("array<{}>", ti.neutral_type), false)
                } else {
                    TypeInfo::new("array<unknown>", false)
                }
            }

            Expr::Tuple(exprs) => {
                // A single-element tuple is just a parenthesized expression;
                // its type is that element's type. A real row constructor
                // (`ROW(a, b, ...)`, more than one element) has no single
                // neutral type today -- taking the first field's type and
                // silently dropping the rest produced a confidently wrong
                // answer (e.g. `array_agg(ROW(o.id, o.total))` inferring
                // `array<int32>`, see #117). `unknown` surfaces as an
                // unresolved-type error at codegen instead.
                match exprs.as_slice() {
                    [] => TypeInfo::unknown(),
                    [only] => self.infer_expr_type(only, scope),
                    _ => {
                        for e in exprs {
                            let _ = self.infer_expr_type(e, scope);
                        }
                        TypeInfo::unknown()
                    }
                }
            }

            Expr::Extract { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                // Dialect-dependent: PostgreSQL 14+ returns `numeric`,
                // MySQL returns an integer -- see #123.
                TypeInfo::new(extract_result_type(self.catalog.dialect()), ti.nullable)
            }

            Expr::Substring { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("string", ti.nullable)
            }

            Expr::Trim { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("string", ti.nullable)
            }

            Expr::Position {
                expr: needle,
                r#in: haystack,
            } => {
                let needle_ti = self.infer_expr_type(needle, scope);
                let haystack_ti = self.infer_expr_type(haystack, scope);
                // Neither operand was inspected before -- see #120.
                TypeInfo::new("int32", needle_ti.nullable || haystack_ti.nullable)
            }

            Expr::AtTimeZone { timestamp, .. } => {
                let ti = self.infer_expr_type(timestamp, scope);
                if ti.neutral_type == "datetime_tz" {
                    TypeInfo::new("datetime", ti.nullable)
                } else {
                    TypeInfo::new("datetime_tz", ti.nullable)
                }
            }

            Expr::TypedString(ts) => {
                let neutral = datatype_to_neutral(&ts.data_type, self.catalog);
                TypeInfo::new(neutral, false)
            }

            Expr::Interval { .. } => TypeInfo::new("interval", false),

            Expr::CompoundFieldAccess { root, access_chain } => {
                let root_ti = self.infer_expr_type(root, scope);
                if let Some(comp_name) = root_ti.neutral_type.strip_prefix("composite::")
                    && let Some(comp) = self.catalog.get_composite(comp_name)
                    && let Some(last) = access_chain.last()
                    && let ast::AccessExpr::Dot(Expr::Identifier(ident)) = last
                {
                    let field_name = ident.value.to_lowercase();
                    if let Some(field) = comp.fields.iter().find(|f| f.name == field_name) {
                        let neutral = sql_type_to_neutral(&field.sql_type, self.catalog);
                        return TypeInfo::new(neutral, true);
                    }
                }
                TypeInfo::unknown()
            }

            Expr::Ceil { expr: inner, .. } | Expr::Floor { expr: inner, .. } => {
                let ti = self.infer_expr_type(inner, scope);
                TypeInfo::new(ti.neutral_type, ti.nullable)
            }

            _ => TypeInfo::unknown(),
        }
    }

    pub(super) fn resolve_column_in_scope(&self, col_name: &str, qualifier: Option<&str>, scope: &Scope) -> TypeInfo {
        if let Some(qual) = qualifier {
            for source in &scope.sources {
                if (source.alias == qual || source.table_name == qual)
                    && let Some(col) = source.columns.iter().find(|c| c.name == col_name)
                {
                    return TypeInfo::from_scope_column(
                        col.sql_type.clone(),
                        col.neutral_type.clone(),
                        col.base_nullable,
                        &source.alias,
                        source.nullable_from_join,
                    );
                }
            }
        } else {
            let mut found: Option<TypeInfo> = None;
            for source in &scope.sources {
                if let Some(col) = source.columns.iter().find(|c| c.name == col_name) {
                    let ti = TypeInfo::from_scope_column(
                        col.sql_type.clone(),
                        col.neutral_type.clone(),
                        col.base_nullable,
                        &source.alias,
                        source.nullable_from_join,
                    );
                    if found.is_some() {
                        return TypeInfo::new(format!("{AMBIGUOUS_COLUMN_MARKER}{col_name}"), false);
                    }
                    found = Some(ti);
                }
            }
            if let Some(ti) = found {
                return ti;
            }
        }

        let has_sources = scope.sources.iter().any(|s| !s.columns.is_empty());
        if has_sources {
            return TypeInfo::new(format!("{UNKNOWN_COLUMN_MARKER}{col_name}"), true);
        }

        TypeInfo::unknown()
    }

    pub(super) fn infer_function_type(&mut self, func: &ast::Function, scope: &Scope) -> TypeInfo {
        let func_name = object_name_to_string(&func.name).to_lowercase();
        let is_window = func.over.is_some();

        let first_arg_ti = self.get_first_arg_type(func, scope);
        let first_arg_nullable = first_arg_ti.as_ref().map(|ti| ti.nullable).unwrap_or(true);

        match func_name.as_str() {
            "count" => TypeInfo::new("int64", false),
            "sum" => {
                // See `sum_result_type` for the engine semantics this mirrors.
                let base_type = first_arg_ti
                    .as_ref()
                    .map(|ti| sum_result_type(&ti.neutral_type))
                    .unwrap_or_else(|| "int64".to_string());
                if is_window {
                    TypeInfo::new(base_type, false)
                } else {
                    TypeInfo::new(base_type, true)
                }
            }
            "avg" => {
                // See `avg_result_type` for the engine semantics this mirrors.
                let base_type = first_arg_ti
                    .as_ref()
                    .map(|ti| avg_result_type(&ti.neutral_type))
                    .unwrap_or_else(|| "decimal".to_string());
                if is_window {
                    TypeInfo::new(base_type, false)
                } else {
                    TypeInfo::new(base_type, true)
                }
            }
            "min" | "max" => {
                let base_type = first_arg_ti
                    .as_ref()
                    .map(|ti| ti.neutral_type.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                if is_window {
                    TypeInfo::new(base_type, first_arg_nullable)
                } else {
                    TypeInfo::new(base_type, true)
                }
            }
            "string_agg" | "array_agg" => {
                let base_type = if func_name == "string_agg" {
                    "string".to_string()
                } else {
                    let inner = first_arg_ti
                        .as_ref()
                        .map(|ti| ti.neutral_type.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    format!("array<{}>", inner)
                };
                TypeInfo::new(base_type, true)
            }
            "bool_and" | "bool_or" | "every" => TypeInfo::new("bool", true),
            // ~keep `jsonb_agg` is `json_agg` with a different storage type, not a
            // different shape: both aggregate one JSON object per row into a
            // JSON array, and `sql_type_to_neutral` already collapses
            // `json`/`jsonb` onto the same neutral `json`. Splitting them here
            // would mean `jsonb_agg(o.*)` — the form most PostgreSQL code
            // actually writes — silently losing the nested struct that
            // `json_agg(o.*)` gets.
            "json_agg" | "jsonb_agg" => self
                .infer_nested_aggregate_type(func, scope, WrapArray::Yes)
                .unwrap_or_else(|| TypeInfo::new("json", true)),
            // ~keep `json_object_agg(k, v)` deliberately does NOT get nested-struct
            // inference, unlike its `json_agg` neighbour above. It builds a
            // JSON *object keyed by the runtime values of its first argument*
            // (`json_object_agg(o.id, o.status)` over one row yields
            // `{"1": "shipped"}`), so the result has no fixed field set to
            // synthesize a struct from — the keys are data, not schema. The
            // `json_nested<...>` container can only express "array of T" or
            // "single T" anyway (see `nested_struct_shape` in scythe-codegen),
            // and there is no map shape to put a row type into. A flat `json`
            // is the honest answer.
            //
            // Nullability is `true` for all four: an aggregate over zero rows
            // returns SQL NULL, not `[]`/`{}` (verified against PostgreSQL 16;
            // only `count` is exempt — see `is_aggregate_function_name`).
            "json_object_agg" | "jsonb_object_agg" => TypeInfo::new("json", true),
            "row_to_json" => self
                .infer_nested_aggregate_type(func, scope, WrapArray::No)
                .unwrap_or_else(|| TypeInfo::new(format!("{UNKNOWN_FUNCTION_MARKER}{func_name}"), first_arg_nullable)),

            "coalesce" => {
                let args = self.get_function_args(func);
                let mut result_type = "unknown".to_string();
                let mut any_non_nullable = false;
                let mut coalesce_name: Option<String> = None;

                for arg in &args {
                    let ti = self.infer_expr_type(arg, scope);
                    // Widen across every argument instead of keeping only
                    // the first non-unknown one (#121).
                    result_type = widen_neutral_type(&result_type, &ti.neutral_type);
                    // `is_non_null_literal`, not `is_literal`: on Oracle a
                    // `''` fallback proves nothing, because the engine
                    // returns NULL for it.
                    if !ti.nullable || is_non_null_literal(arg, self.catalog.dialect()) {
                        any_non_nullable = true;
                    }
                    if coalesce_name.is_none()
                        && !matches!(arg, Expr::Value(vws) if value_is_placeholder(vws).is_some())
                    {
                        let n = expr_to_name(arg);
                        if n != "unknown" {
                            coalesce_name = Some(n);
                        }
                    }
                }

                for arg in &args {
                    if let Expr::Value(vws) = arg
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p, vws.span)
                    {
                        let param_type = if result_type != "unknown" {
                            Some(result_type.clone())
                        } else {
                            None
                        };
                        self.register_param(pos, coalesce_name.clone(), param_type, true);
                    }
                }

                TypeInfo::new(result_type, !any_non_nullable)
            }

            "nullif" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, true)
            }

            // The explicit `ROW(a, b, ...)` row-constructor syntax parses as
            // a plain function call named "row" (sqlparser has no dedicated
            // AST node for it), not as `Expr::Tuple` -- so it needs the same
            // "no single neutral type for more than one field" treatment
            // `Expr::Tuple` gets, for the same reason (#117): silently
            // collapsing to the first field's type produced a confidently
            // wrong answer for e.g. `array_agg(ROW(o.id, o.total))`.
            "row" => {
                let args = self.get_function_args(func);
                match args.as_slice() {
                    [] => TypeInfo::unknown(),
                    [only] => self.infer_expr_type(only, scope),
                    _ => {
                        for arg in &args {
                            let _ = self.infer_expr_type(arg, scope);
                        }
                        TypeInfo::unknown()
                    }
                }
            }

            "upper" | "lower" | "initcap" | "reverse" | "ltrim" | "rtrim" | "btrim" | "lpad" | "rpad" | "repeat"
            | "replace" | "translate" | "left" | "right" | "md5" | "encode" | "decode" | "chr" | "to_hex"
            | "quote_ident" | "quote_literal" | "format" | "regexp_replace" => {
                TypeInfo::new("string", first_arg_nullable)
            }
            "concat" | "concat_ws" => {
                // Dialect-blind before: true on PostgreSQL (`concat`
                // ignores NULL arguments), false on MySQL (any NULL
                // argument makes the whole result NULL) -- see #120.
                let nullable = self.catalog.dialect() == SqlDialect::MySQL && self.any_arg_nullable(func, scope);
                TypeInfo::new("string", nullable)
            }
            "substring" | "substr" => TypeInfo::new("string", first_arg_nullable),
            "length" | "char_length" | "character_length" | "octet_length" | "bit_length" | "strpos" => {
                TypeInfo::new("int32", first_arg_nullable)
            }

            "abs" | "sign" => first_arg_ti.unwrap_or_else(TypeInfo::unknown),
            "ceil" | "ceiling" | "floor" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, ti.nullable)
            }
            "round" | "trunc" => {
                // `ROUND(double precision)` returns `double precision`, not
                // `numeric`, on PostgreSQL -- only the exact-numeric
                // overload does (#123).
                let input_type = first_arg_ti
                    .as_ref()
                    .map(|ti| ti.neutral_type.as_str())
                    .unwrap_or("decimal");
                TypeInfo::new(round_result_type(input_type), first_arg_nullable)
            }
            // `pi`/`random` take no operand that could be NULL; every other
            // function here previously hardcoded non-null while ignoring
            // `first_arg_nullable`, which was already computed and unused
            // (#120).
            "pi" | "random" => TypeInfo::new("float64", false),
            "power" | "sqrt" | "cbrt" | "log" | "ln" | "exp" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan"
            | "atan2" | "degrees" | "radians" => TypeInfo::new("float64", first_arg_nullable),
            "mod" => first_arg_ti.unwrap_or_else(|| TypeInfo::new("int32", false)),
            "div" => TypeInfo::new("int64", first_arg_nullable),
            "greatest" | "least" => {
                let args = self.get_function_args(func);
                let mut result_type = "unknown".to_string();
                // Whether at least one argument is proven non-null. On
                // every dialect except MySQL, GREATEST/LEAST ignore NULLs
                // and only return NULL when *every* argument is NULL, so a
                // single proven-non-null argument guarantees a non-null
                // result -- the same reasoning as COALESCE.
                let mut any_proven_non_null = false;
                let mut any_nullable_arg = false;
                for arg in &args {
                    let ti = self.infer_expr_type(arg, scope);
                    // Widen across every argument instead of taking only
                    // the first one for both type and nullability (#121).
                    result_type = widen_neutral_type(&result_type, &ti.neutral_type);
                    if ti.nullable {
                        any_nullable_arg = true;
                    } else {
                        any_proven_non_null = true;
                    }
                }
                let nullable = if self.catalog.dialect() == SqlDialect::MySQL {
                    // MySQL: NULL if any argument is NULL.
                    any_nullable_arg
                } else {
                    !any_proven_non_null
                };
                TypeInfo::new(result_type, nullable)
            }

            "now" | "current_timestamp" | "statement_timestamp" | "transaction_timestamp" | "clock_timestamp" => {
                TypeInfo::new("datetime_tz", false)
            }
            // Unlike `current_date`/`localdate` (0-arg, genuinely never
            // null), `date` is a conversion function over an argument that
            // can itself be nullable (#120).
            "current_date" | "localdate" => TypeInfo::new("date", false),
            "date" => TypeInfo::new("date", first_arg_nullable),
            "current_time" | "localtime" => TypeInfo::new("time_tz", false),
            "date_trunc" => {
                let args = self.get_function_args(func);
                if args.len() >= 2 {
                    let ti = self.infer_expr_type(&args[1], scope);
                    TypeInfo::new(ti.neutral_type, ti.nullable)
                } else {
                    TypeInfo::new("datetime_tz", first_arg_nullable)
                }
            }
            // `date_part(text, source)` (the PostgreSQL function, as opposed
            // to the `EXTRACT(field FROM source)` syntax) always returns
            // `double precision`, on every PostgreSQL version -- the
            // PG14+ `numeric` change applies only to `EXTRACT`. A bare
            // `extract(...)` function call (some dialects parse it that way
            // instead of the dedicated `Expr::Extract` node) gets the same
            // dialect-aware answer as the `EXTRACT` syntax (#123).
            "date_part" => TypeInfo::new("float64", first_arg_nullable),
            "extract" => TypeInfo::new(extract_result_type(self.catalog.dialect()), first_arg_nullable),
            "age" => {
                let nullable = self.any_arg_nullable(func, scope);
                TypeInfo::new("interval", nullable)
            }
            "make_date" => TypeInfo::new("date", self.any_arg_nullable(func, scope)),
            "make_time" => TypeInfo::new("time", self.any_arg_nullable(func, scope)),
            "make_timestamp" => TypeInfo::new("datetime", self.any_arg_nullable(func, scope)),
            "make_timestamptz" => TypeInfo::new("datetime_tz", self.any_arg_nullable(func, scope)),
            "make_interval" => TypeInfo::new("interval", self.any_arg_nullable(func, scope)),
            "to_timestamp" => TypeInfo::new("datetime_tz", first_arg_nullable),
            "to_date" => TypeInfo::new("date", first_arg_nullable),
            "to_char" => TypeInfo::new("string", first_arg_nullable),

            "row_number" | "rank" | "dense_rank" | "cume_dist" | "ntile" | "percent_rank" => {
                TypeInfo::new("int64", false)
            }
            "lag" | "lead" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                // A three-argument LAG/LEAD returns the third argument (the
                // default) instead of NULL at partition boundaries, so the
                // result is non-null only when both the tracked expression
                // and the default are non-null. With fewer than three
                // arguments the boundary genuinely returns NULL.
                //
                // `IGNORE NULLS` changes which rows the offset counts over
                // and can exhaust the partition even when both operands are
                // non-null, so it forces nullable regardless of arity. See
                // `function_has_null_treatment`.
                let nullable = if function_has_null_treatment(func) {
                    true
                } else {
                    let args = self.get_function_args(func);
                    if args.len() >= 3 {
                        let default_ti = self.infer_expr_type(&args[2], scope);
                        ti.nullable || default_ti.nullable
                    } else {
                        true
                    }
                };
                TypeInfo::new(ti.neutral_type, nullable)
            }
            "first_value" | "last_value" | "nth_value" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, true)
            }

            // ~keep The `json_build_*` family is `proisstrict = f` in `pg_proc`:
            // `json_build_object('a', NULL)` yields `{"a": null}`, a JSON null
            // *inside* a non-NULL document, so the column itself never goes
            // SQL NULL. That is why these stay unconditionally non-nullable
            // and must not be folded into the `to_json` arm below.
            "json_build_object" | "jsonb_build_object" | "json_build_array" | "jsonb_build_array" => {
                TypeInfo::new("json", false)
            }
            // ~keep `to_json(o.*)` / `to_jsonb(o)` over a whole-row reference is
            // `row_to_json` spelled differently — PostgreSQL 16 returns the
            // identical document for all three — so it gets the same
            // `WrapArray::No` nested single-object inference. `to_json` is not
            // an aggregate, so there is no array to wrap.
            //
            // The fallback is the scalar case (`to_json(o.notes)`), which
            // `row_to_json` cannot express and which is why this arm does not
            // simply share `row_to_json`'s `UNKNOWN_FUNCTION_MARKER` fallback:
            // a scalar conversion really is a plain `json`.
            //
            // Nullability follows the argument rather than being hardcoded
            // `false`: all three are `proisstrict = t` in `pg_proc`, so
            // `to_json(NULL::text)` is SQL NULL, not the JSON document `null`
            // (verified against PostgreSQL 16). The nested path is nullable
            // for the same reason `row_to_json`'s is: on a null-extended row
            // from an outer join the whole-row variable is itself NULL.
            "to_json" | "to_jsonb" => self
                .infer_nested_aggregate_type(func, scope, WrapArray::No)
                .unwrap_or_else(|| TypeInfo::new("json", first_arg_nullable)),
            // ~keep Strict too (`proisstrict = t`), so it belongs with `to_json`
            // rather than with the `json_build_*` family it used to share an
            // arm with: `json_strip_nulls(NULL::json)` is SQL NULL, not the
            // document `null`. It gets no nested inference because it takes a
            // JSON document, not a whole-row reference -- the shape it strips
            // from is whatever its argument already was.
            "json_strip_nulls" | "jsonb_strip_nulls" => TypeInfo::new("json", first_arg_nullable),
            // ~keep `array_to_json` is `proisstrict = t`, so the column is SQL NULL
            // exactly when an argument is -- including the common
            // `array_to_json(array_agg(x))` shape, where the inner aggregate
            // over zero rows is already NULL and carries that through.
            // Nothing else makes it NULL: an empty array converts to the
            // document `[]` and a NULL *element* becomes a JSON `null`
            // inside a non-NULL document (`array_to_json(ARRAY[1,NULL,3])`
            // is `[1,null,3]`).
            //
            // `any_arg_nullable`, not `first_arg_nullable`, because the
            // two-argument `array_to_json(anyarray, boolean)` pretty-print
            // form is strict in the flag too.
            //
            // No nested-struct inference: the argument is an array
            // expression, not the whole-row reference
            // `infer_nested_aggregate_type` needs.
            "array_to_json" => {
                let nullable = self.any_arg_nullable(func, scope);
                TypeInfo::new("json", nullable)
            }
            // ~keep `json_object(text[])` and `json_object(text[], text[])` are both
            // strict, and neither has a second NULL source: an empty array
            // yields the document `{}` and an odd-length one raises
            // `array must have even number of elements` rather than
            // returning NULL.
            "json_object" | "jsonb_object" => {
                let nullable = self.any_arg_nullable(func, scope);
                TypeInfo::new("json", nullable)
            }
            // ~keep Both are strict in *every* argument, not just the document:
            // `jsonb_set('{"a":1}', '{a}', NULL)` and
            // `jsonb_set('{"a":1}', NULL, '1')` are each SQL NULL, so a
            // nullable replacement value or path makes the column nullable.
            "jsonb_set" | "jsonb_insert" => {
                let nullable = self.any_arg_nullable(func, scope);
                TypeInfo::new("json", nullable)
            }
            // ~keep The one JSON function where `proisstrict` is not the whole
            // story in the *other* direction. `jsonb_set_lax` is
            // `proisstrict = f`, yet a NULL target or a NULL path still
            // yields SQL NULL -- only the third argument, the replacement,
            // gets the lenient treatment `null_value_treatment` selects
            // (`use_json_null` embeds `null`, `return_target` returns the
            // document unchanged, `delete_key` drops the key; none of them
            // returns SQL NULL). So nullability comes from the first two
            // arguments alone, and a nullable replacement -- the whole point
            // of reaching for `_lax` over `jsonb_set` -- must not be allowed
            // to infect the column.
            "jsonb_set_lax" => {
                let args = self.get_function_args(func);
                // Fewer than the three mandatory arguments means a malformed
                // call; stay conservative rather than claim non-null.
                let nullable =
                    args.len() < 3 || args.iter().take(2).any(|arg| self.infer_expr_type(arg, scope).nullable);
                TypeInfo::new("json", nullable)
            }
            // ~keep Strict, and no second NULL source: every JSON value has a type
            // name, and the JSON `null` document reports the *string*
            // `'null'`, not SQL NULL. So the column is nullable exactly when
            // the argument is, rather than unconditionally.
            "json_typeof" | "jsonb_typeof" => TypeInfo::new("string", first_arg_nullable),
            // ~keep Strict *and* unconditionally nullable, which is not a
            // contradiction and is why these do not follow the
            // `json_typeof` rule above: a path that does not exist in a
            // perfectly non-NULL document returns SQL NULL
            // (`json_extract_path('{"a":1}', 'b')` is NULL). Path presence
            // is data, not schema, so scythe cannot prove the lookup hits.
            "json_extract_path_text" | "jsonb_extract_path_text" => TypeInfo::new("string", true),
            "json_extract_path" | "jsonb_extract_path" => TypeInfo::new("json", true),
            // ~keep Strict, and a non-NULL argument can only produce a number or
            // an error (`json_array_length('{"a":1}')` raises "cannot get
            // array length of a non-array"), never SQL NULL -- an empty
            // array is `0`. Nullability therefore follows the argument.
            "json_array_length" | "jsonb_array_length" => TypeInfo::new("int32", first_arg_nullable),
            // ~keep Strict, returns `text`; the pretty-printed rendering of a
            // non-NULL document is never SQL NULL.
            "jsonb_pretty" => TypeInfo::new("string", first_arg_nullable),
            // ~keep In FROM position (`FROM json_each(j) AS kv`) these expand into
            // two real columns -- see the `known_functions` match in
            // `scope.rs`, which types `key` as `string` and `value` as
            // `json`/`string`. In *select-list* position
            // (`SELECT json_each(j) FROM t`), PostgreSQL's SRF-in-target-list
            // extension instead yields a single column of the pseudo-type
            // `record` -- an anonymous composite, not a scalar. The neutral
            // type vocabulary can name a *catalog* composite
            // (`composite::{name}`) or a *synthesized* one for
            // `json_agg`/`row_to_json` (`json_nested<...>`, which tells every
            // backend "decode this value as embedded JSON"), but neither fits
            // here: there is no catalog entry to point `composite::` at, and
            // the value on the wire is a native PostgreSQL record, not JSON
            // text, so `json_nested<...>` would tell backends to decode it
            // the wrong way. `string` was worse still -- a silent wrong
            // answer, not merely an imprecise one. `unknown` is the same
            // honest fallback already used a few lines down for
            // `json_populate_record`/`jsonb_populate_recordset`, another
            // record-shaped result the vocabulary cannot name.
            "json_each" | "jsonb_each" | "json_each_text" | "jsonb_each_text" => TypeInfo::new("unknown", true),
            "json_object_keys" | "jsonb_object_keys" => TypeInfo::new("string", false),
            "json_populate_record"
            | "jsonb_populate_record"
            | "json_populate_recordset"
            | "jsonb_populate_recordset" => TypeInfo::new("unknown", true),
            // ~keep Unlike `json_each`, these are `SETOF json`/`SETOF jsonb` --
            // one scalar JSON value per row, not a record -- so in
            // select-list position (`SELECT json_array_elements(j) FROM t`)
            // the column genuinely is `json`, matching the `value` column
            // FROM position already assigns in `scope.rs`. Previously
            // unhandled here, these fell through to the catch-all arm below
            // and were reported as an unknown-function error rather than
            // resolved.
            "json_array_elements" | "jsonb_array_elements" => TypeInfo::new("json", true),

            "array_length" | "array_ndims" | "array_lower" | "array_upper" | "cardinality" => {
                TypeInfo::new("int32", true)
            }
            "array_cat" | "array_append" | "array_prepend" | "array_remove" | "array_replace" | "array_positions" => {
                first_arg_ti.unwrap_or_else(TypeInfo::unknown)
            }
            "array_position" => TypeInfo::new("int32", true),
            "array_to_string" => TypeInfo::new("string", true),
            "unnest" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                let inner = if ti.neutral_type.starts_with("array<") && ti.neutral_type.ends_with('>') {
                    ti.neutral_type[6..ti.neutral_type.len() - 1].to_string()
                } else {
                    "unknown".to_string()
                };
                TypeInfo::new(inner, true)
            }

            "gen_random_uuid" | "uuid_generate_v4" => TypeInfo::new("uuid", false),
            "nextval" | "currval" | "lastval" | "setval" => TypeInfo::new("int64", false),
            "pg_typeof" => TypeInfo::new("string", false),

            _ => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(format!("{UNKNOWN_FUNCTION_MARKER}{func_name}"), ti.nullable)
            }
        }
    }

    pub(super) fn get_first_arg_type(&mut self, func: &ast::Function, scope: &Scope) -> Option<TypeInfo> {
        let args = self.get_function_args(func);
        args.first().map(|arg| self.infer_expr_type(arg, scope))
    }

    /// Whether any argument of `func` is nullable -- the nullability rule
    /// for functions where any operand being NULL can make the whole call
    /// NULL (date/time constructors like `make_date`, `age`; MySQL's
    /// `CONCAT`), as opposed to the single-first-argument heuristic most
    /// other arms in [`Analyzer::infer_function_type`] use (#120).
    pub(super) fn any_arg_nullable(&mut self, func: &ast::Function, scope: &Scope) -> bool {
        let args = self.get_function_args(func);
        args.iter().any(|arg| self.infer_expr_type(arg, scope).nullable)
    }

    pub(super) fn get_function_args(&self, func: &ast::Function) -> Vec<Expr> {
        match &func.args {
            ast::FunctionArguments::List(arg_list) => function_arg_exprs(&arg_list.args),
            _ => Vec::new(),
        }
    }

    /// Widened view of a function's argument list that preserves wildcard and
    /// relation-reference shapes `get_function_args` drops.
    ///
    /// Deliberately a sibling, not a replacement: `get_function_args` feeds
    /// `get_first_arg_type`, which in turn feeds `sum`, `avg`, `min`/`max`,
    /// `array_agg`, `lag`/`lead`, `first_value`, `unnest`, `array_cat` and the
    /// catch-all arm of [`Analyzer::infer_function_type`]. Widening that path
    /// would silently change how `array_agg(o.*)` and nullability derived
    /// from `first_arg_nullable` behave. Only the PostgreSQL nested-aggregate
    /// arms use this method; every other call site is untouched.
    ///
    /// **Arity is not guaranteed to match the source argument list.** Like
    /// `get_function_args`, this uses `filter_map`: a `FunctionArg` variant
    /// this match doesn't recognize (currently `ExprNamed`, sqlparser's
    /// arbitrary-expression-as-name form) is silently dropped rather than
    /// represented as a shape. Every current caller passes a single-argument
    /// aggregate call and checks `shapes.len() == 1` before indexing, so a
    /// dropped argument shows up as a length mismatch and is caught, not
    /// misread as a different argument. A caller that needs positional
    /// correspondence with the source list must not assume `shapes[i]`
    /// corresponds to `arg_list.args[i]`.
    ///
    /// Consumed by [`Analyzer::infer_nested_aggregate_type`] for the
    /// PostgreSQL `json_agg`/`row_to_json` nested-struct arms.
    pub(super) fn get_function_arg_shapes(&self, func: &ast::Function, scope: &Scope) -> Vec<FuncArgShape> {
        let ast::FunctionArguments::List(arg_list) = &func.args else {
            return Vec::new();
        };

        arg_list
            .args
            .iter()
            .filter_map(|arg| {
                let fae = match arg {
                    FunctionArg::Unnamed(fae) | FunctionArg::Named { arg: fae, .. } => fae,
                    _ => return None,
                };
                Some(self.classify_function_arg_expr(fae, scope))
            })
            .collect()
    }

    fn classify_function_arg_expr(&self, fae: &FunctionArgExpr, scope: &Scope) -> FuncArgShape {
        match fae {
            FunctionArgExpr::Expr(Expr::Identifier(ident)) => {
                let name = if ident.quote_style.is_some() {
                    ident.value.clone()
                } else {
                    ident.value.to_lowercase()
                };
                match self.scope_relation_alias(&name, scope) {
                    Some(alias) => FuncArgShape::Relation(alias),
                    None => FuncArgShape::Expr(Box::new(Expr::Identifier(ident.clone()))),
                }
            }
            FunctionArgExpr::Expr(e) => FuncArgShape::Expr(Box::new(e.clone())),
            FunctionArgExpr::QualifiedWildcard(object_name) => {
                let qualifier = object_name_to_string(object_name).to_lowercase();
                match self.find_scope_source_alias(&qualifier, scope) {
                    Some(alias) => FuncArgShape::Relation(alias),
                    None => FuncArgShape::Wildcard,
                }
            }
            FunctionArgExpr::Wildcard | FunctionArgExpr::WildcardWithOptions(_) => FuncArgShape::Wildcard,
        }
    }

    /// Resolve a bare identifier to the scope source it names, but only when
    /// it is unambiguously a relation reference (`json_agg(o)` where `o` is
    /// the `orders o` alias) rather than a column that happens to share the
    /// name.
    fn scope_relation_alias(&self, name: &str, scope: &Scope) -> Option<String> {
        let is_column = scope.sources.iter().any(|s| s.columns.iter().any(|c| c.name == name));
        if is_column {
            return None;
        }
        self.find_scope_source_alias(name, scope)
    }

    /// Resolve a name to the alias of the scope source it matches (by alias
    /// or table name), mirroring the `o.*` expansion in `statements.rs`.
    fn find_scope_source_alias(&self, name: &str, scope: &Scope) -> Option<String> {
        scope
            .sources
            .iter()
            .find(|s| s.alias == name || s.table_name == name)
            .map(|s| s.alias.clone())
    }

    /// PostgreSQL-only nested-struct type inference for
    /// `json_agg`/`jsonb_agg` over a relation wildcard (or the
    /// bare-identifier form `json_agg(relation)`) and for
    /// `row_to_json`/`to_json`/`to_jsonb` over one.
    ///
    /// `WrapArray::Yes` wraps the placeholder in `array<>` for the two
    /// aggregates (one JSON array element per row aggregated);
    /// `WrapArray::No` leaves it bare for the three row-to-document
    /// conversions (one JSON object per output row, not an aggregate).
    ///
    /// Returns `None` whenever the nested shape can't be established — wrong
    /// dialect or engine, zero or more than one argument, or an argument that
    /// isn't a `FuncArgShape::Relation` (a bare wildcard, a scalar expression,
    /// or a relation alias that somehow resolved to no scope columns). Every
    /// caller falls back to the pre-existing behaviour for that function on
    /// `None`, so this never changes output for anything it doesn't
    /// explicitly handle.
    fn infer_nested_aggregate_type(
        &mut self,
        func: &ast::Function,
        scope: &Scope,
        wrap: WrapArray,
    ) -> Option<TypeInfo> {
        if !catalog_has_nested_aggregates(self.catalog) {
            return None;
        }

        let shapes = self.get_function_arg_shapes(func, scope);
        let [FuncArgShape::Relation(alias)] = shapes.as_slice() else {
            return None;
        };

        let fields = self.nested_fields_for_relation(alias, scope);
        if fields.is_empty() {
            return None;
        }

        // ~keep Nested-of-nested: `alias` is a CTE or derived-subquery column
        // whose own neutral_type is itself an unresolved `__nested__{id}`
        // placeholder (e.g. an outer json_agg(oi.*) over a CTE column that
        // is itself the result of an inner json_agg). Phase 2 naming
        // (resolve_nested_struct_names) only walks the query's own
        // top-level output columns, not recursively into the fields of the
        // NestedStructInfo it just built, so a placeholder embedded here
        // would never be substituted -- it would reach resolve_type in
        // every backend's generate_nested_struct_def, including opted-in
        // ones, as an unresolvable type name. Reject with a clear
        // diagnostic instead of leaking that placeholder into a
        // downstream "unknown type" error.
        if let Some(field) = fields.iter().find(|f| f.neutral_type.contains("__nested__")) {
            self.type_errors.push(format!(
                "nested aggregate over nested aggregate is not supported: field \"{}\" of \"{alias}\" is itself \
                 a json_agg/row_to_json result; wrap only one level of aggregation per query",
                field.name
            ));
            return None;
        }

        let elements_nullable = scope
            .sources
            .iter()
            .find(|s| s.alias == *alias)
            .is_some_and(|s| s.nullable_from_join);

        let id = self.push_pending_nested(fields);
        let placeholder = format!("__nested__{id}");

        // ~keep Element nullability, not field nullability, is the axis an outer
        // join moves. For a LEFT JOIN row with no match PostgreSQL makes the
        // whole-row variable itself NULL — not a row of NULLs — so
        // `json_agg(o.*)` aggregates one NULL and the column's value is the
        // JSON array `[null]`, never `[{"id":null,...}]`. Widening the
        // *fields* would therefore model a value PostgreSQL never produces
        // while still leaving `Vec<Foo>` / `list[Foo]` unable to hold the one
        // it does: `serde_json` rejects `[null]` into `Vec<Foo>` with
        // "invalid type: null", and Python's `[Foo(...) for item in raw]`
        // raises on the NULL element.
        //
        // Deliberately conservative: `json_agg(o.*) FILTER (WHERE o.id IS NOT
        // NULL)` — the idiom for suppressing exactly that `[null]` — cannot
        // produce a null element, but recognising that would mean proving an
        // arbitrary filter excludes the non-matching rows. Over-approximating
        // costs an `Option`/`| None` that is always `Some`; under-
        // approximating is a runtime deserialization failure, so this errs
        // toward the former.
        let element = if elements_nullable {
            format!("nullable<{placeholder}>")
        } else {
            placeholder.clone()
        };
        let neutral_type = match wrap {
            WrapArray::Yes => format!("json_nested<array<{element}>>"),
            // ~keep `row_to_json(o.*)` over a null-extended row returns SQL NULL,
            // not a JSON null, so the *column* is nullable (it always is
            // here) and there is no element to wrap.
            WrapArray::No => format!("json_nested<{placeholder}>"),
        };
        Some(TypeInfo::new(neutral_type, true))
    }

    /// Build the field list for a nested struct from a scope source's
    /// columns.
    ///
    /// Unlike [`TypeInfo::from_scope_column`] for a plain column reference,
    /// `nullable_from_join` is deliberately *not* folded in here — see
    /// [`Analyzer::infer_nested_aggregate_type`], which applies an outer
    /// join's effect to the array element instead. Inside a JSON object that
    /// `json_agg` actually emitted, every field carries its own schema
    /// nullability and nothing more.
    fn nested_fields_for_relation(&self, alias: &str, scope: &Scope) -> Vec<NestedFieldInfo> {
        let Some(source) = scope.sources.iter().find(|s| s.alias == alias) else {
            return Vec::new();
        };
        source
            .columns
            .iter()
            .map(|col| NestedFieldInfo {
                name: col.name.clone(),
                neutral_type: col.neutral_type.clone(),
                nullable: col.base_nullable,
            })
            .collect()
    }
}

/// Whether nested-aggregate inference is available for this catalog.
///
/// Two independent conditions, because neither implies the other:
/// - the dialect must be PostgreSQL, since `json_agg`/`row_to_json` and the
///   whole-row `alias.*` argument form are PostgreSQL syntax; and
/// - the *engine*, when stated, must actually ship those functions.
///   `SqlDialect::from_str` maps `redshift` and `duckdb` onto
///   `SqlDialect::PostgreSQL`, but Redshift has no `json_agg` at all and
///   DuckDB spells it `json_group_array`, so the dialect check alone admits
///   two engines where the inferred type could never be produced.
///
/// An unstated engine (`Catalog::from_ddl`, every unit test, any embedder
/// predating `Catalog::with_engine`) is treated as PostgreSQL proper.
fn catalog_has_nested_aggregates(catalog: &crate::catalog::Catalog) -> bool {
    if catalog.dialect() != SqlDialect::PostgreSQL {
        return false;
    }
    catalog
        .engine()
        .is_none_or(|engine| matches!(engine, "postgresql" | "postgres" | "pg" | "cockroachdb" | "crdb"))
}

/// Whether [`Analyzer::infer_nested_aggregate_type`] wraps its placeholder in
/// `array<>` (`json_agg`/`jsonb_agg`, one element per aggregated row) or
/// leaves it bare (`row_to_json`/`to_json`/`to_jsonb`, one object per output
/// row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapArray {
    Yes,
    No,
}

/// Shape of a single function argument, widened from sqlparser's
/// `FunctionArgExpr` to preserve wildcard and relation-reference forms that
/// [`Analyzer::get_function_args`] silently drops.
///
/// `infer_nested_aggregate_type` only ever needs to distinguish `Relation`
/// from everything else, so `Expr`'s payload is currently read by tests only
/// (see `test_get_function_arg_shapes_plain_expr_unaffected`) — kept for a
/// caller that needs the actual expression, not because it's unused.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(super) enum FuncArgShape {
    /// A normal scalar/column expression argument. Boxed: `Expr` is ~328
    /// bytes and this enum is carried by value through `Vec<FuncArgShape>`.
    Expr(Box<Expr>),
    /// `*`, or `alias.*` whose qualifier did not resolve to a scope source.
    Wildcard,
    /// `alias.*`, or a bare identifier that names a scope source (table
    /// alias or table name) rather than a column — e.g. `json_agg(o)` where
    /// `o` is the `orders o` alias. Carries the resolved alias.
    Relation(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use ahash::AHashMap;
    use sqlparser::ast::{
        Function, FunctionArg, FunctionArgExpr, FunctionArgumentClause, FunctionArgumentList, FunctionArguments, Ident,
        NullTreatment, ObjectName, ObjectNamePart, Value, ValueWithSpan, WildcardAdditionalOptions, WindowFrame,
        WindowFrameBound, WindowFrameUnits, WindowSpec, WindowType,
    };
    use sqlparser::tokenizer::Span;

    fn empty_catalog() -> Catalog {
        Catalog::from_ddl(&[]).unwrap()
    }

    fn empty_catalog_with_dialect(dialect: crate::dialect::SqlDialect) -> Catalog {
        Catalog::from_ddl_with_dialect(&[], &dialect).unwrap()
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

    fn empty_scope() -> Scope {
        Scope { sources: Vec::new() }
    }

    fn make_func(name: &str, args: Vec<Expr>) -> ast::Function {
        let func_args = args
            .into_iter()
            .map(|e| FunctionArg::Unnamed(FunctionArgExpr::Expr(e)))
            .collect();
        Function {
            name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new(name))]),
            args: FunctionArguments::List(FunctionArgumentList {
                args: func_args,
                duplicate_treatment: None,
                clauses: Vec::new(),
            }),
            filter: None,
            over: None,
            null_treatment: None,
            within_group: Vec::new(),
            parameters: FunctionArguments::None,
            uses_odbc_syntax: false,
        }
    }

    fn make_window_func(name: &str, args: Vec<Expr>) -> ast::Function {
        let mut f = make_func(name, args);
        f.over = Some(WindowType::WindowSpec(WindowSpec {
            window_name: None,
            partition_by: Vec::new(),
            order_by: Vec::new(),
            window_frame: Some(WindowFrame {
                units: WindowFrameUnits::Rows,
                start_bound: WindowFrameBound::CurrentRow,
                end_bound: None,
            }),
        }));
        f
    }

    fn make_no_arg_func(name: &str) -> ast::Function {
        Function {
            name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new(name))]),
            args: FunctionArguments::None,
            filter: None,
            over: None,
            null_treatment: None,
            within_group: Vec::new(),
            parameters: FunctionArguments::None,
            uses_odbc_syntax: false,
        }
    }

    fn string_literal(s: &str) -> Expr {
        Expr::Value(ValueWithSpan {
            value: Value::SingleQuotedString(s.to_string()),
            span: Span::empty(),
        })
    }

    fn int_literal() -> Expr {
        Expr::Value(ValueWithSpan {
            value: Value::Number("1".to_string(), false),
            span: Span::empty(),
        })
    }

    fn null_literal() -> Expr {
        Expr::Value(ValueWithSpan {
            value: Value::Null,
            span: Span::empty(),
        })
    }

    fn col_expr(name: &str) -> Expr {
        Expr::Identifier(Ident::new(name))
    }

    /// Regression for #117: a bare parenthesized tuple with more than one
    /// element (`(a, b)`, e.g. `VALUES` or `x IN ((1, 2), (3, 4))`) has no
    /// single neutral type. Taking the first element's type silently
    /// dropped every other field; `unknown` surfaces the gap instead.
    #[test]
    fn test_tuple_multi_element_is_unknown() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let expr = Expr::Tuple(vec![int_literal(), string_literal("a")]);
        let ti = analyzer.infer_expr_type(&expr, &scope);
        assert_eq!(ti.neutral_type, "unknown");
        assert!(ti.nullable);
    }

    /// A single-element "tuple" is just a parenthesized expression and
    /// keeps that element's real type.
    #[test]
    fn test_tuple_single_element_keeps_element_type() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let expr = Expr::Tuple(vec![int_literal()]);
        let ti = analyzer.infer_expr_type(&expr, &scope);
        assert_eq!(ti.neutral_type, "int32");
    }

    /// Regression for #117: the explicit `ROW(...)` syntax parses as a
    /// function call named "row", not `Expr::Tuple` -- it needs the same
    /// treatment through a separate code path.
    #[test]
    fn test_row_function_multi_arg_is_unknown() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("row", vec![int_literal(), string_literal("a")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "unknown");
    }

    /// A single-source scope with one column `c` of the given neutral type,
    /// for exercising aggregate-function widening rules against every
    /// numeric neutral type (columns, unlike numeric literals, carry their
    /// real neutral type instead of always resolving to `int64`).
    fn scope_with_column(neutral_type: &str) -> Scope {
        Scope {
            sources: vec![ScopeSource {
                alias: "t".to_string(),
                table_name: "t".to_string(),
                columns: vec![ScopeColumn::new("c", neutral_type, false)],
                nullable_from_join: false,
            }],
        }
    }

    /// Same as [`scope_with_column`] but the column is nullable -- for tests
    /// that need to prove narrowing does *not* fire when the source column
    /// can be NULL.
    fn scope_with_nullable_column(neutral_type: &str) -> Scope {
        Scope {
            sources: vec![ScopeSource {
                alias: "t".to_string(),
                table_name: "t".to_string(),
                columns: vec![ScopeColumn::new("c", neutral_type, true)],
                nullable_from_join: false,
            }],
        }
    }

    #[test]
    fn test_count_returns_int64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("count", vec![int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
        assert!(!ti.nullable, "count should not be nullable");
    }

    #[test]
    fn test_sum_returns_nullable() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        // `int_literal()` is now `int32` (see #122's magnitude-aware literal
        // typing), so `sum_result_type` widens it to `int64`, not `decimal`
        // -- `sum(int64)` is the case that widens to `decimal`.
        let func = make_func("sum", vec![int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
        assert!(ti.nullable, "sum (non-window) should be nullable");
    }

    #[test]
    fn test_sum_window_not_nullable() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_window_func("sum", vec![int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
        assert!(!ti.nullable, "sum as window function should not be nullable");
    }

    #[test]
    fn test_sum_result_type_int32_widens_to_int64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("int32");
        let func = make_func("sum", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
    }

    #[test]
    fn test_sum_result_type_int64_widens_to_decimal() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("int64");
        let func = make_func("sum", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
    }

    #[test]
    fn test_sum_result_type_decimal_stays_decimal() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("decimal");
        let func = make_func("sum", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
    }

    #[test]
    fn test_sum_result_type_float32_stays_float32() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("float32");
        let func = make_func("sum", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "float32");
    }

    #[test]
    fn test_sum_result_type_float64_stays_float64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("float64");
        let func = make_func("sum", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "float64");
    }

    #[test]
    fn test_avg_returns_decimal_nullable() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("avg", vec![int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
        assert!(ti.nullable);
    }

    #[test]
    fn test_avg_result_type_int32_widens_to_decimal() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("int32");
        let func = make_func("avg", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
    }

    #[test]
    fn test_avg_result_type_int64_widens_to_decimal() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("int64");
        let func = make_func("avg", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
    }

    #[test]
    fn test_avg_result_type_decimal_stays_decimal() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("decimal");
        let func = make_func("avg", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
    }

    #[test]
    fn test_avg_result_type_float32_widens_to_float64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("float32");
        let func = make_func("avg", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "float64");
    }

    #[test]
    fn test_avg_result_type_float64_stays_float64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_column("float64");
        let func = make_func("avg", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "float64");
    }

    #[test]
    fn test_string_functions_return_string() {
        let catalog = empty_catalog();
        let scope = empty_scope();
        for fname in &["upper", "lower", "initcap", "reverse", "ltrim", "rtrim", "replace"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![string_literal("hello")]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(ti.neutral_type, "string", "{} should return string", fname);
        }
    }

    #[test]
    fn test_concat_returns_non_nullable_string() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("concat", vec![string_literal("a"), string_literal("b")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "string");
        assert!(!ti.nullable, "concat should not be nullable");
    }

    #[test]
    fn test_substring_returns_string() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("substring", vec![string_literal("hello")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "string");
    }

    #[test]
    fn test_length_returns_int32() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("length", vec![string_literal("hello")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int32");
    }

    #[test]
    fn test_math_functions_abs_sign() {
        let catalog = empty_catalog();
        let scope = empty_scope();
        for fname in &["abs", "sign"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(
                ti.neutral_type, "int32",
                "{} should return int32 for int32 literal input",
                fname
            );
        }
    }

    #[test]
    fn test_math_functions_ceil_floor() {
        let catalog = empty_catalog();
        let scope = empty_scope();
        for fname in &["ceil", "ceiling", "floor"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(ti.neutral_type, "int32", "{} preserves input type", fname);
        }
    }

    #[test]
    fn test_math_functions_round() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("round", vec![int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
    }

    #[test]
    fn test_math_functions_power_sqrt() {
        let catalog = empty_catalog();
        let scope = empty_scope();
        for fname in &["power", "sqrt", "cbrt", "log", "ln", "exp", "random"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(ti.neutral_type, "float64", "{} should return float64", fname);
            assert!(!ti.nullable, "{} should not be nullable", fname);
        }
    }

    #[test]
    fn test_now_returns_datetime_tz() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_no_arg_func("now");
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "datetime_tz");
        assert!(!ti.nullable);
    }

    #[test]
    fn test_current_date_returns_date() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_no_arg_func("current_date");
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "date");
        assert!(!ti.nullable);
    }

    #[test]
    fn test_extract_function_name_form_postgresql_returns_decimal() {
        // PostgreSQL 14+ types `EXTRACT` as `numeric` -- see #123. The
        // function-call spelling gets the same dialect-aware answer as the
        // `Expr::Extract` AST node.
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("extract", vec![string_literal("year")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
    }

    #[test]
    fn test_extract_function_name_form_mysql_returns_int64() {
        let catalog = empty_catalog_with_dialect(crate::dialect::SqlDialect::MySQL);
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("extract", vec![string_literal("year")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
    }

    #[test]
    fn test_date_part_function_always_returns_float64() {
        // `date_part(text, source)` is a distinct PostgreSQL function from
        // `EXTRACT` and always returns `double precision`, on every version
        // -- the PG14+ `numeric` change applies only to `EXTRACT` (#123).
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("date_part", vec![string_literal("year"), string_literal("2024-01-01")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "float64");
    }

    #[test]
    fn test_date_trunc_with_two_args() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func(
            "date_trunc",
            vec![string_literal("month"), string_literal("2024-01-01")],
        );
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "string");
    }

    #[test]
    fn test_age_returns_interval() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_no_arg_func("age");
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "interval");
        assert!(!ti.nullable);
    }

    #[test]
    fn test_row_number_returns_int64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_no_arg_func("row_number");
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
        assert!(!ti.nullable);
    }

    #[test]
    fn test_rank_dense_rank_ntile() {
        let catalog = empty_catalog();
        let scope = empty_scope();
        for fname in &["rank", "dense_rank", "ntile", "cume_dist", "percent_rank"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_no_arg_func(fname);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(ti.neutral_type, "int64", "{} should return int64", fname);
            assert!(!ti.nullable, "{} should not be nullable", fname);
        }
    }

    #[test]
    fn test_lag_lead_nullable() {
        let catalog = empty_catalog();
        let scope = empty_scope();
        for fname in &["lag", "lead"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(ti.neutral_type, "int32", "{} should pass through input type", fname);
            assert!(ti.nullable, "{} should be nullable", fname);
        }
    }

    #[test]
    fn test_lag_lead_three_args_non_null_default_and_source_is_non_null() {
        let catalog = empty_catalog();
        for fname in &["lag", "lead"] {
            let mut analyzer = make_analyzer(&catalog);
            let scope = scope_with_column("int64");
            let func = make_window_func(fname, vec![col_expr("c"), int_literal(), int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert!(
                !ti.nullable,
                "{} with a non-null default and non-null tracked expr should not be nullable",
                fname
            );
        }
    }

    #[test]
    fn test_lag_lead_two_args_stays_nullable_even_when_source_non_null() {
        let catalog = empty_catalog();
        for fname in &["lag", "lead"] {
            let mut analyzer = make_analyzer(&catalog);
            let scope = scope_with_column("int64");
            let func = make_window_func(fname, vec![col_expr("c"), int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert!(
                ti.nullable,
                "{} without a default must stay nullable at partition boundaries",
                fname
            );
        }
    }

    #[test]
    fn test_lag_lead_three_args_nullable_source_stays_nullable() {
        let catalog = empty_catalog();
        for fname in &["lag", "lead"] {
            let mut analyzer = make_analyzer(&catalog);
            let scope = scope_with_nullable_column("int64");
            let func = make_window_func(fname, vec![col_expr("c"), int_literal(), int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert!(
                ti.nullable,
                "{} must stay nullable when the tracked expression is nullable, even with a default",
                fname
            );
        }
    }

    #[test]
    fn test_lag_lead_three_args_null_default_stays_nullable() {
        let catalog = empty_catalog();
        for fname in &["lag", "lead"] {
            let mut analyzer = make_analyzer(&catalog);
            let scope = scope_with_column("int64");
            let func = make_window_func(fname, vec![col_expr("c"), int_literal(), null_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert!(
                ti.nullable,
                "{} with an explicit NULL default should be nullable",
                fname
            );
        }
    }

    #[test]
    fn test_lag_lead_ignore_nulls_postfix_bails_out_to_nullable() {
        let catalog = empty_catalog();
        for fname in &["lag", "lead"] {
            let mut analyzer = make_analyzer(&catalog);
            let scope = scope_with_column("int64");
            let mut func = make_window_func(fname, vec![col_expr("c"), int_literal(), int_literal()]);
            func.null_treatment = Some(NullTreatment::IgnoreNulls);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert!(
                ti.nullable,
                "{} with IGNORE NULLS must stay nullable even with a non-null default",
                fname
            );
        }
    }

    #[test]
    fn test_lag_lead_ignore_nulls_argument_clause_bails_out_to_nullable() {
        let catalog = empty_catalog();
        for fname in &["lag", "lead"] {
            let mut analyzer = make_analyzer(&catalog);
            let scope = scope_with_column("int64");
            let mut func = make_window_func(fname, vec![col_expr("c"), int_literal(), int_literal()]);
            if let FunctionArguments::List(arg_list) = &mut func.args {
                arg_list
                    .clauses
                    .push(FunctionArgumentClause::IgnoreOrRespectNulls(NullTreatment::IgnoreNulls));
            }
            let ti = analyzer.infer_function_type(&func, &scope);
            assert!(
                ti.nullable,
                "{} with an in-argument-list IGNORE NULLS clause must stay nullable",
                fname
            );
        }
    }

    #[test]
    fn test_json_build_object() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_no_arg_func("json_build_object");
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "json");
        assert!(!ti.nullable);
    }

    #[test]
    fn test_gen_random_uuid() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_no_arg_func("gen_random_uuid");
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "uuid");
        assert!(!ti.nullable);
    }

    #[test]
    fn test_coalesce_with_literal_is_not_nullable() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("coalesce", vec![col_expr("x"), string_literal("default")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "string");
        assert!(!ti.nullable, "coalesce with a literal fallback should not be nullable");
    }

    #[test]
    fn coalesce_with_an_empty_string_fallback_is_nullable_on_oracle() {
        // Oracle stores `''` as NULL, so the fallback is itself NULL and
        // COALESCE really can return NULL. Inferring this non-nullable made
        // codegen emit a non-optional field that the driver then could not
        // decode -- caught by the live Oracle conformance leg as an A2
        // soundness failure, see
        // `testing_data/nullability_live/coalesce_non_null/live_coalesce_with_empty_string_default_is_null_on_oracle.json`.
        let catalog = empty_catalog_with_dialect(crate::dialect::SqlDialect::Oracle);
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("coalesce", vec![col_expr("x"), string_literal("")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert!(
            ti.nullable,
            "on Oracle an empty-string COALESCE fallback proves nothing about nullability"
        );
    }

    #[test]
    fn coalesce_with_an_empty_string_fallback_is_not_nullable_off_oracle() {
        // The counterpart the Oracle branch must not overreach into: every
        // other engine keeps `''` distinct from NULL, so the fallback does
        // guarantee non-NULL there and marking it nullable would be a
        // gratuitous `Option` in generated code for five of six engines.
        for dialect in [
            crate::dialect::SqlDialect::PostgreSQL,
            crate::dialect::SqlDialect::MySQL,
            crate::dialect::SqlDialect::SQLite,
            crate::dialect::SqlDialect::MsSql,
            crate::dialect::SqlDialect::Snowflake,
        ] {
            let catalog = empty_catalog_with_dialect(dialect);
            let mut analyzer = make_analyzer(&catalog);
            let scope = empty_scope();
            let func = make_func("coalesce", vec![col_expr("x"), string_literal("")]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert!(
                !ti.nullable,
                "{dialect:?} keeps '' distinct from NULL, so the fallback guarantees non-NULL"
            );
        }
    }

    #[test]
    fn a_non_empty_string_fallback_is_still_non_nullable_on_oracle() {
        // The Oracle branch is about the *empty* literal only: `''` is NULL
        // there, `'none'` is not. Widening it to every string literal would
        // make every COALESCE on Oracle nullable.
        let catalog = empty_catalog_with_dialect(crate::dialect::SqlDialect::Oracle);
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("coalesce", vec![col_expr("x"), string_literal("none")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert!(!ti.nullable, "a non-empty literal fallback is non-NULL on Oracle too");
    }

    #[test]
    fn test_nullif_always_nullable() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("nullif", vec![int_literal(), int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int32");
        assert!(ti.nullable, "nullif should always be nullable");
    }

    #[test]
    fn test_min_max_nullable_non_window() {
        let catalog = empty_catalog();
        let scope = empty_scope();
        for fname in &["min", "max"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![int_literal()]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(ti.neutral_type, "int32", "{} should preserve input type", fname);
            assert!(ti.nullable, "{} (non-window) should be nullable", fname);
        }
    }

    #[test]
    fn test_unknown_function() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_no_arg_func("my_custom_function");
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "__unknown_func__:my_custom_function");
    }

    #[test]
    fn test_nextval_returns_int64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("nextval", vec![string_literal("seq")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
        assert!(!ti.nullable);
    }

    fn make_func_with_arg_exprs(name: &str, args: Vec<FunctionArgExpr>) -> ast::Function {
        let mut f = make_func(name, Vec::new());
        f.args = FunctionArguments::List(FunctionArgumentList {
            args: args.into_iter().map(FunctionArg::Unnamed).collect(),
            duplicate_treatment: None,
            clauses: Vec::new(),
        });
        f
    }

    fn qualified_wildcard(qualifier: &str) -> FunctionArgExpr {
        FunctionArgExpr::QualifiedWildcard(ObjectName(vec![ObjectNamePart::Identifier(Ident::new(qualifier))]))
    }

    /// Pins `get_function_args` to still drop wildcard args entirely — the
    /// contract `get_function_arg_shapes` was added alongside, not in place
    /// of, it (see the doc comment on `get_function_arg_shapes`).
    #[test]
    fn test_get_function_args_still_drops_wildcards() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let func = make_func_with_arg_exprs("json_agg", vec![qualified_wildcard("o")]);
        assert_eq!(analyzer.get_function_args(&func), Vec::<Expr>::new());
    }

    #[test]
    fn test_get_function_arg_shapes_qualified_wildcard_resolves_relation() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = scope_with_source_alias("o", "orders");
        let func = make_func_with_arg_exprs("json_agg", vec![qualified_wildcard("o")]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Relation(alias) if alias == "o"));
    }

    /// `add_table_factor_to_scope` (`scope.rs`) always lowercases an
    /// unquoted table alias when it builds `ScopeSource.alias`, so `FROM
    /// orders O` still stores `alias: "o"`. `json_agg(O.*)`, written with the
    /// alias's original case, must resolve against that lowercased scope
    /// entry rather than degrading to `Wildcard`.
    #[test]
    fn test_get_function_arg_shapes_qualified_wildcard_uppercase_alias_resolves_relation() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = scope_with_source_alias("o", "orders");
        let func = make_func_with_arg_exprs("json_agg", vec![qualified_wildcard("O")]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Relation(alias) if alias == "o"));
    }

    /// Same as above for the bare-identifier form: `json_agg(O)` against a
    /// scope built from `FROM orders O` (alias stored lowercased as `"o"`).
    #[test]
    fn test_get_function_arg_shapes_bare_identifier_uppercase_alias_is_relation() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = scope_with_source_alias("o", "orders");
        let func = make_func_with_arg_exprs("json_agg", vec![FunctionArgExpr::Expr(col_expr("O"))]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Relation(alias) if alias == "o"));
    }

    #[test]
    fn test_get_function_arg_shapes_qualified_wildcard_unresolved_is_wildcard() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func_with_arg_exprs("json_agg", vec![qualified_wildcard("o")]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Wildcard));
    }

    #[test]
    fn test_get_function_arg_shapes_bare_wildcard() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func_with_arg_exprs("count", vec![FunctionArgExpr::Wildcard]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Wildcard));
    }

    #[test]
    fn test_get_function_arg_shapes_wildcard_with_options() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func_with_arg_exprs(
            "count",
            vec![FunctionArgExpr::WildcardWithOptions(
                WildcardAdditionalOptions::default(),
            )],
        );
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Wildcard));
    }

    /// `json_agg(o)` where `o` is a scope source alias, not a column, must
    /// resolve as `Relation("o")` rather than falling into
    /// `Expr::Identifier` (which would resolve to `__unknown_col__:o` today).
    #[test]
    fn test_get_function_arg_shapes_bare_identifier_matching_alias_is_relation() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = scope_with_source_alias("o", "orders");
        let func = make_func_with_arg_exprs("json_agg", vec![FunctionArgExpr::Expr(col_expr("o"))]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Relation(alias) if alias == "o"));
    }

    /// A bare identifier that is also a real column name must NOT be
    /// reclassified as a relation, even if some other source in scope
    /// happens to share the same alias — column identity wins.
    #[test]
    fn test_get_function_arg_shapes_bare_identifier_matching_column_stays_expr() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        // `t` is both a source alias (via scope_with_column) and, separately,
        // a column named "c" lives on it; use a name ("c") that collides with
        // the column instead of the alias to prove column identity wins.
        let mut scope = scope_with_column("string");
        scope.sources[0].alias = "c".to_string();
        scope.sources[0].table_name = "c".to_string();
        let func = make_func_with_arg_exprs("json_agg", vec![FunctionArgExpr::Expr(col_expr("c"))]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(
            matches!(&shapes[0], FuncArgShape::Expr(e) if matches!(e.as_ref(), Expr::Identifier(ident) if ident.value == "c"))
        );
    }

    #[test]
    fn test_get_function_arg_shapes_plain_expr_unaffected() {
        let catalog = empty_catalog();
        let analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func_with_arg_exprs("sum", vec![FunctionArgExpr::Expr(int_literal())]);
        let shapes = analyzer.get_function_arg_shapes(&func, &scope);
        assert_eq!(shapes.len(), 1);
        assert!(matches!(&shapes[0], FuncArgShape::Expr(e) if matches!(e.as_ref(), Expr::Value(_))));
    }

    /// `jsonb_agg` must take the same nested-aggregate path as `json_agg`:
    /// same `array<>` wrapper, same nullable column.
    #[test]
    fn test_jsonb_agg_relation_arg_infers_nested_array() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = scope_with_source_alias("o", "orders");
        let func = make_func_with_arg_exprs("jsonb_agg", vec![qualified_wildcard("o")]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "json_nested<array<__nested__0>>");
        assert!(ti.nullable, "an aggregate over zero rows is SQL NULL");
    }

    /// `to_json`/`to_jsonb` over a whole-row reference are `row_to_json`
    /// spelled differently: one nested object, no array wrapper.
    #[test]
    fn test_to_json_relation_arg_infers_nested_object() {
        let catalog = empty_catalog();
        let scope = scope_with_source_alias("o", "orders");
        for fname in ["to_json", "to_jsonb"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func_with_arg_exprs(fname, vec![qualified_wildcard("o")]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(
                ti.neutral_type, "json_nested<__nested__0>",
                "{fname} must not wrap in array<>"
            );
            assert!(ti.nullable, "{fname} of a null-extended whole-row variable is SQL NULL");
        }
    }

    /// `to_json`/`to_jsonb` are strict, so a scalar conversion is nullable
    /// exactly when its argument is.
    #[test]
    fn test_to_json_scalar_arg_follows_argument_nullability() {
        let catalog = empty_catalog();
        for fname in ["to_json", "to_jsonb"] {
            let mut analyzer = make_analyzer(&catalog);
            let nullable_scope = scope_with_nullable_column("string");
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &nullable_scope);
            assert_eq!(ti.neutral_type, "json");
            assert!(ti.nullable, "{fname} is strict, so a nullable argument yields SQL NULL");

            let mut analyzer = make_analyzer(&catalog);
            let non_null_scope = scope_with_column("string");
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &non_null_scope);
            assert!(!ti.nullable, "{fname} of a NOT NULL argument can never be SQL NULL");
        }
    }

    /// Guardrail for the deliberate non-change: `json_object_agg` builds a
    /// map keyed by its first argument's runtime values, so it has no fixed
    /// row shape to infer and stays a flat nullable `json` even when handed
    /// a relation-shaped argument.
    #[test]
    fn test_json_object_agg_relation_arg_stays_plain_json() {
        let catalog = empty_catalog();
        let scope = scope_with_source_alias("o", "orders");
        for fname in ["json_object_agg", "jsonb_object_agg"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func_with_arg_exprs(fname, vec![qualified_wildcard("o")]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(
                ti.neutral_type, "json",
                "{fname} has no fixed field set to synthesize a struct from"
            );
            assert!(ti.nullable, "{fname} over zero rows is SQL NULL");
        }
    }

    /// The `json_build_*` family is not strict — it embeds a JSON `null`
    /// rather than returning SQL NULL — so splitting it out of the `to_json`
    /// arm must leave it unconditionally non-nullable.
    #[test]
    fn test_json_build_family_stays_non_nullable() {
        let catalog = empty_catalog();
        let scope = scope_with_nullable_column("string");
        for fname in [
            "json_build_object",
            "jsonb_build_object",
            "json_build_array",
            "jsonb_build_array",
        ] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope);
            assert_eq!(ti.neutral_type, "json");
            assert!(!ti.nullable, "{fname} never returns SQL NULL for a NULL argument");
        }
    }

    /// The other half of that split: `json_strip_nulls` shared the
    /// `json_build_*` arm but is strict, so a nullable argument yields SQL
    /// NULL and the column must be nullable.
    #[test]
    fn test_json_strip_nulls_follows_argument_nullability() {
        let catalog = empty_catalog();
        for fname in ["json_strip_nulls", "jsonb_strip_nulls"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("string"));
            assert_eq!(ti.neutral_type, "json");
            assert!(ti.nullable, "{fname} is strict, so a nullable argument yields SQL NULL");

            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_column("string"));
            assert!(!ti.nullable, "{fname} of a NOT NULL argument can never be SQL NULL");
        }
    }

    /// `array_to_json` had no arm at all, so it fell through to the
    /// catch-all and became `__unknown_func__:array_to_json` -- a marker
    /// `reject_unresolved_columns` turns into an `unknown function` error,
    /// making a perfectly ordinary PostgreSQL query uncompilable. It is
    /// `proisstrict = t` and returns `json`.
    #[test]
    fn test_array_to_json_follows_argument_nullability() {
        let catalog = empty_catalog();

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func("array_to_json", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("array<int32>"));
        assert_eq!(ti.neutral_type, "json");
        assert!(
            ti.nullable,
            "array_to_json is strict, so a nullable array yields SQL NULL"
        );

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func("array_to_json", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope_with_column("array<int32>"));
        assert_eq!(ti.neutral_type, "json");
        assert!(
            !ti.nullable,
            "an empty array converts to `[]` and a NULL element to a JSON `null`, \
             so a NOT NULL array can never yield SQL NULL"
        );
    }

    /// The pretty-print flag of the two-argument
    /// `array_to_json(anyarray, boolean)` is strict too, so the arm must use
    /// `any_arg_nullable` rather than looking only at the array.
    #[test]
    fn test_array_to_json_second_argument_is_strict_too() {
        let catalog = empty_catalog();

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func("array_to_json", vec![col_expr("c"), null_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope_with_column("array<int32>"));
        assert!(ti.nullable, "a NULL pretty-print flag makes the whole call SQL NULL");

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func("array_to_json", vec![col_expr("c"), string_literal("t")]);
        let ti = analyzer.infer_function_type(&func, &scope_with_column("array<int32>"));
        assert!(!ti.nullable, "a non-NULL flag leaves the array's nullability alone");
    }

    /// `json_object`/`jsonb_object` had no arm either. Both are strict, and
    /// an empty key array yields the document `{}` while an odd-length one
    /// raises an error -- neither is a second source of SQL NULL.
    #[test]
    fn test_json_object_follows_argument_nullability() {
        let catalog = empty_catalog();
        for fname in ["json_object", "jsonb_object"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("array<string>"));
            assert_eq!(ti.neutral_type, "json");
            assert!(ti.nullable, "{fname} is strict, so a nullable array yields SQL NULL");

            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_column("array<string>"));
            assert!(!ti.nullable, "{fname} of a NOT NULL array can never be SQL NULL");
        }
    }

    /// `jsonb_set`/`jsonb_insert` are strict in *every* argument, so a
    /// nullable replacement value is enough to make the column nullable even
    /// when the document is NOT NULL.
    #[test]
    fn test_jsonb_set_is_strict_in_every_argument() {
        let catalog = empty_catalog();
        for fname in ["jsonb_set", "jsonb_insert"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(
                fname,
                vec![string_literal("{\"a\":1}"), string_literal("{a}"), col_expr("c")],
            );
            let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("json"));
            assert_eq!(ti.neutral_type, "json");
            assert!(ti.nullable, "{fname} with a nullable replacement yields SQL NULL");

            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(
                fname,
                vec![string_literal("{\"a\":1}"), string_literal("{a}"), col_expr("c")],
            );
            let ti = analyzer.infer_function_type(&func, &scope_with_column("json"));
            assert!(
                !ti.nullable,
                "{fname} with all arguments NOT NULL can never be SQL NULL"
            );
        }
    }

    /// `jsonb_set_lax` is the counter-example to reading `proisstrict`
    /// alone: it is `proisstrict = f`, yet a NULL target or path still gives
    /// SQL NULL. Only the replacement gets lenient treatment, so a nullable
    /// third argument -- the entire reason to use `_lax` -- must leave the
    /// column non-nullable.
    #[test]
    fn test_jsonb_set_lax_ignores_replacement_nullability() {
        let catalog = empty_catalog();

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func(
            "jsonb_set_lax",
            vec![string_literal("{\"a\":1}"), string_literal("{a}"), col_expr("c")],
        );
        let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("json"));
        assert_eq!(ti.neutral_type, "json");
        assert!(
            !ti.nullable,
            "a NULL replacement is embedded as a JSON null (or drops the key), never SQL NULL"
        );

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func(
            "jsonb_set_lax",
            vec![col_expr("c"), string_literal("{a}"), string_literal("1")],
        );
        let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("json"));
        assert!(ti.nullable, "a NULL target document still yields SQL NULL");
    }

    /// `json_typeof` was unconditionally nullable. It is strict and every
    /// JSON value has a type name -- the JSON `null` document reports the
    /// string `'null'`, not SQL NULL -- so nullability follows the argument.
    #[test]
    fn test_json_typeof_follows_argument_nullability() {
        let catalog = empty_catalog();
        for fname in ["json_typeof", "jsonb_typeof"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_column("json"));
            assert_eq!(ti.neutral_type, "string");
            assert!(
                !ti.nullable,
                "{fname} of a NOT NULL document always names a type, so it is never SQL NULL"
            );

            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("json"));
            assert!(ti.nullable, "{fname} is strict, so a nullable document yields SQL NULL");
        }
    }

    /// Same fix for `json_array_length`: strict, and a non-NULL argument
    /// either produces a number (`0` for `[]`) or raises -- never SQL NULL.
    #[test]
    fn test_json_array_length_follows_argument_nullability() {
        let catalog = empty_catalog();
        for fname in ["json_array_length", "jsonb_array_length"] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_column("json"));
            assert_eq!(ti.neutral_type, "int32");
            assert!(!ti.nullable, "{fname} of a NOT NULL array document is never SQL NULL");

            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("json"));
            assert!(ti.nullable, "{fname} is strict, so a nullable document yields SQL NULL");
        }
    }

    /// `jsonb_pretty` had no arm and became an `unknown function` error. It
    /// is strict and returns `text`.
    #[test]
    fn test_jsonb_pretty_returns_string_following_argument_nullability() {
        let catalog = empty_catalog();

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func("jsonb_pretty", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope_with_column("json"));
        assert_eq!(ti.neutral_type, "string", "jsonb_pretty returns text, not json");
        assert!(!ti.nullable, "the rendering of a NOT NULL document is never SQL NULL");

        let mut analyzer = make_analyzer(&catalog);
        let func = make_func("jsonb_pretty", vec![col_expr("c")]);
        let ti = analyzer.infer_function_type(&func, &scope_with_nullable_column("json"));
        assert!(
            ti.nullable,
            "jsonb_pretty is strict, so a nullable document yields SQL NULL"
        );
    }

    /// Guardrail for the deliberate non-change next door: `json_extract_path`
    /// and its `_text` variant are strict *and* unconditionally nullable,
    /// because a path that does not exist in a non-NULL document returns SQL
    /// NULL. They must not be "fixed" to follow their argument the way
    /// `json_typeof` now does.
    #[test]
    fn test_json_extract_path_stays_unconditionally_nullable() {
        let catalog = empty_catalog();
        for (fname, expected) in [
            ("json_extract_path", "json"),
            ("jsonb_extract_path", "json"),
            ("json_extract_path_text", "string"),
            ("jsonb_extract_path_text", "string"),
        ] {
            let mut analyzer = make_analyzer(&catalog);
            let func = make_func(fname, vec![col_expr("c"), string_literal("a")]);
            let ti = analyzer.infer_function_type(&func, &scope_with_column("json"));
            assert_eq!(ti.neutral_type, expected, "{fname} return type");
            assert!(
                ti.nullable,
                "{fname} returns SQL NULL for a missing path even on a NOT NULL document"
            );
        }
    }

    fn scope_with_source_alias(alias: &str, table_name: &str) -> Scope {
        Scope {
            sources: vec![ScopeSource {
                alias: alias.to_string(),
                table_name: table_name.to_string(),
                columns: vec![ScopeColumn::new("id", "int64", false)],
                nullable_from_join: false,
            }],
        }
    }
}
