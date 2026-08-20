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

#[test]
fn elixir_ecto_binds_composites_as_postgrex_tuples() {
    let query = AnalyzedQuery::build(|query| {
        query.name = "CreateWidget".to_string();
        query.command = QueryCommand::Exec;
        query.sql = "INSERT INTO widgets (home_address) VALUES ($1)".to_string();
        query.params = vec![AnalyzedParam {
            name: "home_address".to_string(),
            neutral_type: "composite::address".to_string(),
            nullable: true,
            position: 1,
            source_relation: None,
        }];
        query.composites = vec![address_composite()];
    });
    let backend = get_backend("elixir-ecto", "postgresql").expect("backend must exist");
    let composite = backend
        .generate_composite_def(&address_composite())
        .expect("composite generation must succeed");
    let result = generate_with_backend(&query, &*backend).expect("query generation must succeed");
    let query_fn = result.query_fn.expect("exec query must emit a function");

    assert!(
        query_fn.contains("if(is_nil(home_address), do: nil, else: Address.to_tuple(home_address))"),
        "whole-composite null must remain nil while values become tuples:\n{query_fn}"
    );
    assert!(
        composite.contains("def to_tuple(%__MODULE__{} = value)"),
        "missing tuple encoder:\n{composite}"
    );
    assert!(
        composite.contains("{value.street, value.city}"),
        "tuple encoder must preserve declared field order:\n{composite}"
    );
}
