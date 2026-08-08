// scythe:provenance v=0.14.0 backend=rust-sqlx engine=mariadb schema=sch1:262bec5a0954c973 queries=q1:2f37bd0f0a685c79
#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
pub enum UsersStatus {
    Active,
    Inactive,
    Banned,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreateOrderRow {
    pub id: i32,
    pub user_id: String,
    pub total: rust_decimal::Decimal,
    pub notes: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetOrdersByUserRow {
    pub id: i32,
    pub total: rust_decimal::Decimal,
    pub notes: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetOrderTotalRow {
    pub total_sum: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserByIdRow {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub status: String,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListActiveUsersRow {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreateUserRow {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchUsersRow {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
}
