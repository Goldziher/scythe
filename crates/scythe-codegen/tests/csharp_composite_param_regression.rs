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

#[test]
fn csharp_npgsql_encodes_nullable_composite_params_as_text() {
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
    let backend = get_backend("csharp-npgsql", "postgresql").expect("backend must exist");
    let composite = backend
        .generate_composite_def(&address_composite())
        .expect("composite generation must succeed");
    let result = generate_with_backend(&query, &*backend).expect("query generation must succeed");
    let query_fn = result.query_fn.expect("exec query must emit a function");

    assert!(
        query_fn.contains("@p1::text::address"),
        "missing explicit cast:\n{query_fn}"
    );
    assert!(
        query_fn.contains("(object?)home_address?.ToPgText() ?? DBNull.Value"),
        "whole-composite null must bind as DBNull:\n{query_fn}"
    );
    assert!(
        composite.contains("public string ToPgText()"),
        "missing encoder:\n{composite}"
    );
    assert!(
        composite.contains("EncodeCompositeField(Street), EncodeCompositeField(City)"),
        "encoder must preserve declared field order:\n{composite}"
    );
    assert!(composite.contains("f[1] is null ? null : f[1]"), "{composite}");
    assert!(
        composite.contains("f[2] is null ? null : Enum.Parse<State>(f[2], true)"),
        "{composite}"
    );
    assert!(
        composite.contains("f[3] is null ? null : DeliveryDetails.FromText(f[3])!"),
        "{composite}"
    );
    assert!(
        !composite.contains("State?.FromText"),
        "nullable spelling leaked into static call:\n{composite}"
    );
}

#[test]
fn csharp_npgsql_casts_placeholder_tokens_without_prefix_corruption() {
    let query = AnalyzedQuery::build(|query| {
        query.name = "AdversarialPlaceholders".to_string();
        query.command = QueryCommand::Exec;
        query.sql = "SELECT $1, $10, '$1', $$ $1 $10 $$".to_string();
        query.params = (1..=10)
            .map(|position| AnalyzedParam {
                name: format!("p{position}"),
                neutral_type: if position == 1 {
                    "composite::address".to_string()
                } else {
                    "string".to_string()
                },
                nullable: false,
                position,
                source_relation: None,
            })
            .collect();
        query.composites = vec![address_composite()];
    });
    let backend = get_backend("csharp-npgsql", "postgresql").expect("backend must exist");
    let query_fn = generate_with_backend(&query, &*backend)
        .expect("query generation must succeed")
        .query_fn
        .expect("exec query must emit a function");

    assert!(query_fn.contains("@p1::text::address, @p10"), "{query_fn}");
    assert!(!query_fn.contains("@p1::text::address0"), "{query_fn}");
    assert!(query_fn.contains("'$1'"), "{query_fn}");
    assert!(query_fn.contains("$$ $1 $10 $$"), "{query_fn}");
}
