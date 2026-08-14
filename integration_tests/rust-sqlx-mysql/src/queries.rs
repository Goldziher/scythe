// scythe:provenance v=0.14.0 backend=rust-sqlx engine=mysql schema=sch1:4332a9c33cb39297 queries=q1:f928696deb211f90 options=opt1:57af1d7acc85e6c7
#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
pub enum UsersStatus {
    Active,
    Inactive,
    Banned,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetLastInsertOrderRow {
    pub id: i32,
    pub user_id: i32,
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
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub status: String,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListActiveUsersRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetLastInsertUserRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub status: String,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchUsersRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
}
