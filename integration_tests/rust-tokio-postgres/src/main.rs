#[allow(dead_code, unused_imports, clippy::all)]
mod generated;

use generated::*;
use rust_decimal::Decimal;
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
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable required");

    let (client, connection) = tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("connection error: {error}");
        }
    });

    // Clean slate: drop tables in dependency order, then recreate
    client
        .batch_execute(
            "DROP TABLE IF EXISTS user_tags CASCADE;
             DROP TABLE IF EXISTS tags CASCADE;
             DROP TABLE IF EXISTS orders CASCADE;
             DROP TABLE IF EXISTS users CASCADE;
             DROP TYPE IF EXISTS user_status CASCADE;
             CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
             CREATE TABLE users (
                 id SERIAL PRIMARY KEY,
                 name TEXT NOT NULL,
                 email TEXT,
                 status user_status NOT NULL DEFAULT 'active',
                 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
             );
             CREATE TABLE orders (
                 id SERIAL PRIMARY KEY,
                 user_id INT NOT NULL REFERENCES users (id),
                 total NUMERIC(10, 2) NOT NULL,
                 notes TEXT,
                 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
             );
             CREATE TABLE tags (
                 id SERIAL PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE
             );
             CREATE TABLE user_tags (
                 user_id INT NOT NULL REFERENCES users (id),
                 tag_id INT NOT NULL REFERENCES tags (id),
                 PRIMARY KEY (user_id, tag_id)
             );",
        )
        .await?;

    // Test: CreateUser
    let user = create_user(&client, "Alice", "alice@example.com", &UserStatus::Active).await?;
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(user.email.as_deref() == Some("alice@example.com"), "CreateUser");
    assert_test!(user.status == UserStatus::Active, "CreateUser");
    let user_id = user.id;
    pass!("CreateUser");

    // Test: GetUserById
    let fetched = get_user_by_id(&client, user_id).await?;
    assert_test!(fetched.id == user_id, "GetUserById");
    assert_test!(fetched.name == "Alice", "GetUserById");
    assert_test!(
        fetched.email.as_deref() == Some("alice@example.com"),
        "GetUserById"
    );
    assert_test!(fetched.status == UserStatus::Active, "GetUserById");
    pass!("GetUserById");

    // Test: ListActiveUsers
    let active_users = list_active_users(&client, &UserStatus::Active).await?;
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder
    let total = Decimal::from_str("99.95").unwrap();
    let order = create_order(&client, user_id, &total, "first order").await?;
    assert_test!(order.user_id == user_id, "CreateOrder");
    assert_test!(order.total == total, "CreateOrder");
    assert_test!(order.notes.as_deref() == Some("first order"), "CreateOrder");
    pass!("CreateOrder");

    // Test: GetOrdersByUser
    let orders = get_orders_by_user(&client, user_id).await?;
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == total, "GetOrdersByUser");
    pass!("GetOrdersByUser");

    // Test: DeleteUser (delete orders first due to FK)
    let deleted_orders = delete_orders_by_user(&client, user_id).await?;
    assert_test!(deleted_orders == 1, "DeleteUser");
    delete_user(&client, user_id).await?;
    // Verify user is gone - query_one should fail
    let result = get_user_by_id(&client, user_id).await;
    assert_test!(result.is_err(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
