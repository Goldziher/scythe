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
            },
            CompositeFieldInfo {
                name: "city".to_string(),
                neutral_type: "string".to_string(),
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
