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

fn composite_query() -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
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
    })
}

#[test]
fn rust_postgresql_backends_keep_native_composite_binding() {
    for backend_name in ["rust-sqlx", "rust-tokio-postgres"] {
        let backend = get_backend(backend_name, "postgresql").expect("backend must exist");
        let composite = backend
            .generate_composite_def(&address_composite())
            .expect("composite generation must succeed");
        let result = generate_with_backend(&composite_query(), &*backend).expect("query generation must succeed");
        let query_fn = result.query_fn.expect("exec query must emit a function");

        assert!(
            query_fn.contains("home_address"),
            "{backend_name} must bind the declared native composite parameter:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("::text::address"),
            "{backend_name} has a native composite codec and must not downgrade to text:\n{query_fn}"
        );
        if backend_name == "rust-sqlx" {
            assert!(
                composite.contains("sqlx::Type"),
                "sqlx composite must derive its native codec:\n{composite}"
            );
        } else {
            assert!(
                composite.contains("postgres_types::ToSql"),
                "tokio-postgres composite must derive its native encoder:\n{composite}"
            );
        }
    }
}
