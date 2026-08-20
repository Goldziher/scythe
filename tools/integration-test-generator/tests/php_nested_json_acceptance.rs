use std::fs;
use std::path::{Path, PathBuf};

fn php_template() -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/php.php.jinja");
    fs::read_to_string(path).expect("read PHP integration template")
}

#[test]
fn php_nested_json_harness_asserts_the_seeded_aggregate_row() {
    let template = php_template();
    assert!(
        template.contains("$matches = array_values(array_filter(") && template.contains("$candidate->id === $user_id"),
        "PHP nested JSON harness must locate the row seeded by the test"
    );
    assert!(
        template.contains("GetUsersAsJson aggregate address.street"),
        "PHP nested JSON harness must assert exact nested aggregate fields"
    );
}

#[test]
fn php_nested_json_harness_asserts_empty_aggregate_is_null() {
    let template = php_template();
    assert!(
        template.contains("assert_null($empty->payload, \"GetUsersAsJson empty aggregate\")"),
        "PHP nested JSON harness must cover jsonb_agg over an empty input"
    );
}
