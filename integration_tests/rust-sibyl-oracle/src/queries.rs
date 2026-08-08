// scythe:provenance v=0.13.0 backend=rust-sibyl engine=oracle schema=sch1:51c12e41405f20c2
use sibyl::*;

#[derive(Debug, Clone)]
pub struct CreateAttachmentRow {
    pub id: i64,
    pub order_id: i64,
    pub filename: String,
}

pub async fn create_attachment<'a>(
    session: &'a Session<'a>,
    order_id: i64,
    filename: &str,
    payload: &[u8],
    description: Option<&str>,
) -> sibyl::Result<Option<CreateAttachmentRow>> {
    let stmt = session.prepare("INSERT INTO attachments (order_id, filename, payload, description) VALUES (:ORDER_ID, :FILENAME, :PAYLOAD, :DESCRIPTION) RETURNING id, order_id, filename INTO :OUT_ID, :OUT_ORDER_ID, :OUT_FILENAME").await?;
    let mut out_id: i64 = 0;
    let mut out_order_id: i64 = 0;
    let mut out_filename = String::with_capacity(4000);
    stmt.execute((
        (":ORDER_ID", order_id),
        (":FILENAME", filename),
        (":PAYLOAD", payload),
        (":DESCRIPTION", description),
        (":OUT_ID", &mut out_id),
        (":OUT_ORDER_ID", &mut out_order_id),
        (":OUT_FILENAME", &mut out_filename),
    ))
    .await?;
    let id = out_id;
    let order_id = out_order_id;
    let filename = out_filename;
    Ok(Some(CreateAttachmentRow {
        id: id,
        order_id: order_id,
        filename: filename,
    }))
}

#[derive(Debug, Clone)]
pub struct GetAttachmentsByOrderRow {
    pub id: i64,
    pub order_id: i64,
    pub filename: String,
    pub payload: Vec<u8>,
    pub description: Option<String>,
}

pub async fn get_attachments_by_order<'a>(
    session: &'a Session<'a>,
    order_id: i64,
) -> sibyl::Result<Vec<GetAttachmentsByOrderRow>> {
    let stmt = session.prepare("SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = :ORDER_ID ORDER BY id").await?;
    let rows = stmt.query((":ORDER_ID", order_id)).await?;
    let mut results = Vec::new();
    while let Some(row) = rows.next().await? {
        let id: i64 = row.get(0)?;
        let order_id: i64 = row.get(1)?;
        let filename: String = row.get(2)?;
        let payload_lob: BLOB<'_> = row.get(3)?;
        let payload_len = payload_lob.len().await?;
        let mut payload: Vec<u8> = Vec::new();
        let mut payload_read = 0usize;
        while payload_read < payload_len {
            let payload_n = payload_lob
                .read(payload_read, payload_len - payload_read, &mut payload)
                .await?;
            if payload_n == 0 {
                return Err(sibyl::Error::Interface(format!(
                    "incomplete LOB read for column 'payload': expected {} bytes, got {}",
                    payload_len, payload_read
                )));
            }
            payload_read += payload_n;
        }
        let description: Option<String> = match row.get::<Option<CLOB<'_>>, _>(4)? {
            Some(lob) => {
                let len = lob.len().await?;
                let mut buf = String::new();
                let mut read = 0usize;
                while read < len {
                    let n = lob.read(read, len - read, &mut buf).await?;
                    if n == 0 {
                        return Err(sibyl::Error::Interface(format!(
                            "incomplete LOB read for column 'description': expected {} characters, got {}",
                            len, read
                        )));
                    }
                    read += n;
                }
                Some(buf)
            }
            None => None,
        };
        results.push(GetAttachmentsByOrderRow {
            id: id,
            order_id: order_id,
            filename: filename,
            payload: payload,
            description: description,
        });
    }
    Ok(results)
}

