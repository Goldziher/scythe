//! Integration test for nested-aggregate structs (#78).
//!
//! Kept separate from `integration_tests/rust-sqlx` so the dependency set the
//! nested path needs (`sqlx/json`, `serde`) is visible on its own, and so a
//! failure here points at nested aggregates rather than at the general sqlx
//! surface.
//!
//! What only a live server can check: that the JSON keys `json_agg` and
//! `row_to_json` actually emit line up with the generated field names, that
//! `[null]` from a LEFT JOIN with no match deserializes, and that an enum
//! inside a nested struct round-trips through serde rather than through the
//! driver.

#[allow(dead_code, unused_imports, clippy::all)]
mod queries;

use queries::{
    GetUserAsJsonRow, GetUsersWithOrdersOuterRow, GetUsersWithOrdersRow, RoundTripUserAddressRow, UserAddress,
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

    for stmt in [
        "DROP TABLE IF EXISTS user_tags CASCADE",
        "DROP TABLE IF EXISTS tags CASCADE",
        "DROP TABLE IF EXISTS orders CASCADE",
        "DROP TABLE IF EXISTS users CASCADE",
        "DROP TYPE IF EXISTS user_status CASCADE",
        // ~keep This project reuses ../sql/pg/schema.sql, which defines user_address, and every
        // postgres project in the CI job shares one database. Dropping only user_status left the
        // composite behind and the next CREATE TYPE failed with 42710.
        "DROP TYPE IF EXISTS user_address CASCADE",
    ] {
        sqlx::query(stmt).execute(&pool).await?;
    }

    let schema_sql = std::fs::read_to_string("../sql/pg/schema.sql")?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(schema_sql))
        .execute(&pool)
        .await?;

    // Alice has two orders; Bob has none, which is what makes the LEFT JOIN
    // produce the `[null]` element the generated `Option<_>` exists for.
    let alice_id: i32 =
        sqlx::query_scalar("INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id")
            .bind("Alice")
            .bind("alice@example.com")
            .bind(UserStatus::Active)
            .fetch_one(&pool)
            .await?;
    sqlx::query_scalar::<_, i32>("INSERT INTO users (name, status) VALUES ($1, $2) RETURNING id")
        .bind("Bob")
        .bind(UserStatus::Inactive)
        .fetch_one(&pool)
        .await?;

    for (total, notes) in [("99.95", "first order"), ("10.00", "second order")] {
        sqlx::query("INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3)")
            .bind(alice_id)
            .bind(Decimal::from_str(total).unwrap())
            .bind(notes)
            .execute(&pool)
            .await?;
    }

    // Test: GetUsersWithOrders -- json_agg over an INNER JOIN.
    let rows: Vec<GetUsersWithOrdersRow> = sqlx::query_as(
        "SELECT u.id, u.name, json_agg(o.*) AS orders \
         FROM users u JOIN orders o ON o.user_id = u.id GROUP BY u.id, u.name",
    )
    .fetch_all(&pool)
    .await?;
    assert_test!(rows.len() == 1, "GetUsersWithOrders");
    let orders = rows[0].orders.as_ref().expect("orders present").0.clone();
    assert_test!(orders.len() == 2, "GetUsersWithOrders");
    assert_test!(orders.iter().all(|o| o.user_id == alice_id), "GetUsersWithOrders");
    // Proves the JSON keys map onto the generated fields: a wrong key would
    // have failed deserialization above, and a wrong *type* here.
    assert_test!(
        orders.iter().any(|o| o.total == Decimal::from_str("99.95").unwrap()),
        "GetUsersWithOrders"
    );
    assert_test!(
        orders.iter().all(|o| o.weight_kg.is_none()),
        "GetUsersWithOrders"
    );
    assert_test!(
        orders.iter().any(|o| o.notes.as_deref() == Some("second order")),
        "GetUsersWithOrders"
    );
    pass!("GetUsersWithOrders");

    // Test: GetUsersWithOrdersOuter -- json_agg over a LEFT JOIN. Bob has no
    // orders, so PostgreSQL aggregates one NULL whole-row value and the
    // column is the JSON array `[null]`. Without the Option element this
    // deserialization fails with "invalid type: null".
    let outer: Vec<GetUsersWithOrdersOuterRow> = sqlx::query_as(
        "SELECT u.id, json_agg(o.*) AS orders \
         FROM users u LEFT JOIN orders o ON o.user_id = u.id GROUP BY u.id",
    )
    .fetch_all(&pool)
    .await?;
    assert_test!(outer.len() == 2, "GetUsersWithOrdersOuter");
    let bob = outer
        .iter()
        .find(|r| r.id != alice_id)
        .expect("Bob's row present");
    let bob_orders = bob.orders.as_ref().expect("orders present").0.clone();
    assert_test!(bob_orders.len() == 1, "GetUsersWithOrdersOuter");
    assert_test!(bob_orders[0].is_none(), "GetUsersWithOrdersOuter");
    let alice = outer
        .iter()
        .find(|r| r.id == alice_id)
        .expect("Alice's row present");
    let alice_orders = alice.orders.as_ref().expect("orders present").0.clone();
    assert_test!(alice_orders.len() == 2, "GetUsersWithOrdersOuter");
    assert_test!(
        alice_orders.iter().all(|o| o.is_some()),
        "GetUsersWithOrdersOuter"
    );
    pass!("GetUsersWithOrdersOuter");

    // Test: GetUserAsJson -- row_to_json, whose nested struct carries an
    // enum field. The enum is decoded by serde here, not by sqlx, which is
    // why its definition needs serde derives the plain enum path omits.
    let payloads: Vec<GetUserAsJsonRow> =
        sqlx::query_as("SELECT row_to_json(u.*) AS payload FROM users u ORDER BY u.id")
            .fetch_all(&pool)
            .await?;
    assert_test!(payloads.len() == 2, "GetUserAsJson");
    let first = payloads[0].payload.as_ref().expect("payload present").0.clone();
    assert_test!(first.name == "Alice", "GetUserAsJson");
    assert_test!(first.email.as_deref() == Some("alice@example.com"), "GetUserAsJson");
    assert_test!(first.status == UserStatus::Active, "GetUserAsJson");
    let second = payloads[1].payload.as_ref().expect("payload present").0.clone();
    assert_test!(second.email.is_none(), "GetUserAsJson");
    assert_test!(second.status == UserStatus::Inactive, "GetUserAsJson");
    pass!("GetUserAsJson");

    let address = UserAddress {
        street: r#"12 "Main", Apt \3"#.to_string(),
        city: String::new(),
        zip: "10115".to_string(),
    };
    let present: RoundTripUserAddressRow = sqlx::query_as(
        "INSERT INTO users (name, status, address) VALUES ('Composite Parameter Round Trip', 'active', $1) RETURNING address",
    )
    .bind(Some(address.clone()))
    .fetch_one(&pool)
    .await?;
    let returned = present.address.expect("composite address present");
    assert_test!(returned.street == address.street, "RoundTripUserAddress street");
    assert_test!(returned.city == address.city, "RoundTripUserAddress city");
    assert_test!(returned.zip == address.zip, "RoundTripUserAddress zip");
    let absent: RoundTripUserAddressRow = sqlx::query_as(
        "INSERT INTO users (name, status, address) VALUES ('Composite Parameter Round Trip', 'active', $1) RETURNING address",
    )
    .bind(Option::<UserAddress>::None)
    .fetch_one(&pool)
    .await?;
    assert_test!(absent.address.is_none(), "RoundTripUserAddress null");
    pass!("RoundTripUserAddress");

    println!("All nested-aggregate integration tests passed.");
    Ok(())
}
