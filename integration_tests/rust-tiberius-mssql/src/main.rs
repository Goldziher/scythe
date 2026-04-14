#[allow(dead_code, unused_imports, clippy::all)]
mod queries;

use queries::{
    CreateOrderRow, CreateUserRow,
    GetOrdersByUserRow, GetUserByIdRow, ListActiveUsersRow,
};
use tiberius::{Client, Config, AuthMethod};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;
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
        std::env::var("MSSQL_URL").expect("MSSQL_URL environment variable required");

let mut config = Config::from_ado_string(&database_url)?;
    config.trust_cert();
    let tcp = TcpStream::connect(config.get_addr()).await?;
    tcp.set_nodelay(true)?;
    let mut client = Client::connect(config, tcp.compat_write()).await?;

    // Clean slate
    for table in &["user_tags", "tags", "orders", "users"] {
        client
            .execute(
                &format!(
                    "IF OBJECT_ID('{}', 'U') IS NOT NULL DROP TABLE {}",
                    table, table
                ),
                &[],
            )
            .await?;
    }

    let schema_sql = std::fs::read_to_string("../sql/mssql/schema.sql")?;
    for stmt in schema_sql.split(';') {
        let stmt = stmt.trim();
        if !stmt.is_empty() {
            client.execute(stmt, &[]).await?;
        }
    }

    // Test: CreateUser
let mut stream = client
        .query(
            "INSERT INTO users (id, name, email, active) OUTPUT INSERTED.id, INSERTED.name, INSERTED.email, INSERTED.active, INSERTED.created_at VALUES (@p1, @p2, @p3, @p4)",
            &[&1i32, &"Alice", &"alice@example.com", &true],
        )
        .await?;
    let row = stream
        .into_row()
        .await?
        .expect("expected a row");
    let user = CreateUserRow::from_row(&row);
    assert_test!(user.name == "Alice", "CreateUser");
    assert_test!(
        user.email.as_deref() == Some("alice@example.com"),
        "CreateUser"
    );
    let user_id = user.id;
    pass!("CreateUser");

    // Test: GetUserById
let mut stream = client
        .query(
            "SELECT id, name, email, active, created_at FROM users WHERE id = @p1",
            &[&user_id],
        )
        .await?;
    let row = stream
        .into_row()
        .await?
        .expect("expected a row");
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
            "SELECT id, name, email FROM users WHERE active = CAST(1 AS BIT)",
            &[],
        )
        .await?
        .into_first_result()
        .await?;
    let active_users: Vec<ListActiveUsersRow> =
        rows.iter().map(ListActiveUsersRow::from_row).collect();
    assert_test!(!active_users.is_empty(), "ListActiveUsers");
    assert_test!(active_users[0].name == "Alice", "ListActiveUsers");
    pass!("ListActiveUsers");

    // Test: CreateOrder
let total = Decimal::from_str("99.95").unwrap();
let mut stream = client
        .query(
            "INSERT INTO orders (id, user_id, total, notes) OUTPUT INSERTED.id, INSERTED.user_id, INSERTED.total, INSERTED.notes, INSERTED.created_at VALUES (@p1, @p2, @p3, @p4)",
            &[&1i32, &user_id, &total, &"first order"],
        )
        .await?;
    let row = stream
        .into_row()
        .await?
        .expect("expected a row");
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
            "SELECT id, total, notes, created_at FROM orders WHERE user_id = @p1 ORDER BY created_at DESC",
            &[&user_id],
        )
        .await?
        .into_first_result()
        .await?;
    let orders: Vec<GetOrdersByUserRow> = rows.iter().map(GetOrdersByUserRow::from_row).collect();
    assert_test!(orders.len() == 1, "GetOrdersByUser");
    assert_test!(orders[0].total == total, "GetOrdersByUser");
    pass!("GetOrdersByUser");

    // Test: DeleteUser (delete orders first due to FK)
client
        .execute("DELETE FROM orders WHERE user_id = @p1", &[&user_id])
        .await?;
    client
        .execute("DELETE FROM users WHERE id = @p1", &[&user_id])
        .await?;
    // Verify user is gone
    let mut stream = client
        .query(
            "SELECT id, name, email, active, created_at FROM users WHERE id = @p1",
            &[&user_id],
        )
        .await?;
    let deleted = stream.into_row().await?;
    assert_test!(deleted.is_none(), "DeleteUser");
    pass!("DeleteUser");

    println!("ALL TESTS PASSED");
    Ok(())
}
