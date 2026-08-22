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
                nullable: false,
            },
        ],
    }
}

fn query(command: QueryCommand) -> AnalyzedQuery {
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
fn php_postgresql_backends_encode_composite_params_as_text() {
    for backend_name in ["php-pdo", "php-amphp"] {
        let backend = get_backend(backend_name, "postgresql").expect("backend must exist");
        let composite = backend
            .generate_composite_def(&address_composite())
            .expect("composite generation must succeed");
        let result =
            generate_with_backend(&query(QueryCommand::Exec), &*backend).expect("query generation must succeed");
        let query_fn = result.query_fn.expect("exec query must emit a function");

        assert!(
            query_fn.contains("::text::address"),
            "{backend_name} missing cast:\n{query_fn}"
        );
        assert!(
            query_fn.contains("$home_address?->toPgText()"),
            "{backend_name} must preserve whole-composite null:\n{query_fn}"
        );
        assert!(
            composite.contains("public function toPgText(): string"),
            "missing encoder:\n{composite}"
        );
        assert!(
            composite.contains("self::encodeCompositeField($this->street)"),
            "encoder must preserve field order:\n{composite}"
        );
    }
}

#[test]
fn php_postgresql_batches_encode_composite_items() {
    for backend_name in ["php-pdo", "php-amphp"] {
        let backend = get_backend(backend_name, "postgresql").expect("backend must exist");
        let result =
            generate_with_backend(&query(QueryCommand::Batch), &*backend).expect("query generation must succeed");
        let query_fn = result.query_fn.expect("batch query must emit a function");

        assert!(
            query_fn.contains("$item[0]?->toPgText()"),
            "{backend_name} batch must encode composite items:\n{query_fn}"
        );
    }
}
