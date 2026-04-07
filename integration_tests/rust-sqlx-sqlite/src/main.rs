#[allow(dead_code, unused_imports, clippy::all)]
mod generated;

use generated::{GetOrderTotalRow, GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow};
use sqlx::sqlite::SqlitePoolOptions;

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
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;

    // Enable foreign keys
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;

    // Create tables from schema
    sqlx::query(
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            email TEXT,
            status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'inactive', 'banned')),
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE orders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL REFERENCES users (id),
            total REAL NOT NULL,
            notes TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE user_tags (
            user_id INTEGER NOT NULL REFERENCES users (id),
            tag_id INTEGER NOT NULL REFERENCES tags (id),
            PRIMARY KEY (user_id, tag_id)
        )",
    )
    .execute(&pool)
    .await?;

    // Test: CreateUser
    // SQLite has no RETURNING, so we use :exec then raw SELECT last_insert_rowid()
    sqlx::query("INSERT INTO users (name, email, status) VALUES (?, ?, ?)")
        .bind("Alice")
        .bind("alice@example.com")
        .bind("active")
        .execute(&pool)
        .await?;

    let row: (i32,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&pool)
        .await?;
    let user_id = row.0;
    assert_test!(user_id > 0, "CreateUser");
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
    sqlx::query("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)")
        .bind(user_id)
        .bind(99.95_f64)
        .bind("first order")
        .execute(&pool)
        .await?;

    let order_row: (i32,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&pool)
        .await?;
    let order_id = order_row.0;
    assert_test!(order_id > 0, "CreateOrder");
    pass!("CreateOrder");

    // Test: GetOrdersByUser
    let orders: Vec<GetOrdersByUserRow> = sqlx::query_as(
        "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await?;
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!((orders[0].total - 99.95_f32).abs() < 0.01, "GetOrdersByUser");
    assert_test!(
        orders[0].notes.as_deref() == Some("first order"),
        "GetOrdersByUser"
    );
    pass!("GetOrdersByUser");

    // Test: GetOrderTotal
    let order_total: GetOrderTotalRow =
        sqlx::query_as("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&pool)
            .await?;
    assert_test!(order_total.total_sum.is_some(), "GetOrderTotal");
    assert_test!(
        (order_total.total_sum.unwrap() - 99.95_f64).abs() < 0.01,
        "GetOrderTotal"
    );
    pass!("GetOrderTotal");

    // Test: DeleteOrdersByUser (must delete orders before user due to FK)
    let delete_result = sqlx::query("DELETE FROM orders WHERE user_id = ?")
        .bind(user_id)
        .execute(&pool)
        .await?;
    assert_test!(
        delete_result.rows_affected() == 1,
        "DeleteOrdersByUser"
    );
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
