#[allow(dead_code, unused_imports, clippy::all)]
mod generated;

use generated::{
    CreateUserRow, GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
};
use rust_decimal::Decimal;
use sqlx::mysql::MySqlPoolOptions;
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
        std::env::var("MARIADB_URL").expect("MARIADB_URL environment variable required");

    let pool = MySqlPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    // Clean slate: drop tables in dependency order, then recreate
    sqlx::query("DROP TABLE IF EXISTS user_tags")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS tags")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS orders")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS users")
        .execute(&pool)
        .await?;

    let schema_sql = std::fs::read_to_string("../sql/mariadb/schema.sql")?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() {
            sqlx::query(stmt).execute(&pool).await?;
        }
    }

    // Test: CreateUser
    // Note: sqlx's MySQL driver doesn't support RETURNING result parsing,
    // so we INSERT then SELECT to get the created row.
    sqlx::query("INSERT INTO users (name, email, status) VALUES (?, ?, ?)")
        .bind("Alice")
        .bind("alice@example.com")
        .bind("active")
        .execute(&pool)
        .await?;
    let user: CreateUserRow = sqlx::query_as(
        "SELECT CAST(id AS CHAR) AS id, name, email FROM users WHERE name = ? AND email = ? LIMIT 1",
    )
    .bind("Alice")
    .bind("alice@example.com")
    .fetch_one(&pool)
    .await?;
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(
        user.email.as_deref() == Some("alice@example.com"),
        "CreateUser"
    );
    let user_id: String = user.id;
    pass!("CreateUser");

    // Test: GetUserById
    let fetched: GetUserByIdRow = sqlx::query_as(
        "SELECT CAST(id AS CHAR) AS id, name, email, status, created_at FROM users WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_one(&pool)
    .await?;
    assert_test!(fetched.id == user_id, "GetUserById");
    assert_test!(fetched.name == "Alice", "GetUserById");
    assert_test!(
        fetched.email.as_deref() == Some("alice@example.com"),
        "GetUserById"
    );
    assert_test!(fetched.status == "active", "GetUserById");
    pass!("GetUserById");

    // Test: ListActiveUsers
    let active_users: Vec<ListActiveUsersRow> =
        sqlx::query_as("SELECT CAST(id AS CHAR) AS id, name, email FROM users WHERE status = ?")
            .bind("active")
            .fetch_all(&pool)
            .await?;
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder
    let total = Decimal::from_str("99.95").unwrap();
    sqlx::query("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)")
        .bind(&user_id)
        .bind(&total)
        .bind("first order")
        .execute(&pool)
        .await?;
    let orders_check: Vec<GetOrdersByUserRow> = sqlx::query_as(
        "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(&user_id)
    .fetch_all(&pool)
    .await?;
    assert_test!(!orders_check.is_empty(), "CreateOrder");
    assert_test!(orders_check[0].total == total, "CreateOrder");
    assert_test!(
        orders_check[0].notes.as_deref() == Some("first order"),
        "CreateOrder"
    );
    pass!("CreateOrder");

    // Test: GetOrdersByUser
    let orders: Vec<GetOrdersByUserRow> = sqlx::query_as(
        "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(&user_id)
    .fetch_all(&pool)
    .await?;
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == total, "GetOrdersByUser");
    pass!("GetOrdersByUser");

    // Test: DeleteUser (delete orders first due to FK constraint)
    sqlx::query("DELETE FROM orders WHERE user_id = ?")
        .bind(&user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(&user_id)
        .execute(&pool)
        .await?;
    let deleted: Option<GetUserByIdRow> = sqlx::query_as(
        "SELECT CAST(id AS CHAR) AS id, name, email, status, created_at FROM users WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(&pool)
    .await?;
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
