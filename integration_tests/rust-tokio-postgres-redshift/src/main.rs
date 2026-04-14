#[allow(dead_code, unused_imports, clippy::all)]
mod queries;

use queries::{
    CreateOrderRow, CreateUserRow,
    GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
};
use rust_decimal::Decimal;
use tokio_postgres::NoTls;
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
        std::env::var("REDSHIFT_URL").expect("REDSHIFT_URL environment variable required");

let (client, connection) = tokio_postgres::connect(&database_url, NoTls).await?;

    // Spawn connection handler
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("connection error: {}", err);
        }
    });

    // Clean slate: drop tables in dependency order, then recreate
    client
        .execute("DROP TABLE IF EXISTS user_tags CASCADE", &[])
        .await?;
    client
        .execute("DROP TABLE IF EXISTS tags CASCADE", &[])
        .await?;
    client
        .execute("DROP TABLE IF EXISTS orders CASCADE", &[])
        .await?;
    client
        .execute("DROP TABLE IF EXISTS users CASCADE", &[])
        .await?;
    client
        .execute("DROP TYPE IF EXISTS user_status CASCADE", &[])
        .await?;

    let schema_sql = std::fs::read_to_string("../sql/redshift/schema.sql")?;
    client.batch_execute(&schema_sql).await?;

    // Test: CreateUser
let row = client
        .query_one(
            "INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at",
            &[&"Alice", &"alice@example.com", &UserStatus::Active],
        )
        .await?;
    let user = CreateUserRow::from_row(&row);
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(
        user.email.as_deref() == Some("alice@example.com"),
        "CreateUser"
    );
    let user_id = user.id;
    pass!("CreateUser");

    // Test: GetUserById

    let row = client
        .query_one(
            "SELECT id, name, email, created_at FROM users WHERE id = $1",
            &[&user_id],
        )
        .await?;
    let fetched = GetUserByIdRow::from_row(&row);
    assert_test!(fetched.id == user_id, "GetUserById");
    assert_test!(fetched.name == "Alice", "GetUserById");
    assert_test!(
        fetched.email.as_deref() == Some("alice@example.com"),
        "GetUserById"
    );
    pass!("GetUserById");

    // Test: ListActiveUsers

    let rows = client
        .query(
            "SELECT id, name, email FROM users WHERE status = $1",
            &[],
        )
        .await?;
    let active_users: Vec<ListActiveUsersRow> = rows.iter().map(ListActiveUsersRow::from_row).collect();
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder

    let total = Decimal::from_str("99.95").unwrap();

    let row = client
        .query_one(
            "INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at",
            &[&user_id, &total, &"first order"],
        )
        .await?;
    let order = CreateOrderRow::from_row(&row);
    assert_test!(order.user_id == user_id, "CreateOrder");
    assert_test!(order.total == total, "CreateOrder");
    assert_test!(
        order.notes.as_deref() == Some("first order"),
        "CreateOrder"
    );
    pass!("CreateOrder");

    // Test: GetOrdersByUser

    let rows = client
        .query(
            "SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC",
            &[&user_id],
        )
        .await?;
    let orders: Vec<GetOrdersByUserRow> = rows.iter().map(GetOrdersByUserRow::from_row).collect();
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == total, "GetOrdersByUser");
    pass!("GetOrdersByUser");

    // Test: DeleteUser (delete orders first due to FK)

    client
        .execute("DELETE FROM orders WHERE user_id = $1", &[&user_id])
        .await?;
    client
        .execute("DELETE FROM users WHERE id = $1", &[&user_id])
        .await?;
    // Verify user is gone
    let row = client
        .query_opt(
            "SELECT id, name, email, created_at FROM users WHERE id = $1",
            &[&user_id],
        )
        .await?;
    let deleted: Option<GetUserByIdRow> = row.as_ref().map(GetUserByIdRow::from_row);
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