#[derive(Debug, Clone)]
pub struct GetAttachmentByIdRow {
    pub id: i64,
    pub order_id: i64,
    pub filename: String,
    pub payload: Vec<u8>,
    pub description: Option<String>,
}

pub async fn get_attachment_by_id<'a>(
    session: &'a Session<'a>,
    id: i64,
) -> sibyl::Result<Option<GetAttachmentByIdRow>> {
    let stmt = session
        .prepare("SELECT id, order_id, filename, payload, description FROM attachments WHERE id = :ID")
        .await?;
    let rows = stmt.query((":ID", id)).await?;
    if let Some(row) = rows.next().await? {
        let id: i64 = row.get(0)?;
        let order_id: i64 = row.get(1)?;
        let filename: String = row.get(2)?;
        let payload_lob: BLOB<'_> = row.get(3)?;
        let payload_len = payload_lob.len().await?;
        let mut payload: Vec<u8> = Vec::new();
        let mut payload_read = 0usize;
        while payload_read < payload_len {
            let payload_n = payload_lob
                .read(payload_read, payload_len - payload_read, &mut payload)
                .await?;
            if payload_n == 0 {
                return Err(sibyl::Error::Interface(format!(
                    "incomplete LOB read for column 'payload': expected {} bytes, got {}",
                    payload_len, payload_read
                )));
            }
            payload_read += payload_n;
        }
        let description: Option<String> = match row.get::<Option<CLOB<'_>>, _>(4)? {
            Some(lob) => {
                let len = lob.len().await?;
                let mut buf = String::new();
                let mut read = 0usize;
                while read < len {
                    let n = lob.read(read, len - read, &mut buf).await?;
                    if n == 0 {
                        return Err(sibyl::Error::Interface(format!(
                            "incomplete LOB read for column 'description': expected {} characters, got {}",
                            len, read
                        )));
                    }
                    read += n;
                }
                Some(buf)
            }
            None => None,
        };
        Ok(Some(GetAttachmentByIdRow {
            id: id,
            order_id: order_id,
            filename: filename,
            payload: payload,
            description: description,
        }))
    } else {
        Ok(None)
    }
}

pub async fn delete_attachments_by_order<'a>(session: &'a Session<'a>, order_id: i64) -> sibyl::Result<usize> {
    let stmt = session
        .prepare("DELETE FROM attachments WHERE order_id = :ORDER_ID")
        .await?;
    let num_rows = stmt.execute((":ORDER_ID", order_id)).await?;
    Ok(num_rows)
}

#[derive(Debug, Clone)]
pub struct CreateOrderRow {
    pub id: i64,
    pub user_id: i64,
    pub total: f64,
    pub notes: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

pub async fn create_order<'a>(
    session: &'a Session<'a>,
    user_id: i64,
    total: f64,
    notes: Option<&str>,
) -> sibyl::Result<Option<CreateOrderRow>> {
    let stmt = session.prepare("INSERT INTO orders (user_id, total, notes) VALUES (:USER_ID, :TOTAL, :NOTES) RETURNING id, user_id, total, notes, created_at INTO :OUT_ID, :OUT_USER_ID, :OUT_TOTAL, :OUT_NOTES, :OUT_CREATED_AT").await?;
    let mut out_id: i64 = 0;
    let mut out_user_id: i64 = 0;
    let mut out_total: f64 = 0.0;
    let mut out_notes = String::with_capacity(4000);
    let mut out_created_at = Date::new(session);
    stmt.execute((
        (":USER_ID", user_id),
        (":TOTAL", total),
        (":NOTES", notes),
        (":OUT_ID", &mut out_id),
        (":OUT_USER_ID", &mut out_user_id),
        (":OUT_TOTAL", &mut out_total),
        (":OUT_NOTES", &mut out_notes),
        (":OUT_CREATED_AT", &mut out_created_at),
    ))
    .await?;
    let id = out_id;
    let user_id = out_user_id;
    let total = out_total;
    let notes = if stmt.is_null(":OUT_NOTES")? {
        None
    } else {
        Some(out_notes)
    };
    let created_at = {
        let (y, mo, d, h, mi, s) = out_created_at.date_and_time();
        chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32)
            .and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32))
            .expect("invalid date from Oracle")
    };
    Ok(Some(CreateOrderRow {
        id: id,
        user_id: user_id,
        total: total,
        notes: notes,
        created_at: created_at,
    }))
}

