#[allow(dead_code, unused_imports, clippy::all)]
mod queries;

use queries::{
    CreateOrderRow, CreateUserRow,
    GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
    UserStatus,
    GetUserProfileRow, UserAddress,
    RoundTripUserAddressRow,
};
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
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
    sqlx::query("DROP TYPE IF EXISTS user_address CASCADE")
        .execute(&pool)
        .await?;

    let schema_sql = std::fs::read_to_string("../sql/pg/schema.sql")?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(schema_sql)).execute(&pool).await?;

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
    // Test: GetUserProfile (board #197) -- a nullable enum and a nullable
    // composite column, each observed both present and as SQL NULL. Seeded
    // via raw SQL because a composite VALUES literal is outside this
    // generator's parameter-binding surface; the point is the *read* path,
    // which runs through the generated `GetUserProfileRow`.
    let composite_address = UserAddress {
        street: r#"12 "Main", Apt \3"#.to_string(),
        city: String::new(),
        zip: "10115".to_string(),
    };
    let round_tripped_address: RoundTripUserAddressRow = sqlx::query_as(
        "INSERT INTO users (name, status, address) VALUES ('Composite Parameter Round Trip', 'active', $1) RETURNING address",
    )
    .bind(Some(composite_address.clone()))
    .fetch_one(&pool)
    .await?;
    let returned_address = round_tripped_address.address.expect("RoundTripUserAddress address present");
    assert_test!(returned_address.street == composite_address.street, "RoundTripUserAddress street");
    assert_test!(returned_address.city == composite_address.city, "RoundTripUserAddress empty city");
    assert_test!(returned_address.zip == composite_address.zip, "RoundTripUserAddress zip");
    let round_tripped_null: RoundTripUserAddressRow = sqlx::query_as(
        "INSERT INTO users (name, status, address) VALUES ('Composite Parameter Round Trip', 'active', $1) RETURNING address",
    )
    .bind(Option::<UserAddress>::None)
    .fetch_one(&pool)
    .await?;
    assert_test!(round_tripped_null.address.is_none(), "RoundTripUserAddress null");
    pass!("RoundTripUserAddress");

    let present_row = sqlx::query(
        "INSERT INTO users (name, email, status, secondary_status, address) \
         VALUES ($1, $2, 'active', 'inactive', ROW('1 Main St', 'Springfield', '12345')) RETURNING id",
    )
    .bind("Carol")
    .bind("carol@example.com")
    .fetch_one(&pool)
    .await?;
    let present_id: i32 = present_row.get(0);
    let absent_row = sqlx::query(
        "INSERT INTO users (name, email, status, secondary_status, address) \
         VALUES ($1, $2, 'active', NULL, NULL) RETURNING id",
    )
    .bind("Dave")
    .bind("dave@example.com")
    .fetch_one(&pool)
    .await?;
    let absent_id: i32 = absent_row.get(0);

    let profile: GetUserProfileRow =
        sqlx::query_as("SELECT id, secondary_status, address FROM users WHERE id = $1")
            .bind(present_id)
            .fetch_one(&pool)
            .await?;
    // Fails if a nullable enum reader zero-decodes instead of returning the value.
    assert_test!(profile.secondary_status == Some(UserStatus::Inactive), "GetUserProfile secondary_status present");
    // Fails if a nullable composite reader errors or returns zero fields on a present value.
    let address = profile.address.expect("GetUserProfile address should be present");
    assert_test!(address.street == "1 Main St", "GetUserProfile address.street");
    assert_test!(address.city == "Springfield", "GetUserProfile address.city");
    assert_test!(address.zip == "12345", "GetUserProfile address.zip");

    let null_profile: GetUserProfileRow =
        sqlx::query_as("SELECT id, secondary_status, address FROM users WHERE id = $1")
            .bind(absent_id)
            .fetch_one(&pool)
            .await?;
    // Fails if a nullable enum reader decodes SQL NULL as a zero/empty variant instead of None.
    assert_test!(null_profile.secondary_status.is_none(), "GetUserProfile secondary_status null");
    // Fails if a nullable composite reader decodes SQL NULL as a Some(all-default) value.
    assert_test!(null_profile.address.is_none(), "GetUserProfile address null");
    pass!("GetUserProfile (nullable enum + composite)");

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
