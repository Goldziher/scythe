#[allow(dead_code, unused_imports, clippy::all)]
mod queries;

use queries::{
    CreateOrderRow, CreateUserRow,
    GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
    UserStatus,
    GetUserProfileRow, UserAddress,
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
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable required");

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
    client
        .execute("DROP TYPE IF EXISTS user_address CASCADE", &[])
        .await?;

    let schema_sql = std::fs::read_to_string("../sql/pg/schema.sql")?;
    client.batch_execute(&schema_sql).await?;

    // Test: CreateUser
let row = client
        .query_one(
            "INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at",
            &[&"Alice", &"alice@example.com", &(UserStatus::Active)],
        )
        .await?;
    let user = CreateUserRow::from_row(&row);
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(
        user.email.as_deref() == Some("alice@example.com"),
        "CreateUser"
    );
    assert_test!(user.status == UserStatus::Active, "CreateUser");
    let user_id = user.id;
    pass!("CreateUser");

    // Test: GetUserById

    let row = client
        .query_one(
            "SELECT id, name, email, status, created_at FROM users WHERE id = $1",
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
    assert_test!(fetched.status == UserStatus::Active, "GetUserById");
    pass!("GetUserById");

    // Test: ListActiveUsers
let rows = client
        .query(
            "SELECT id, name, email FROM users WHERE status = $1",
            &[&(UserStatus::Active)],
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
    // Test: GetUserProfile (board #197) -- a nullable enum and a nullable
    // composite column, each observed both present and as SQL NULL. Seeded
    // via raw SQL because a composite VALUES literal is outside this
    // generator's parameter-binding surface; the point is the *read* path,
    // which runs through the generated `GetUserProfileRow`.
    let composite_address = UserAddress {
        street: Some(r#"12 "Main", Apt \3"#.to_string()),
        city: Some(String::new()),
        zip: Some("10115".to_string()),
    };
    let round_tripped_address = queries::round_trip_user_address(&client, Some(&composite_address)).await?;
    let returned_address = round_tripped_address.address.expect("RoundTripUserAddress address present");
    assert_test!(returned_address.street == composite_address.street, "RoundTripUserAddress street");
    assert_test!(returned_address.city == composite_address.city, "RoundTripUserAddress empty city");
    assert_test!(returned_address.zip == composite_address.zip, "RoundTripUserAddress zip");
    let round_tripped_null = queries::round_trip_user_address(&client, None).await?;
    assert_test!(round_tripped_null.address.is_none(), "RoundTripUserAddress null");
    pass!("RoundTripUserAddress");

    let present_row = client
        .query_one(
            "INSERT INTO users (name, email, status, secondary_status, address) \
             VALUES ($1, $2, 'active', 'inactive', ROW('1 Main St', 'Springfield', '12345')) RETURNING id",
            &[&"Carol", &"carol@example.com"],
        )
        .await?;
    let present_id: i32 = present_row.get(0);
    let absent_row = client
        .query_one(
            "INSERT INTO users (name, email, status, secondary_status, address) \
             VALUES ($1, $2, 'active', NULL, NULL) RETURNING id",
            &[&"Dave", &"dave@example.com"],
        )
        .await?;
    let absent_id: i32 = absent_row.get(0);

    let row = client
        .query_one(
            "SELECT id, secondary_status, address FROM users WHERE id = $1",
            &[&present_id],
        )
        .await?;
    let profile = GetUserProfileRow::from_row(&row);
    // Fails if a nullable enum reader zero-decodes instead of returning the value.
    assert_test!(profile.secondary_status == Some(UserStatus::Inactive), "GetUserProfile secondary_status present");
    // Fails if a nullable composite reader errors or returns zero fields on a present value.
    let address = profile.address.expect("GetUserProfile address should be present");
    assert_test!(address.street.as_deref() == Some("1 Main St"), "GetUserProfile address.street");
    assert_test!(address.city.as_deref() == Some("Springfield"), "GetUserProfile address.city");
    assert_test!(address.zip.as_deref() == Some("12345"), "GetUserProfile address.zip");

    let null_row = client
        .query_one(
            "SELECT id, secondary_status, address FROM users WHERE id = $1",
            &[&absent_id],
        )
        .await?;
    let null_profile = GetUserProfileRow::from_row(&null_row);
    // Fails if a nullable enum reader decodes SQL NULL as a zero/empty variant instead of None.
    assert_test!(null_profile.secondary_status.is_none(), "GetUserProfile secondary_status null");
    // Fails if a nullable composite reader decodes SQL NULL as a Some(all-default) value.
    assert_test!(null_profile.address.is_none(), "GetUserProfile address null");
    pass!("GetUserProfile (nullable enum + composite)");

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
            "SELECT id, name, email, status, created_at FROM users WHERE id = $1",
            &[&user_id],
        )
        .await?;
    let deleted: Option<GetUserByIdRow> = row.as_ref().map(GetUserByIdRow::from_row);
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
