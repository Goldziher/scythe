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
fn go_pgx_encodes_nullable_composite_params_as_text() {
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
    let backend = get_backend("go-pgx", "postgresql").expect("backend must exist");
    let composite = backend
        .generate_composite_def(&address_composite())
        .expect("composite generation must succeed");
    let result = generate_with_backend(&query, &*backend).expect("query generation must succeed");
    let query_fn = result.query_fn.expect("exec query must emit a function");

    assert!(
        query_fn.contains("$1::text::address"),
        "missing explicit cast:\n{query_fn}"
    );
    assert!(
        query_fn.contains("if HomeAddress == nil { return nil }; text := HomeAddress.ToPgText()"),
        "whole-composite null must bind as nil:\n{query_fn}"
    );
    assert!(
        composite.contains("func (v Address) ToPgText() string"),
        "missing encoder:\n{composite}"
    );
    assert!(
        composite.contains("encodeCompositeField(v.Street), encodeCompositeFieldPtr(v.City)"),
        "encoder must preserve declared field order:\n{composite}"
    );
    assert!(
        composite.contains("if f[1] != nil"),
        "nullable text must accept NULL:\n{composite}"
    );
    assert!(
        composite.contains("fieldValue = State(raw)"),
        "nullable enums must use their base type:\n{composite}"
    );
    assert!(
        composite.contains("DeliveryDetailsFromText(raw)"),
        "nullable nested composites must use their base type:\n{composite}"
    );
}
