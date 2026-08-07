use sqlparser::ast::{self, BinaryOperator, Expr, FunctionArg, FunctionArgExpr, UnaryOperator};

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
                if value_is_number(vws) {
                    TypeInfo::new("int64", false)
                } else if value_is_string(vws) {
                    TypeInfo::new("string", false)
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
                expr: inner, data_type, ..
            } => {
                let inner_ti = self.infer_expr_type(inner, scope);
                let neutral = datatype_to_neutral(data_type, self.catalog);
                self.collect_param_type_from_cast(inner, &neutral);
                TypeInfo::new(neutral, inner_ti.nullable)
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
                        let result_type = if left_ti.neutral_type == "unknown" {
                            right_ti.neutral_type.clone()
                        } else {
                            left_ti.neutral_type.clone()
                        };
                        TypeInfo::new(result_type, left_ti.nullable || right_ti.nullable)
                    }
                    BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
                    | BinaryOperator::And
                    | BinaryOperator::Or => TypeInfo::new("bool", false),
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
                        && let Some(pos) = self.resolve_placeholder_position(p)
                    {
                        let col_name = expr_to_name(col_expr);
                        self.register_param(pos, Some(col_name), Some(col_ti.neutral_type.clone()), false);
                    }
                }
                TypeInfo::new("bool", false)
            }

            Expr::InSubquery { .. } => TypeInfo::new("bool", false),

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
                TypeInfo::new("bool", false)
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
                let _col_ti = self.infer_expr_type(col_expr, scope);
                self.collect_param_from_expr_with_type(pattern, "string", &expr_to_name(col_expr));
                TypeInfo::new("bool", false)
            }

            Expr::Case {
                operand: _,
                conditions,
                else_result,
                ..
            } => {
                let mut result_type = "unknown".to_string();
                let mut any_nullable = false;

                for case_when in conditions {
                    let _ = self.infer_expr_type(&case_when.condition, scope);
                    if let Expr::Value(vws) = &case_when.condition
                        && let Some(p) = value_is_placeholder(vws)
                        && let Some(pos) = self.resolve_placeholder_position(p)
                    {
                        self.register_param(pos, Some("flag".to_string()), Some("bool".to_string()), false);
                    }

                    let ti = self.infer_expr_type(&case_when.result, scope);
                    if result_type == "unknown" && ti.neutral_type != "unknown" {
                        result_type = ti.neutral_type.clone();
                    }
                    let guarded = is_not_null_guard(&case_when.condition, &case_when.result);
                    if ti.nullable && !guarded {
                        any_nullable = true;
                    }
                }

                if let Some(else_expr) = else_result {
                    let ti = self.infer_expr_type(else_expr, scope);
                    if result_type == "unknown" && ti.neutral_type != "unknown" {
                        result_type = ti.neutral_type.clone();
                    }
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
                    && let Some(pos) = self.resolve_placeholder_position(p)
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
                    && let Some(pos) = self.resolve_placeholder_position(p)
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
                if let Some(first) = exprs.first() {
                    self.infer_expr_type(first, scope)
                } else {
                    TypeInfo::unknown()
                }
            }

            Expr::Extract { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("float64", ti.nullable)
            }

            Expr::Substring { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("string", ti.nullable)
            }

            Expr::Trim { expr, .. } => {
                let ti = self.infer_expr_type(expr, scope);
                TypeInfo::new("string", ti.nullable)
            }

            Expr::Position { .. } => TypeInfo::new("int32", false),

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
                        return TypeInfo::new(format!("__ambiguous__:{}", col_name), false);
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
            return TypeInfo::new(format!("__unknown_col__:{}", col_name), true);
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
            "json_agg" | "jsonb_agg" | "json_object_agg" | "jsonb_object_agg" => TypeInfo::new("json", true),

            "coalesce" => {
                let args = self.get_function_args(func);
                let mut result_type = "unknown".to_string();
                let mut any_non_nullable = false;
                let mut coalesce_name: Option<String> = None;

                for arg in &args {
                    let ti = self.infer_expr_type(arg, scope);
                    if result_type == "unknown" && ti.neutral_type != "unknown" {
                        result_type = ti.neutral_type.clone();
                    }
                    if !ti.nullable || is_literal(arg) {
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
                        && let Some(pos) = self.resolve_placeholder_position(p)
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

            "upper" | "lower" | "initcap" | "reverse" | "ltrim" | "rtrim" | "btrim" | "lpad" | "rpad" | "repeat"
            | "replace" | "translate" | "left" | "right" | "md5" | "encode" | "decode" | "chr" | "to_hex"
            | "quote_ident" | "quote_literal" | "format" | "regexp_replace" => {
                TypeInfo::new("string", first_arg_nullable)
            }
            "concat" | "concat_ws" => TypeInfo::new("string", false),
            "substring" | "substr" => TypeInfo::new("string", first_arg_nullable),
            "length" | "char_length" | "character_length" | "octet_length" | "bit_length" | "strpos" => {
                TypeInfo::new("int32", first_arg_nullable)
            }

            "abs" | "sign" => first_arg_ti.unwrap_or_else(TypeInfo::unknown),
            "ceil" | "ceiling" | "floor" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, ti.nullable)
            }
            "round" | "trunc" => TypeInfo::new("decimal", first_arg_nullable),
            "power" | "sqrt" | "cbrt" | "log" | "ln" | "exp" | "pi" | "sin" | "cos" | "tan" | "asin" | "acos"
            | "atan" | "atan2" | "degrees" | "radians" | "random" => TypeInfo::new("float64", false),
            "mod" => first_arg_ti.unwrap_or_else(|| TypeInfo::new("int32", false)),
            "div" => TypeInfo::new("int64", first_arg_nullable),
            "greatest" | "least" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, ti.nullable)
            }

            "now" | "current_timestamp" | "statement_timestamp" | "transaction_timestamp" | "clock_timestamp" => {
                TypeInfo::new("datetime_tz", false)
            }
            "current_date" | "localdate" | "date" => TypeInfo::new("date", false),
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
            "date_part" | "extract" => TypeInfo::new("float64", first_arg_nullable),
            "age" => TypeInfo::new("interval", false),
            "make_date" => TypeInfo::new("date", false),
            "make_time" => TypeInfo::new("time", false),
            "make_timestamp" => TypeInfo::new("datetime", false),
            "make_timestamptz" => TypeInfo::new("datetime_tz", false),
            "make_interval" => TypeInfo::new("interval", false),
            "to_timestamp" => TypeInfo::new("datetime_tz", false),
            "to_date" => TypeInfo::new("date", false),
            "to_char" => TypeInfo::new("string", first_arg_nullable),

            "row_number" | "rank" | "dense_rank" | "cume_dist" | "ntile" | "percent_rank" => {
                TypeInfo::new("int64", false)
            }
            "lag" | "lead" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, true)
            }
            "first_value" | "last_value" | "nth_value" => {
                let ti = first_arg_ti.unwrap_or_else(TypeInfo::unknown);
                TypeInfo::new(ti.neutral_type, true)
            }

            "json_build_object" | "jsonb_build_object" | "json_build_array" | "jsonb_build_array" | "to_json"
            | "to_jsonb" | "json_strip_nulls" | "jsonb_strip_nulls" => TypeInfo::new("json", false),
            "json_typeof" | "jsonb_typeof" => TypeInfo::new("string", true),
            "json_extract_path_text" | "jsonb_extract_path_text" => TypeInfo::new("string", true),
            "json_extract_path" | "jsonb_extract_path" => TypeInfo::new("json", true),
            "json_array_length" | "jsonb_array_length" => TypeInfo::new("int32", true),
            "json_each" | "jsonb_each" | "json_each_text" | "jsonb_each_text" => TypeInfo::new("string", true),
            "json_object_keys" | "jsonb_object_keys" => TypeInfo::new("string", false),
            "json_populate_record"
            | "jsonb_populate_record"
            | "json_populate_recordset"
            | "jsonb_populate_recordset" => TypeInfo::new("unknown", true),

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
                TypeInfo::new(format!("__unknown_func__:{}", func_name), ti.nullable)
            }
        }
    }

    pub(super) fn get_first_arg_type(&mut self, func: &ast::Function, scope: &Scope) -> Option<TypeInfo> {
        let args = self.get_function_args(func);
        args.first().map(|arg| self.infer_expr_type(arg, scope))
    }

    pub(super) fn get_function_args(&self, func: &ast::Function) -> Vec<Expr> {
        match &func.args {
            ast::FunctionArguments::None => Vec::new(),
            ast::FunctionArguments::Subquery(_) => Vec::new(),
            ast::FunctionArguments::List(arg_list) => arg_list
                .args
                .iter()
                .filter_map(|arg| match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Some(e.clone()),
                    FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(e),
                        ..
                    } => Some(e.clone()),
                    _ => None,
                })
                .collect(),
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
    /// Unused outside tests until the phase-3 nested-aggregate arms of
    /// `infer_function_type` are added — this commit only introduces the
    /// shape-classification contract and pins it with tests.
    #[allow(dead_code)]
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
}