#[derive(Debug, Clone)]
pub struct GetOrdersByUserRow {
    pub id: i64,
    pub total: f64,
    pub notes: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

pub async fn get_orders_by_user<'a>(session: &'a Session<'a>, user_id: i64) -> sibyl::Result<Vec<GetOrdersByUserRow>> {
    let stmt = session
        .prepare("SELECT id, total, notes, created_at FROM orders WHERE user_id = :USER_ID ORDER BY created_at DESC")
        .await?;
    let rows = stmt.query((":USER_ID", user_id)).await?;
    let mut results = Vec::new();
    while let Some(row) = rows.next().await? {
        let id: i64 = row.get(0)?;
        let total: f64 = row.get(1)?;
        let notes: Option<String> = match row.get::<Option<CLOB<'_>>, _>(2)? {
            Some(lob) => {
                let len = lob.len().await?;
                let mut buf = String::new();
                let mut read = 0usize;
                while read < len {
                    let n = lob.read(read, len - read, &mut buf).await?;
                    if n == 0 {
                        return Err(sibyl::Error::Interface(format!(
                            "incomplete LOB read for column 'notes': expected {} characters, got {}",
                            len, read
                        )));
                    }
                    read += n;
                }
                Some(buf)
            }
            None => None,
        };
        let created_at_date: Date<'_> = row.get(3)?;
        let created_at: chrono::NaiveDateTime = {
            let (y, mo, d, h, mi, s) = created_at_date.date_and_time();
            chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32)
                .and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32))
                .expect("invalid date from Oracle")
        };
        results.push(GetOrdersByUserRow {
            id: id,
            total: total,
            notes: notes,
            created_at: created_at,
        });
    }
    Ok(results)
}

#[derive(Debug, Clone)]
pub struct GetOrderTotalRow {
    pub total_sum: Option<f64>,
}

pub async fn get_order_total<'a>(session: &'a Session<'a>, user_id: i64) -> sibyl::Result<Option<GetOrderTotalRow>> {
    let stmt = session
        .prepare("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :USER_ID")
        .await?;
    let rows = stmt.query((":USER_ID", user_id)).await?;
    if let Some(row) = rows.next().await? {
        let total_sum: Option<f64> = row.get(0)?;
        Ok(Some(GetOrderTotalRow { total_sum: total_sum }))
    } else {
        Ok(None)
    }
}

pub async fn delete_orders_by_user<'a>(session: &'a Session<'a>, user_id: i64) -> sibyl::Result<usize> {
    let stmt = session.prepare("DELETE FROM orders WHERE user_id = :USER_ID").await?;
    let num_rows = stmt.execute((":USER_ID", user_id)).await?;
    Ok(num_rows)
}

#[derive(Debug, Clone)]
pub struct GetUserByIdRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub active: i64,
    pub created_at: chrono::NaiveDateTime,
}

