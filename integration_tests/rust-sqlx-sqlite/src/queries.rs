// scythe:provenance v=0.17.0 backend=rust-sqlx engine=sqlite schema=sch2:588fb635332179bc queries=q1:f7199f36438b6396 options=opt1:57af1d7acc85e6c7
#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetOrdersByUserRow {
    pub id: i64,
    pub total: f64,
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetOrderTotalRow {
    pub total_sum: Option<f64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserByIdRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListActiveUsersRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchUsersRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
}
