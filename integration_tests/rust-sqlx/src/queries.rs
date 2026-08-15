// scythe:provenance v=0.16.0 backend=rust-sqlx engine=postgresql schema=sch1:c247390d575b8f71 queries=q1:a78685f58b075ff5 options=opt1:57af1d7acc85e6c7
#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "user_status", rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Inactive,
    Banned,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreateOrderRow {
    pub id: i32,
    pub user_id: i32,
    pub total: rust_decimal::Decimal,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetOrdersByUserRow {
    pub id: i32,
    pub total: rust_decimal::Decimal,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetOrderTotalRow {
    pub total_sum: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetOrderWeightTotalRow {
    pub weight_total: Option<f64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserByIdRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub status: UserStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListActiveUsersRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreateUserRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub status: UserStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserOrdersRow {
    pub id: i32,
    pub name: String,
    pub total: Option<rust_decimal::Decimal>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CountUsersByStatusRow {
    pub status: UserStatus,
    pub user_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserWithTagsRow {
    pub id: i32,
    pub name: String,
    pub tag_name: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchUsersRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, sqlx::Type)]
#[sqlx(type_name = "user_address")]
pub struct UserAddress {
    pub street: String,
    pub city: String,
    pub zip: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserProfileRow {
    pub id: i32,
    pub secondary_status: Option<UserStatus>,
    pub address: Option<UserAddress>,
}