pub async fn get_user_by_id<'a>(session: &'a Session<'a>, id: i64) -> sibyl::Result<Option<GetUserByIdRow>> {
    let stmt = session
        .prepare("SELECT id, name, email, active, created_at FROM users WHERE id = :ID")
        .await?;
    let rows = stmt.query((":ID", id)).await?;
    if let Some(row) = rows.next().await? {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let email: Option<String> = row.get(2)?;
        let active: i64 = row.get(3)?;
        let created_at_date: Date<'_> = row.get(4)?;
        let created_at: chrono::NaiveDateTime = {
            let (y, mo, d, h, mi, s) = created_at_date.date_and_time();
            chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32)
                .and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32))
                .expect("invalid date from Oracle")
        };
        Ok(Some(GetUserByIdRow {
            id: id,
            name: name,
            email: email,
            active: active,
            created_at: created_at,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct ListActiveUsersRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
}

pub async fn list_active_users<'a>(session: &'a Session<'a>) -> sibyl::Result<Vec<ListActiveUsersRow>> {
    let stmt = session
        .prepare("SELECT id, name, email FROM users WHERE active = 1")
        .await?;
    let rows = stmt.query(()).await?;
    let mut results = Vec::new();
    while let Some(row) = rows.next().await? {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let email: Option<String> = row.get(2)?;
        results.push(ListActiveUsersRow {
            id: id,
            name: name,
            email: email,
        });
    }
    Ok(results)
}

#[derive(Debug, Clone)]
pub struct CreateUserRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub active: i64,
    pub created_at: chrono::NaiveDateTime,
}

pub async fn create_user<'a>(
    session: &'a Session<'a>,
    name: &str,
    email: Option<&str>,
    active: i64,
) -> sibyl::Result<Option<CreateUserRow>> {
    let stmt = session.prepare("INSERT INTO users (name, email, active) VALUES (:NAME, :EMAIL, :ACTIVE) RETURNING id, name, email, active, created_at INTO :OUT_ID, :OUT_NAME, :OUT_EMAIL, :OUT_ACTIVE, :OUT_CREATED_AT").await?;
    let mut out_id: i64 = 0;
    let mut out_name = String::with_capacity(4000);
    let mut out_email = String::with_capacity(4000);
    let mut out_active: i64 = 0;
    let mut out_created_at = Date::new(session);
    stmt.execute((
        (":NAME", name),
        (":EMAIL", email),
        (":ACTIVE", active),
        (":OUT_ID", &mut out_id),
        (":OUT_NAME", &mut out_name),
        (":OUT_EMAIL", &mut out_email),
        (":OUT_ACTIVE", &mut out_active),
        (":OUT_CREATED_AT", &mut out_created_at),
    ))
    .await?;
    let id = out_id;
    let name = out_name;
    let email = if stmt.is_null(":OUT_EMAIL")? {
        None
    } else {
        Some(out_email)
    };
    let active = out_active;
    let created_at = {
        let (y, mo, d, h, mi, s) = out_created_at.date_and_time();
        chrono::NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32)
            .and_then(|dt| dt.and_hms_opt(h as u32, mi as u32, s as u32))
            .expect("invalid date from Oracle")
    };
    Ok(Some(CreateUserRow {
        id: id,
        name: name,
        email: email,
        active: active,
        created_at: created_at,
    }))
}

pub async fn update_user_email<'a>(session: &'a Session<'a>, email: &str, id: i64) -> sibyl::Result<()> {
    let stmt = session
        .prepare("UPDATE users SET email = :EMAIL WHERE id = :ID")
        .await?;
    stmt.execute(((":EMAIL", email), (":ID", id))).await?;
    Ok(())
}

pub async fn delete_user<'a>(session: &'a Session<'a>, id: i64) -> sibyl::Result<()> {
    let stmt = session.prepare("DELETE FROM users WHERE id = :ID").await?;
    stmt.execute((":ID", id)).await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SearchUsersRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
}

pub async fn search_users<'a>(session: &'a Session<'a>, name: &str) -> sibyl::Result<Vec<SearchUsersRow>> {
    let stmt = session
        .prepare("SELECT id, name, email FROM users WHERE name LIKE :NAME")
        .await?;
    let rows = stmt.query((":NAME", name)).await?;
    let mut results = Vec::new();
    while let Some(row) = rows.next().await? {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let email: Option<String> = row.get(2)?;
        results.push(SearchUsersRow {
            id: id,
            name: name,
            email: email,
        });
    }
    Ok(results)
}
