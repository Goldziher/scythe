#[allow(dead_code, unused_imports, clippy::all)]
mod generated;

use generated::{
    GetLastInsertOrderRow, GetLastInsertUserRow, GetOrderTotalRow, GetOrdersByUserRow,
    GetUserByIdRow, ListActiveUsersRow,
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
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://scythe:scythe@localhost:3307/scythe_test".to_string());

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

    // Recreate tables from schema
    sqlx::query(
        "CREATE TABLE users (
            id INT AUTO_INCREMENT PRIMARY KEY,
            name VARCHAR(255) NOT NULL,
            email VARCHAR(255),
            status ENUM('active', 'inactive', 'banned') NOT NULL DEFAULT 'active',
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE orders (
            id INT AUTO_INCREMENT PRIMARY KEY,
            user_id INT NOT NULL,
            total DECIMAL(10, 2) NOT NULL,
            notes TEXT,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_id) REFERENCES users (id)
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE tags (
            id INT AUTO_INCREMENT PRIMARY KEY,
            name VARCHAR(255) NOT NULL UNIQUE
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE user_tags (
            user_id INT NOT NULL,
            tag_id INT NOT NULL,
            PRIMARY KEY (user_id, tag_id),
            FOREIGN KEY (user_id) REFERENCES users (id),
            FOREIGN KEY (tag_id) REFERENCES tags (id)
        )",
    )
    .execute(&pool)
    .await?;

    // Test: CreateUser
    // MySQL has no RETURNING, so we use :exec then fetch by last_insert_id from the result
    let insert_result = sqlx::query("INSERT INTO users (name, email, status) VALUES (?, ?, ?)")
        .bind("Alice")
        .bind("alice@example.com")
        .bind("active")
        .execute(&pool)
        .await?;
    let user_id = insert_result.last_insert_id() as i32;

    let user: GetLastInsertUserRow = sqlx::query_as(
        "SELECT id, name, email, status, created_at FROM users WHERE id = ?",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await?;
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(
        user.email.as_deref() == Some("alice@example.com"),
        "CreateUser"
    );
    assert_test!(user.status == "active", "CreateUser");
    assert_test!(user.id == user_id, "CreateUser");
    pass!("CreateUser");

    // Test: GetUserById
    let fetched: GetUserByIdRow =
        sqlx::query_as("SELECT id, name, email, status, created_at FROM users WHERE id = ?")
            .bind(user_id)
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
        sqlx::query_as("SELECT id, name, email FROM users WHERE status = ?")
            .bind("active")
            .fetch_all(&pool)
            .await?;
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder
    let total = Decimal::from_str("99.95").unwrap();
    let order_insert = sqlx::query("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)")
        .bind(user_id)
        .bind(&total)
        .bind("first order")
        .execute(&pool)
        .await?;
    let order_id = order_insert.last_insert_id() as i32;

    let order: GetLastInsertOrderRow = sqlx::query_as(
        "SELECT id, user_id, total, notes, created_at FROM orders WHERE id = ?",
    )
    .bind(order_id)
    .fetch_one(&pool)
    .await?;
    assert_test!(order.user_id == user_id, "CreateOrder");
    assert_test!(order.total == total, "CreateOrder");
    assert_test!(
        order.notes.as_deref() == Some("first order"),
        "CreateOrder"
    );
    pass!("CreateOrder");

    // Test: GetOrdersByUser
    let orders: Vec<GetOrdersByUserRow> = sqlx::query_as(
        "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await?;
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == total, "GetOrdersByUser");
    pass!("GetOrdersByUser");

    // Test: GetOrderTotal
    let order_total: GetOrderTotalRow =
        sqlx::query_as("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&pool)
            .await?;
    assert_test!(order_total.total_sum == Some(total), "GetOrderTotal");
    pass!("GetOrderTotal");

    // Test: DeleteOrdersByUser
    let delete_result = sqlx::query("DELETE FROM orders WHERE user_id = ?")
        .bind(user_id)
        .execute(&pool)
        .await?;
    assert_test!(delete_result.rows_affected() == 1, "DeleteOrdersByUser");
    pass!("DeleteOrdersByUser");

    // Test: DeleteUser
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(user_id)
        .execute(&pool)
        .await?;
    let deleted: Option<GetUserByIdRow> =
        sqlx::query_as("SELECT id, name, email, status, created_at FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&pool)
            .await?;
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
