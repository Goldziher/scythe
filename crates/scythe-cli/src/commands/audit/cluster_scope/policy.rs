//! Default-deny classification of PostgreSQL statements for the cluster-scope gate.
//!
//! Two layers share one vocabulary:
//!
//! - [`classify`] reads the libpg_query node type and fields of a parsed
//!   statement. Unknown statements are refused, not assumed local.
//! - [`dynamic_sql_allowed`] reads the *text* of a PL/pgSQL `EXECUTE`
//!   expression, because a dynamic statement has no parsed node until it is
//!   recovered from its static literals. It reproduces the fail-closed policy
//!   the hand-written gate enforced, including the three known-safe
//!   `format(...)` revocations emitted by the schema ACL preservation step.

use serde_json::Value;

/// A routine statement whose body must be inspected recursively.
#[derive(Debug)]
pub struct RoutineSpec {
    pub language: RoutineLanguage,
    /// The body as it appeared in the AST (`options.as` / `args.as`).
    pub body: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RoutineLanguage {
    Plpgsql,
    Sql,
    Other(String),
}

/// The classification of a single parsed statement.
#[derive(Debug)]
pub enum Verdict {
    /// A database-local statement that is safe to apply.
    Local,
    /// A statement that writes cluster-global shared state.
    ClusterScope { kind: &'static str },
    /// A statement whose kind is not on the database-local allow-list.
    NotAllowlisted,
    /// A routine (`DO` / `CREATE FUNCTION` / `CREATE PROCEDURE`) that is
    /// allowed but whose body must be inspected for cluster-scope writes.
    Routine(RoutineSpec),
}

/// Classify one parsed statement by its node type and fields.
pub fn classify(node_type: &str, value: &Value) -> Verdict {
    if let Some(kind) = cluster_scope_kind(node_type, value) {
        return Verdict::ClusterScope { kind };
    }
    if let Some(routine) = routine_spec(node_type, value) {
        return Verdict::Routine(routine);
    }
    if is_local_allowlisted(node_type, value) {
        Verdict::Local
    } else {
        Verdict::NotAllowlisted
    }
}

/// Cluster-global statement kinds, keyed by PostgreSQL node type and the
/// discriminating field where one node type is used for local and cluster
/// targets (`GRANT`, `COMMENT`, `SECURITY LABEL`, `ALTER TABLE`).
fn cluster_scope_kind(node_type: &str, value: &Value) -> Option<&'static str> {
    match node_type {
        "CreateRoleStmt" | "AlterRoleStmt" | "AlterRoleSetStmt" | "DropRoleStmt" => Some("role"),
        "GrantRoleStmt" => Some("role membership"),
        "CreatedbStmt"
        | "AlterDatabaseStmt"
        | "AlterDatabaseSetStmt"
        | "AlterDatabaseRefreshCollStmt"
        | "DropdbStmt" => Some("database"),
        "CreateTableSpaceStmt"
        | "AlterTableSpaceStmt"
        | "AlterTableSpaceOptionsStmt"
        | "DropTableSpaceStmt"
        | "AlterTableMoveAllStmt" => Some("tablespace"),
        "CreateSubscriptionStmt" | "AlterSubscriptionStmt" | "DropSubscriptionStmt" => Some("subscription"),
        "AlterSystemStmt" => Some("server configuration"),
        "DropOwnedStmt" | "ReassignOwnedStmt" | "AlterOwnerStmt" => Some("shared ownership"),
        "GrantStmt" => match field(value, "objtype") {
            Some("OBJECT_DATABASE") => Some("database"),
            Some("OBJECT_TABLESPACE") => Some("tablespace"),
            Some("OBJECT_PARAMETER_ACL") => Some("parameter privilege"),
            _ => None,
        },
        "CommentStmt" | "SecLabelStmt" => match field(value, "objtype") {
            Some("OBJECT_ROLE") => Some("role"),
            Some("OBJECT_DATABASE") => Some("database"),
            Some("OBJECT_TABLESPACE") => Some("tablespace"),
            Some("OBJECT_SUBSCRIPTION") => Some("subscription"),
            _ => None,
        },
        "RenameStmt" => match field(value, "renameType") {
            Some("OBJECT_ROLE") => Some("role"),
            Some("OBJECT_DATABASE") => Some("database"),
            Some("OBJECT_TABLESPACE") => Some("tablespace"),
            Some("OBJECT_SUBSCRIPTION") => Some("subscription"),
            _ => None,
        },
        "AlterTableStmt" => value
            .get("cmds")
            .and_then(Value::as_array)
            .is_some_and(|cmds| {
                cmds.iter().any(|cmd| {
                    cmd.get("AlterTableCmd")
                        .and_then(|c| c.get("subtype"))
                        .and_then(Value::as_str)
                        == Some("AT_ChangeOwner")
                })
            })
            .then_some("shared ownership"),
        _ => None,
    }
}

/// Extract a routine's language and body, if the node is a routine.
fn routine_spec(node_type: &str, value: &Value) -> Option<RoutineSpec> {
    match node_type {
        "DoStmt" => {
            let args = value.get("args").and_then(Value::as_array)?;
            let language = def_elem(args, "language")
                .and_then(|arg| arg.get("String"))
                .and_then(|s| s.get("sval"))
                .and_then(Value::as_str)
                .unwrap_or("plpgsql");
            let body = def_elem(args, "as")
                .and_then(|arg| arg.get("String"))
                .and_then(|s| s.get("sval"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(RoutineSpec {
                language: routine_language(language),
                body,
            })
        }
        "CreateFunctionStmt" => {
            let options = value.get("options").and_then(Value::as_array)?;
            let language = def_elem(options, "language")
                .and_then(|arg| arg.get("String"))
                .and_then(|s| s.get("sval"))
                .and_then(Value::as_str);
            let body = def_elem(options, "as").map(join_body).unwrap_or_default();
            // ~keep A function with no LANGUAGE is not inspectable: PostgreSQL
            // ~keep requires it, so a missing one means we cannot prove local.
            Some(RoutineSpec {
                language: routine_language(language.unwrap_or("")),
                body,
            })
        }
        _ => None,
    }
}

fn routine_language(raw: &str) -> RoutineLanguage {
    match raw {
        "plpgsql" => RoutineLanguage::Plpgsql,
        "sql" => RoutineLanguage::Sql,
        other => RoutineLanguage::Other(other.to_string()),
    }
}

/// `options.as` / `args.as` is a `List` of `String` fragments to concatenate.
fn join_body(arg: &Value) -> String {
    arg.get("List")
        .and_then(|list| list.get("items"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("String").and_then(|s| s.get("sval")).and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn def_elem<'a>(list: &'a [Value], name: &str) -> Option<&'a Value> {
    list.iter().find_map(|entry| {
        let def = entry.get("DefElem")?;
        (def.get("defname").and_then(Value::as_str) == Some(name)).then(|| def.get("arg").unwrap_or(&Value::Null))
    })
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

const LOCAL_OBJECT_TYPES: &[&str] = &[
    "OBJECT_TABLE",
    "OBJECT_INDEX",
    "OBJECT_SEQUENCE",
    "OBJECT_VIEW",
    "OBJECT_MATVIEW",
    "OBJECT_FOREIGN_TABLE",
    "OBJECT_FUNCTION",
    "OBJECT_PROCEDURE",
    "OBJECT_ROUTINE",
    "OBJECT_TYPE",
    "OBJECT_DOMAIN",
    "OBJECT_POLICY",
    "OBJECT_TRIGGER",
    "OBJECT_COLUMN",
    "OBJECT_CONSTRAINT",
    "OBJECT_SCHEMA",
];

fn is_local_allowlisted(node_type: &str, value: &Value) -> bool {
    match node_type {
        // ~keep The allow-list is default-deny: a new node type is refused until it
        // ~keep is proven database-local here.
        "CreateStmt"
        | "IndexStmt"
        | "ViewStmt"
        | "CreateSeqStmt"
        | "CreateEnumStmt"
        | "CompositeTypeStmt"
        | "CreateDomainStmt"
        | "CreatePolicyStmt"
        | "CreateTrigStmt"
        | "CreateEventTrigStmt"
        | "CreateUserMappingStmt"
        | "AlterUserMappingStmt"
        | "AlterSeqStmt"
        | "AlterFunctionStmt"
        | "AlterPolicyStmt"
        | "CreateSchemaStmt" => true,
        "CreateTableAsStmt" => matches!(field(value, "objtype"), Some("OBJECT_TABLE") | Some("OBJECT_MATVIEW")),
        "AlterTableStmt" => matches!(
            field(value, "objtype"),
            Some(
                "OBJECT_TABLE"
                    | "OBJECT_INDEX"
                    | "OBJECT_SEQUENCE"
                    | "OBJECT_VIEW"
                    | "OBJECT_MATVIEW"
                    | "OBJECT_FOREIGN_TABLE"
            )
        ),
        "RenameStmt" => matches!(field(value, "renameType"), Some(t) if LOCAL_OBJECT_TYPES.contains(&t)),
        "CommentStmt" => matches!(field(value, "objtype"), Some(t) if LOCAL_OBJECT_TYPES.contains(&t)),
        "GrantStmt" => matches!(field(value, "objtype"), Some(t) if LOCAL_OBJECT_TYPES.contains(&t)),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Dynamic SQL policy (PL/pgSQL `EXECUTE`).
// ---------------------------------------------------------------------------

/// Decide whether a PL/pgSQL `EXECUTE` expression can be proven database-local.
///
/// Returns `Ok(())` when the dynamic statement is statically recoverable and
/// local, and `Err(reason)` when it must fail closed.
pub fn dynamic_sql_allowed(expression: &str) -> Result<(), String> {
    let raw_upper = normalize_upper(expression);
    let raw_upper = raw_upper.trim_end_matches(';').trim().to_string();
    let upper = static_literals(expression).to_uppercase();

    if upper.contains(';') {
        return Err("dynamic SQL concatenates multiple statements".to_string());
    }
    if is_cluster_scoped_text(&upper) {
        return Err("dynamic SQL reconstructs a cluster-scoped statement".to_string());
    }
    if raw_upper.contains("||") || !is_single_format_call(&raw_upper) {
        return Err("dynamic SQL is not a single statically recoverable format() call".to_string());
    }
    if has_unsupported_placeholder(&upper) {
        return Err("dynamic SQL uses a placeholder the gate cannot prove safe".to_string());
    }
    if upper.contains("%S") && !is_safe_percent_s_expression(&raw_upper) {
        return Err("dynamic SQL uses %S outside the known-safe revocations".to_string());
    }
    if is_allowed_recovered_statement(&upper) {
        Ok(())
    } else {
        Err("dynamic SQL reconstructs a statement kind the gate does not allow".to_string())
    }
}

fn normalize_upper(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase()
}

/// Concatenate the contents of every single-quoted literal, exactly as the
/// hand-written gate did, so a statement assembled across tokens is visible.
fn static_literals(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut result = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\'' {
            i += 1;
            continue;
        }
        i += 1;
        while i < bytes.len() {
            if bytes[i] == b'\'' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                result.push('\'');
                i += 2;
            } else if bytes[i] == b'\'' {
                i += 1;
                break;
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }
        result.push(' ');
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_single_format_call(raw_upper: &str) -> bool {
    let stripped = raw_upper
        .strip_prefix("PG_CATALOG.FORMAT(")
        .or_else(|| raw_upper.strip_prefix("FORMAT("));
    match stripped {
        Some(rest) => rest.trim_start().starts_with('\'') && raw_upper.ends_with(')'),
        None => false,
    }
}

fn has_unsupported_placeholder(upper: &str) -> bool {
    let bytes = upper.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            match bytes.get(i + 1) {
                Some(b'I' | b'L' | b'S' | b'%') => i += 2,
                _ => return true,
            }
        } else {
            i += 1;
        }
    }
    false
}

fn is_safe_percent_s_expression(raw_upper: &str) -> bool {
    let compact: String = raw_upper.split_whitespace().collect();
    // ~keep The three revocations the schema ACL preservation step emits. Any other
    // ~keep %S would let a runtime value name the target object.
    const SAFE: &[&str] = &[
        "FORMAT('REVOKEEXECUTEONFUNCTION%SFROMPUBLIC',TARGET.OID::REGPROCEDURE)",
        "FORMAT('REVOKEUSAGEONTYPE%SFROMPUBLIC',TARGET.OID::REGTYPE)",
        "PG_CATALOG.FORMAT('REVOKE%s(%I)ONTABLE%I.%IFROM%s',V_COLUMN_GRANT.PRIVILEGE_TYPE,V_COLUMN_GRANT.ATTNAME,'PUBLIC',P_PARTITION_NAME,CASEWHENV_COLUMN_GRANT.GRANTEE=0THEN'PUBLIC'ELSEPG_CATALOG.QUOTE_IDENT(V_COLUMN_GRANT.ROLNAME)END)",
    ];
    SAFE.iter().any(|candidate| candidate.eq_ignore_ascii_case(&compact))
}

fn is_allowed_recovered_statement(upper: &str) -> bool {
    (words(upper).first() == Some(&"REVOKE")
        && first_word_after(upper, "ON").is_some_and(|t| matches!(t, "TABLE" | "FUNCTION" | "TYPE")))
        || starts_with_word(upper, "ALTER", "TABLE")
        || starts_with_word(upper, "DROP", "POLICY")
        || starts_with_word(upper, "CREATE", "POLICY")
        || starts_with_word(upper, "CREATE", "TABLE")
        || starts_with_word(upper, "LOCK", "TABLE")
        || starts_with_word(upper, "DROP", "TABLE")
}

/// Textual cluster-scope detection for a reconstructed dynamic statement.
fn is_cluster_scoped_text(upper: &str) -> bool {
    let stripped = strip_user_mapping(upper);
    for kind in ["ROLE", "USER", "GROUP", "DATABASE", "TABLESPACE", "SUBSCRIPTION"] {
        if pair(&stripped, "CREATE", kind) || pair(&stripped, "ALTER", kind) || pair(&stripped, "DROP", kind) {
            return true;
        }
    }
    pair(&stripped, "ALTER", "SYSTEM")
        || shared_target(&stripped, "DATABASE")
        || shared_target(&stripped, "TABLESPACE")
        || shared_target(&stripped, "PARAMETER")
        || shared_target(&stripped, "ROLE")
        || shared_target(&stripped, "SUBSCRIPTION")
        || pair(&stripped, "OWNER", "TO")
        || pair(&stripped, "DROP", "OWNED")
        || pair(&stripped, "REASSIGN", "OWNED")
        || membership(&stripped, "GRANT", "TO")
        || membership(&stripped, "REVOKE", "FROM")
}

fn strip_user_mapping(upper: &str) -> String {
    // ~keep `CREATE USER MAPPING ... FOR` must not read as `CREATE USER`.
    let mut text = upper.to_string();
    for flat in [
        "USER MAPPING IF NOT EXISTS FOR",
        "USER MAPPING IF EXISTS FOR",
        "USER MAPPING FOR",
    ] {
        text = text.replace(flat, "USER_MAPPING_FOR");
    }
    text
}

fn words(upper: &str) -> Vec<&str> {
    upper
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

fn pair(upper: &str, first: &str, second: &str) -> bool {
    let tokens = words(upper);
    tokens.windows(2).any(|w| w[0] == first && w[1] == second)
}

fn shared_target(upper: &str, kind: &str) -> bool {
    // ~keep `ON <kind>` must not consume a schema-qualified name (`ON DATABASE.x`).
    let tokens = words(upper);
    tokens.windows(2).any(|w| w[0] == "ON" && w[1] == kind)
}

fn first_word_after<'a>(upper: &'a str, word: &str) -> Option<&'a str> {
    let tokens = words(upper);
    tokens
        .iter()
        .position(|t| *t == word)
        .and_then(|i| tokens.get(i + 1).copied())
}

fn starts_with_word(upper: &str, first: &str, second: &str) -> bool {
    let tokens = words(upper);
    tokens.first() == Some(&first) && tokens.get(1) == Some(&second)
}

fn membership(upper: &str, action: &str, recipient: &str) -> bool {
    let tokens = words(upper);
    let Some(action_at) = tokens.iter().position(|t| *t == action) else {
        return false;
    };
    let Some(recipient_rel) = tokens[action_at..].iter().position(|t| *t == recipient) else {
        return false;
    };
    let between = &tokens[action_at..action_at + recipient_rel];
    !between.contains(&"ON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(kind: &str, fields: Value) -> (String, Value) {
        (kind.to_string(), fields)
    }

    #[test]
    fn role_and_database_statements_are_cluster_scope() {
        for (kind, fields) in [
            node("CreateRoleStmt", json!({})),
            node("GrantRoleStmt", json!({})),
            node("CreatedbStmt", json!({})),
            node("AlterSystemStmt", json!({})),
            node("DropOwnedStmt", json!({})),
        ] {
            assert!(
                matches!(classify(&kind, &fields), Verdict::ClusterScope { .. }),
                "{kind} must be cluster-scope"
            );
        }
    }

    #[test]
    fn grant_target_discriminates_local_from_cluster() {
        assert!(matches!(
            classify("GrantStmt", &json!({"objtype": "OBJECT_TABLE"})),
            Verdict::Local
        ));
        assert!(matches!(
            classify("GrantStmt", &json!({"objtype": "OBJECT_DATABASE"})),
            Verdict::ClusterScope { .. }
        ));
        assert!(matches!(
            classify("GrantStmt", &json!({"objtype": "OBJECT_PARAMETER_ACL"})),
            Verdict::ClusterScope { .. }
        ));
    }

    #[test]
    fn alter_table_owner_is_cluster_scope_but_rls_is_local() {
        assert!(matches!(
            classify(
                "AlterTableStmt",
                &json!({"objtype": "OBJECT_TABLE", "cmds": [{"AlterTableCmd": {"subtype": "AT_ChangeOwner"}}]})
            ),
            Verdict::ClusterScope { .. }
        ));
        assert!(matches!(
            classify(
                "AlterTableStmt",
                &json!({"objtype": "OBJECT_TABLE", "cmds": [{"AlterTableCmd": {"subtype": "AT_EnableRowSecurity"}}]})
            ),
            Verdict::Local
        ));
    }

    #[test]
    fn unknown_statement_kinds_are_not_allowlisted() {
        assert!(matches!(classify("SelectStmt", &json!({})), Verdict::NotAllowlisted));
        assert!(matches!(classify("CallStmt", &json!({})), Verdict::NotAllowlisted));
    }

    #[test]
    fn routine_carries_language_and_body() {
        let value = json!({"options": [
            {"DefElem": {"defname": "language", "arg": {"String": {"sval": "plpgsql"}}}},
            {"DefElem": {"defname": "as", "arg": {"List": {"items": [{"String": {"sval": " BEGIN NULL; END "}}]}}}}
        ]});
        match classify("CreateFunctionStmt", &value) {
            Verdict::Routine(spec) => {
                assert_eq!(spec.language, RoutineLanguage::Plpgsql);
                assert_eq!(spec.body, " BEGIN NULL; END ");
            }
            other => panic!("expected routine, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_static_cluster_statement_is_refused() {
        assert!(dynamic_sql_allowed("'CREATE ROLE planted_role'").is_err());
        assert!(dynamic_sql_allowed("format('GRANT ALL ON DATABASE %I TO %I', 'a', 'b')").is_err());
    }

    #[test]
    fn dynamic_concatenation_and_variables_are_refused() {
        assert!(dynamic_sql_allowed("'CREATE ' || 'ROLE x'").is_err());
        assert!(dynamic_sql_allowed("statement").is_err());
        assert!(dynamic_sql_allowed("format('CREATE TABLE safe_%s(id int)', planted)").is_err());
    }

    #[test]
    fn dynamic_known_safe_revoke_is_allowed() {
        assert!(
            dynamic_sql_allowed("format('REVOKE EXECUTE ON FUNCTION %s FROM PUBLIC', target.oid::regprocedure)")
                .is_ok()
        );
        assert!(dynamic_sql_allowed("format('REVOKE USAGE ON TYPE %s FROM PUBLIC', target.oid::regtype)").is_ok());
    }

    #[test]
    fn bare_literal_dynamic_sql_is_refused_even_when_local() {
        // ~keep The gate only proves a statement local by reconstructing a single
        // ~keep format() call; a bare literal EXECUTE stays unproven and is refused.
        assert!(dynamic_sql_allowed("'CREATE TABLE safe(id int)'").is_err());
    }
}
