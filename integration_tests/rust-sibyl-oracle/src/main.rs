#[allow(dead_code, unused_imports, clippy::all)]
mod queries;

use queries::{
    CreateAttachmentRow, CreateOrderRow, CreateUserRow,
    GetAttachmentByIdRow, GetAttachmentsByOrderRow, GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
};
use sibyl::Environment;
use std::str::FromStr;

macro_rules! assert_test {
    ($cond:expr, $name:expr) => {
        if !($cond) {
            eprintln!("FAIL: {}: assertion failed: {}", $name, stringify!($cond));
            std::process::exit(1);
        }
    };
}

macro_rules! pass {
    ($name:expr) => {
        println!("PASS: {}", $name);
    };
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url =
        std::env::var("ORACLE_URL").expect("ORACLE_URL environment variable required");

// Parse oracle://user:pass@host:port/service URL
    let url = url::Url::parse(&database_url).expect("invalid ORACLE_URL");
    let oracle_user = url.username();
    let oracle_pass = url.password().unwrap_or("");
    let oracle_host = url.host_str().unwrap_or("localhost");
    let oracle_port = url.port().unwrap_or(1521);
    let oracle_service = url.path().trim_start_matches('/');
    let oracle_connect = format!("{}:{}/{}", oracle_host, oracle_port, oracle_service);

    let oracle = Environment::new().expect("Oracle environment");
    let session = oracle
        .connect(&oracle_connect, oracle_user, oracle_pass)
        .await?;

    // Clean slate: drop tables and sequences, ignore errors, then recreate
    for table in &["attachments", "user_tags", "tags", "orders", "users"] {
        if let Ok(stmt) = session.prepare(&format!("DROP TABLE {}", table)).await {
            let _ = stmt.execute(()).await;
        }
    }
    for seq in &["attachments_seq", "tags_seq", "orders_seq", "users_seq"] {
        if let Ok(stmt) = session.prepare(&format!("DROP SEQUENCE {}", seq)).await {
            let _ = stmt.execute(()).await;
        }
    }

    let schema_sql = std::fs::read_to_string("../sql/oracle/schema_full.sql")?;
    for block in schema_sql.split("/\n") {
        let block = block.trim();
        if !block.is_empty() {
            let stmt = session.prepare(block).await?;
            stmt.execute(()).await?;
        }
    }

    // Test: CreateUser
let user = queries::create_user(&session, "Alice", Some("alice@example.com"), 1i64)
        .await?
        .expect("create_user returned None");
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(
        user.email.as_deref() == Some("alice@example.com"),
        "CreateUser"
    );
    let user_id = user.id;
    pass!("CreateUser");

    // Test: GetUserById
let fetched = queries::get_user_by_id(&session, user_id)
        .await?
        .expect("get_user_by_id returned None");
    assert_test!(fetched.id == user_id, "GetUserById");
    assert_test!(fetched.name == "Alice", "GetUserById");
    assert_test!(
        fetched.email.as_deref() == Some("alice@example.com"),
        "GetUserById"
    );
    pass!("GetUserById");

    // Test: ListActiveUsers
let active_users = queries::list_active_users(&session).await?;
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder
let order_total: f64 = 9999.0;
    let order = queries::create_order(&session, user_id, order_total, Some("first order"))
        .await?
        .expect("create_order returned None");

    assert_test!(order.user_id == user_id, "CreateOrder");
    assert_test!(order.total == order_total, "CreateOrder");
    assert_test!(
        order.notes.as_deref() == Some("first order"),
        "CreateOrder"
    );
    pass!("CreateOrder");

    // Test: GetOrdersByUser
let orders = queries::get_orders_by_user(&session, user_id).await?;
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == order_total, "GetOrdersByUser");
    // Round-trips the CLOB `notes` column through the LOB-locator read path
    // (regression coverage for the fixed row.get::<String> CLOB defect).
    assert_test!(
        orders[0].notes.as_deref() == Some("first order"),
        "GetOrdersByUser"
    );
    pass!("GetOrdersByUser");

    // Test: CreateAttachment / GetAttachmentsByOrder / GetAttachmentById.
    // `payload` is BLOB (binary, non-UTF8-safe bytes) and `description` is
    // NCLOB (multi-byte text); both must round-trip through the LOB-locator
    // read loop, not the broken `row.get::<String>`/`row.get::<Vec<u8>>` path.
    let payload: Vec<u8> = (0u16..8000).map(|i| (i % 256) as u8).collect();
    let description = "attachment notes: caf\u{e9}, \u{1f600}, \u{4e2d}\u{6587}".repeat(50);
    let attachment = queries::create_attachment(
        &session,
        order.id,
        "report.bin",
        &payload,
        Some(description.as_str()),
    )
    .await?
    .expect("create_attachment returned None");
    assert_test!(attachment.order_id == order.id, "CreateAttachment");
    assert_test!(attachment.filename == "report.bin", "CreateAttachment");
    pass!("CreateAttachment");

    let attachments = queries::get_attachments_by_order(&session, order.id).await?;
    assert_test!(attachments.len() == 1, "GetAttachmentsByOrder");
    assert_test!(attachments[0].payload == payload, "GetAttachmentsByOrder");
    assert_test!(
        attachments[0].description.as_deref() == Some(description.as_str()),
        "GetAttachmentsByOrder"
    );
    pass!("GetAttachmentsByOrder");

    let fetched_attachment = queries::get_attachment_by_id(&session, attachment.id)
        .await?
        .expect("get_attachment_by_id returned None");
    assert_test!(fetched_attachment.payload == payload, "GetAttachmentById");
    assert_test!(
        fetched_attachment.description.as_deref() == Some(description.as_str()),
        "GetAttachmentById"
    );
    pass!("GetAttachmentById");

    // Test: DeleteUser (delete attachments, then orders, due to FK)
queries::delete_attachments_by_order(&session, order.id).await?;
    queries::delete_orders_by_user(&session, user_id).await?;
    queries::delete_user(&session, user_id).await?;
    // Verify user is gone
    let deleted = queries::get_user_by_id(&session, user_id).await?;
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
