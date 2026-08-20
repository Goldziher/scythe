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

fn composite_param(name: &str, position: i64) -> AnalyzedParam {
    AnalyzedParam {
        name: name.to_string(),
        neutral_type: "composite::address".to_string(),
        nullable: true,
        position,
        source_relation: None,
    }
}

#[test]
fn ruby_pg_encodes_composite_params_and_casts_from_text() {
    let query = AnalyzedQuery::build(|query| {
        query.name = "CreateWidget".to_string();
        query.command = QueryCommand::Exec;
        query.sql = "INSERT INTO widgets (home_address) VALUES ($1)".to_string();
        query.params = vec![composite_param("home_address", 1)];
        query.composites = vec![address_composite()];
    });
    let backend = get_backend("ruby-pg", "postgresql").expect("ruby-pg supports PostgreSQL");
    let composite = backend
        .generate_composite_def(&address_composite())
        .expect("composite generation must succeed");
    let result = generate_with_backend(&query, &*backend).expect("code generation must succeed");
    let query_fn = result.query_fn.expect("exec query must emit a function");

    assert!(
        query_fn.contains("$1::text::address"),
        "missing explicit composite cast:\n{query_fn}"
    );
    assert!(
        query_fn.contains("[home_address&.to_pg_text]"),
        "whole-composite null must remain nil while values are encoded:\n{query_fn}"
    );
    assert!(composite.contains("def to_pg_text"), "missing encoder:\n{composite}");
    assert!(
        composite.contains("self.class._encode_composite_field(street)"),
        "instance encoder must call the generated class helper:\n{composite}"
    );
    assert!(
        composite.contains("raw.empty? || raw.match?(/[(),\\\"\\\\]/) || raw != raw.strip"),
        "encoder must quote empty and structurally significant values:\n{composite}"
    );
    assert!(
        composite.contains(r#"raw.gsub('\\') { '\\\\' }.gsub('"', '""')"#),
        "encoder must escape backslashes and quotes:\n{composite}"
    );
}

#[test]
fn ruby_pg_encoder_preserves_backslashes_at_runtime() {
    let backend = get_backend("ruby-pg", "postgresql").expect("ruby-pg supports PostgreSQL");
    let composite = backend
        .generate_composite_def(&address_composite())
        .expect("composite generation must succeed");
    let script = format!(
        "module Queries\n{composite}\nend\nvalue = Queries::Address.new(street: '12 \"Main\", Apt \\\\3', city: '')\nprint value.to_pg_text"
    );
    let output = std::process::Command::new("ruby")
        .args(["-e", &script])
        .output()
        .expect("ruby must be available for the runtime encoder regression");

    assert!(
        output.status.success(),
        "generated encoder must run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("encoder output must be UTF-8"),
        r#"("12 ""Main"", Apt \\3","")"#
    );
}

#[test]
fn ruby_pg_encodes_composite_fields_in_multi_param_batches() {
    let query = AnalyzedQuery::build(|query| {
        query.name = "CreateWidget".to_string();
        query.command = QueryCommand::Batch;
        query.sql = "INSERT INTO widgets (kind, home_address) VALUES ($1, $2)".to_string();
        query.params = vec![
            AnalyzedParam {
                name: "kind".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            },
            composite_param("home_address", 2),
        ];
        query.composites = vec![address_composite()];
    });
    let backend = get_backend("ruby-pg", "postgresql").expect("ruby-pg supports PostgreSQL");
    let result = generate_with_backend(&query, &*backend).expect("code generation must succeed");
    let query_fn = result.query_fn.expect("batch query must emit a function");

    assert!(
        query_fn.contains("$2::text::address"),
        "missing batch composite cast:\n{query_fn}"
    );
    assert!(
        query_fn.contains("[item[0], item[1]&.to_pg_text]"),
        "batch must encode only the composite item field:\n{query_fn}"
    );
}
