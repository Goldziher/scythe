#[allow(dead_code, unused_imports, clippy::all)]
mod queries;

use queries::{
    CreateOrderRow, CreateUserRow,
    GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
    UserStatus,
};
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
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

let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    // Clean slate: drop tables in dependency order, then recreate
    sqlx::query("DROP TABLE IF EXISTS user_tags CASCADE")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS tags CASCADE")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS orders CASCADE")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS users CASCADE")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TYPE IF EXISTS user_status CASCADE")
        .execute(&pool)
        .await?;

    let schema_sql = std::fs::read_to_string("../sql/pg/schema.sql")?;
    sqlx::query(&schema_sql).execute(&pool).await?;

    // Test: CreateUser
let user: CreateUserRow = sqlx::query_as(
        "INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at",
    )
    .bind("Alice")
    .bind("alice@example.com")
    .bind(UserStatus::Active)
    .fetch_one(&pool)
    .await?;
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(
        user.email.as_deref() == Some("alice@example.com"),
        "CreateUser"
    );
    assert_test!(user.status == UserStatus::Active, "CreateUser");
    let user_id = user.id;
    pass!("CreateUser");

    // Test: GetUserById
let fetched: GetUserByIdRow =
        sqlx::query_as("SELECT id, name, email, status, created_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await?;
    assert_test!(fetched.id == user_id, "GetUserById");
    assert_test!(fetched.name == "Alice", "GetUserById");
    assert_test!(
        fetched.email.as_deref() == Some("alice@example.com"),
        "GetUserById"
    );
    assert_test!(fetched.status == UserStatus::Active, "GetUserById");
    pass!("GetUserById");

    // Test: ListActiveUsers
let active_users: Vec<ListActiveUsersRow> =
        sqlx::query_as("SELECT id, name, email FROM users WHERE status = $1")
            .bind(UserStatus::Active)
            .fetch_all(&pool)
            .await?;
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder

    let total = Decimal::from_str("99.95").unwrap();
let order: CreateOrderRow = sqlx::query_as(
        "INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at",
    )
    .bind(user_id)
    .bind(&total)
    .bind("first order")
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
        "SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await?;
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == total, "GetOrdersByUser");
    pass!("GetOrdersByUser");

    // Test: DeleteUser (delete orders first due to FK)
sqlx::query("DELETE FROM orders WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;
    // Verify user is gone
    let deleted: Option<GetUserByIdRow> =
        sqlx::query_as("SELECT id, name, email, status, created_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&pool)
            .await?;
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
