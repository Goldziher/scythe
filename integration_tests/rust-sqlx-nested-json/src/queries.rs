// scythe:provenance v=0.18.0 backend=rust-sqlx engine=postgresql schema=sch2:59e0edaa3ac94824 queries=q1:7067fdb0f4886dd1 options=opt1:57af1d7acc85e6c7
#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(type_name = "user_status", rename_all = "snake_case")]
pub enum UserStatus {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "inactive")]
    Inactive,
    #[serde(rename = "banned")]
    Banned,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GetUsersWithOrdersRowOrders {
    pub id: i32,
    pub user_id: i32,
    pub total: rust_decimal::Decimal,
    pub weight_kg: Option<f64>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GetUsersWithOrdersOuterRowOrders {
    pub id: i32,
    pub user_id: i32,
    pub total: rust_decimal::Decimal,
    pub weight_kg: Option<f64>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GetUserAsJsonRowPayload {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub status: UserStatus,
    pub secondary_status: Option<UserStatus>,
    pub address: Option<UserAddress>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUsersWithOrdersRow {
    pub id: i32,
    pub name: String,
    pub orders: Option<sqlx::types::Json<Vec<GetUsersWithOrdersRowOrders>>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUsersWithOrdersOuterRow {
    pub id: i32,
    pub orders: Option<sqlx::types::Json<Vec<Option<GetUsersWithOrdersOuterRowOrders>>>>,
}

#[derive(Debug, Clone, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(type_name = "user_address")]
pub struct UserAddress {
    pub street: Option<String>,
    pub city: Option<String>,
    pub zip: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserAsJsonRow {
    pub payload: Option<sqlx::types::Json<GetUserAsJsonRowPayload>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RoundTripUserAddressRow {
    pub address: Option<UserAddress>,
}
