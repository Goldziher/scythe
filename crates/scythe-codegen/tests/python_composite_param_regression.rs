use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo};
use scythe_core::parser::QueryCommand;

fn address_composite() -> CompositeInfo {
    CompositeInfo {
        sql_name: "address".to_string(),
        fields: vec![
            CompositeFieldInfo {
                name: "street".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            },
            CompositeFieldInfo {
                name: "city".to_string(),
                neutral_type: "string".to_string(),
                nullable: true,
            },
            CompositeFieldInfo {
                name: "state".to_string(),
                neutral_type: "enum::state".to_string(),
                nullable: true,
            },
            CompositeFieldInfo {
                name: "delivery".to_string(),
                neutral_type: "composite::delivery_details".to_string(),
                nullable: true,
            },
        ],
    }
}

fn composite_query(command: QueryCommand) -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
        query.name = "CreateWidget".to_string();
        query.command = command;
        query.sql = "INSERT INTO widgets (home_address) VALUES ($1)".to_string();
        query.params = vec![AnalyzedParam {
            name: "home_address".to_string(),
            neutral_type: "composite::address".to_string(),
            nullable: true,
            position: 1,
            source_relation: None,
        }];
        query.composites = vec![address_composite()];
    })
}

#[test]
fn python_psycopg3_encodes_composite_params_and_casts_from_text() {
    let backend = get_backend("python-psycopg3", "postgresql").expect("backend must exist");
    let composite = backend
        .generate_composite_def(&address_composite())
        .expect("composite generation must succeed");
    let result =
        generate_with_backend(&composite_query(QueryCommand::Exec), &*backend).expect("query generation must succeed");
    let query_fn = result.query_fn.expect("exec query must emit a function");

    assert!(
        query_fn.contains("%(home_address)s::text::address"),
        "missing explicit text-to-composite cast:\n{query_fn}"
    );
    assert!(
        query_fn.contains("None if home_address is None else home_address._to_pg_text()"),
        "whole-composite null must remain None while values are encoded:\n{query_fn}"
    );
    assert!(
        composite.contains("def _to_pg_text(self) -> str:"),
        "missing encoder:\n{composite}"
    );
    assert!(
        composite.contains("raw.replace(\"\\\\\", \"\\\\\\\\\").replace('\\\"', '\\\"\\\"')"),
        "encoder must escape backslashes and quotes:\n{composite}"
    );
    assert!(composite.contains("city: str | None"), "{composite}");
    assert!(
        composite.contains("city=None if f[1] is None else cls._require_composite_field(f[1], \"city\")"),
        "{composite}"
    );
    assert!(
        composite.contains("state=None if f[2] is None else State(f[2])"),
        "{composite}"
    );
    assert!(
        composite.contains("delivery=None if f[3] is None else DeliveryDetails._from_text(f[3])"),
        "{composite}"
    );
    assert!(
        !composite.contains("State | None("),
        "nullable spelling leaked into constructor:\n{composite}"
    );
}

#[test]
fn python_psycopg3_encodes_single_composite_batch_items() {
    let backend = get_backend("python-psycopg3", "postgresql").expect("backend must exist");
    let result =
        generate_with_backend(&composite_query(QueryCommand::Batch), &*backend).expect("query generation must succeed");
    let query_fn = result.query_fn.expect("batch query must emit a function");

    assert!(
        query_fn.contains("None if item is None else item._to_pg_text()"),
        "batch items must use the same null-safe encoder:\n{query_fn}"
    );
}

#[test]
fn python_asyncpg_binds_composites_as_native_records() {
    let backend = get_backend("python-asyncpg", "postgresql").expect("backend must exist");
    let composite = backend
        .generate_composite_def(&address_composite())
        .expect("composite generation must succeed");
    let result =
        generate_with_backend(&composite_query(QueryCommand::Exec), &*backend).expect("query generation must succeed");
    let query_fn = result.query_fn.expect("exec query must emit a function");

    assert!(
        query_fn.contains("None if home_address is None else home_address._to_record()"),
        "asyncpg must receive its native positional composite shape:\n{query_fn}"
    );
    assert!(
        composite.contains("def _to_record(self) -> tuple[Any, ...]:"),
        "missing native-record conversion:\n{composite}"
    );
    assert!(
        composite.contains(
            "return (self.street, self.city, self.state, None if self.delivery is None else self.delivery._to_record())"
        ),
        "record conversion must preserve declared field order:\n{composite}"
    );
}