/// Shape of a single function argument, widened from sqlparser's
/// `FunctionArgExpr` to preserve wildcard and relation-reference forms that
/// [`Analyzer::get_function_args`] silently drops.
///
/// `#[allow(dead_code)]`: fields are read from tests only until the phase-3
/// nested-aggregate arms of `infer_function_type` consume them.
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
        Function, FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments, Ident, ObjectName,
        ObjectNamePart, Value, ValueWithSpan, WildcardAdditionalOptions, WindowFrame, WindowFrameBound,
        WindowFrameUnits, WindowSpec, WindowType,
    };
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

    fn col_expr(name: &str) -> Expr {
        Expr::Identifier(Ident::new(name))
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
        let func = make_func("sum", vec![int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
        assert!(ti.nullable, "sum (non-window) should be nullable");
    }

    #[test]
    fn test_sum_window_not_nullable() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_window_func("sum", vec![int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "decimal");
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
            assert_eq!(ti.neutral_type, "int64", "{} should return int64 for int input", fname);
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
            assert_eq!(ti.neutral_type, "int64", "{} preserves input type", fname);
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
    fn test_extract_returns_float64() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("extract", vec![string_literal("year")]);
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
            assert_eq!(ti.neutral_type, "int64", "{} should pass through input type", fname);
            assert!(ti.nullable, "{} should be nullable", fname);
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
    fn test_nullif_always_nullable() {
        let catalog = empty_catalog();
        let mut analyzer = make_analyzer(&catalog);
        let scope = empty_scope();
        let func = make_func("nullif", vec![int_literal(), int_literal()]);
        let ti = analyzer.infer_function_type(&func, &scope);
        assert_eq!(ti.neutral_type, "int64");
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
            assert_eq!(ti.neutral_type, "int64", "{} should preserve input type", fname);
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
