use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo};
use scythe_core::parser::QueryCommand;

const JVM_BACKENDS: &[&str] = &[
    "java-jdbc",
    "java-r2dbc",
    "kotlin-jdbc",
    "kotlin-r2dbc",
    "kotlin-exposed",
];

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
fn jvm_backends_encode_nullable_composite_params_from_text() {
    for backend_name in JVM_BACKENDS {
        let backend = get_backend(backend_name, "postgresql").expect("JVM backend must support PostgreSQL");
        let composite = backend
            .generate_composite_def(&address_composite())
            .expect("composite generation must succeed");
        let result = generate_with_backend(&composite_query(QueryCommand::Exec), &*backend)
            .expect("query generation must succeed");
        let query_fn = result.query_fn.expect("exec query must emit a function");

        assert!(
            query_fn.contains("::text::address"),
            "{backend_name} must cast encoded text to the composite type:\n{query_fn}"
        );
        assert!(
            query_fn.contains("toPgText()"),
            "{backend_name} must encode the bound value:\n{query_fn}"
        );
        assert!(
            query_fn.contains("null") || query_fn.contains("?.toPgText()"),
            "{backend_name} must preserve whole-composite NULL:\n{query_fn}"
        );
        assert!(
            composite.contains("toPgText"),
            "{backend_name} must emit a composite encoder:\n{composite}"
        );
        assert!(
            composite.contains("street") && composite.contains("city"),
            "{backend_name} must encode fields in their declared order:\n{composite}"
        );
        assert!(
            composite.contains("raw.isEmpty()") && composite.contains("replace"),
            "{backend_name} must quote empty values and escape structural characters:\n{composite}"
        );
    }
}

#[test]
fn jvm_backends_encode_single_composite_batch_items() {
    for backend_name in JVM_BACKENDS {
        let backend = get_backend(backend_name, "postgresql").expect("JVM backend must support PostgreSQL");
        let result = generate_with_backend(&composite_query(QueryCommand::Batch), &*backend)
            .expect("batch query generation must succeed");
        let query_fn = result.query_fn.expect("batch query must emit a function");

        assert!(
            query_fn.contains("::text::address"),
            "{backend_name} batch SQL must cast encoded text:\n{query_fn}"
        );
        assert!(
            query_fn.contains("item") && query_fn.contains("toPgText()"),
            "{backend_name} batch items must use the same encoder:\n{query_fn}"
        );
    }
}
