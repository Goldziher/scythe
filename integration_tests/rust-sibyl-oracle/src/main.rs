#[allow(dead_code, unused_imports, clippy::all)]
mod generated;

use generated::{
    CreateOrderRow, CreateUserRow, GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
};
use sibyl::prelude::*;
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

    let oracle = Oracle::new().expect("Oracle environment");
    let session = oracle
        .connect(&oracle_connect, oracle_user, oracle_pass)
        .await?;

    // Clean slate: drop tables and sequences, ignore errors, then recreate
    for table in &["user_tags", "tags", "orders", "users"] {
        if let Ok(stmt) = session.prepare(&format!("DROP TABLE {}", table)).await {
            let _ = stmt.execute("").await;
        }
    }
    for seq in &["tags_seq", "orders_seq", "users_seq"] {
        if let Ok(stmt) = session.prepare(&format!("DROP SEQUENCE {}", seq)).await {
            let _ = stmt.execute("").await;
        }
    }

    let schema_sql = std::fs::read_to_string("../sql/oracle/schema_full.sql")?;
    for block in schema_sql.split("/\n") {
        let block = block.trim();
        if !block.is_empty() {
            let stmt = session.prepare(block).await?;
            stmt.execute("").await?;
        }
    }

    // Test: CreateUser

let user = generated::create_user(&session, "Alice", "alice@example.com", 1i32)
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
let fetched = generated::get_user_by_id(&session, user_id)
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
let active_users = generated::list_active_users(&session).await?;
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder
let order_total: i32 = 9999;
    let order = generated::create_order(&session, user_id, order_total, "first order")
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
let orders = generated::get_orders_by_user(&session, user_id).await?;
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == order_total, "GetOrdersByUser");
    pass!("GetOrdersByUser");

    // Test: DeleteUser (delete orders first due to FK)
generated::delete_orders_by_user(&session, user_id).await?;
    generated::delete_user(&session, user_id).await?;
    // Verify user is gone
    let deleted = generated::get_user_by_id(&session, user_id).await?;
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
