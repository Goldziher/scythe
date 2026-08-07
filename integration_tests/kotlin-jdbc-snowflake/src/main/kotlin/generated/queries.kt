package generated

import java.math.BigDecimal
import java.sql.Connection
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.OffsetTime
import java.time.ZoneOffset


fun createOrder(
    conn: Connection,
    user_id: Long,
    total: java.math.BigDecimal,
    notes: String?,
) {
    conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)").use { ps ->
        ps.setLong(1, user_id)
        ps.setBigDecimal(2, total)
        ps.setString(3, notes)
        ps.executeUpdate()
    }
}


data class GetOrdersByUserRow(
    val id: Long,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.LocalDateTime,
)


fun getOrdersByUser(
    conn: Connection,
    user_id: Long,
): List<GetOrdersByUserRow> {
    conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC").use { ps ->
        ps.setLong(1, user_id)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<GetOrdersByUserRow>()
            while (rs.next()) {
                val notesValue = rs.getString("notes")
                val notes = if (rs.wasNull()) null else notesValue
                result.add(
                    GetOrdersByUserRow(
                        id = rs.getLong("id"),
                        total = rs.getBigDecimal("total"),
                        notes = notes,
                        created_at = rs.getTimestamp("created_at").toLocalDateTime(),
                    ),
                )
            }
            return result
        }
    }
}


data class GetOrderTotalRow(
    val total_sum: java.math.BigDecimal?,
)


fun getOrderTotal(
    conn: Connection,
    user_id: Long,
): GetOrderTotalRow? {
    conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setLong(1, user_id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val total_sumValue = rs.getBigDecimal("total_sum")
                val total_sum = if (rs.wasNull()) null else total_sumValue
                GetOrderTotalRow(
                    total_sum = total_sum,
                )
            } else {
                null
            }
        }
    }
}


fun deleteOrdersByUser(
    conn: Connection,
    user_id: Long,
): Int {
    return conn.prepareStatement("DELETE FROM orders WHERE id IN (SELECT id FROM orders WHERE user_id = ?)").use { ps ->
        ps.setLong(1, user_id)
        ps.executeUpdate()
    }
}


data class GetUserByIdRow(
    val id: Long,
    val name: String,
    val email: String?,
    val active: Boolean,
    val metadata: String?,
    val created_at: java.time.LocalDateTime,
    val updated_at: java.time.OffsetDateTime?,
)


fun getUserById(
    conn: Connection,
    id: Long,
): GetUserByIdRow? {
    conn.prepareStatement("SELECT id, name, email, active, metadata, created_at, updated_at FROM users WHERE id = ?").use { ps ->
        ps.setLong(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                val metadataValue = rs.getString("metadata")
                val metadata = if (rs.wasNull()) null else metadataValue
                val updated_atValue = rs.getTimestamp("updated_at")
                val updated_at = if (rs.wasNull()) null else updated_atValue.toInstant().atOffset(ZoneOffset.UTC)
                GetUserByIdRow(
                    id = rs.getLong("id"),
                    name = rs.getString("name"),
                    email = email,
                    active = rs.getBoolean("active"),
                    metadata = metadata,
                    created_at = rs.getTimestamp("created_at").toLocalDateTime(),
                    updated_at = updated_at,
                )
            } else {
                null
            }
        }
    }
}


data class ListActiveUsersRow(
    val id: Long,
    val name: String,
    val email: String?,
)


fun listActiveUsers(conn: Connection): List<ListActiveUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE active = TRUE").use { ps ->
        ps.executeQuery().use { rs ->
            val result = mutableListOf<ListActiveUsersRow>()
            while (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                result.add(
                    ListActiveUsersRow(
                        id = rs.getLong("id"),
                        name = rs.getString("name"),
                        email = email,
                    ),
                )
            }
            return result
        }
    }
}


fun createUser(
    conn: Connection,
    name: String,
    email: String?,
    active: Boolean,
) {
    conn.prepareStatement("INSERT INTO users (name, email, active) VALUES (?, ?, ?)").use { ps ->
        ps.setString(1, name)
        ps.setString(2, email)
        ps.setBoolean(3, active)
        ps.executeUpdate()
    }
}


fun updateUserEmail(
    conn: Connection,
    email: String,
    id: Long,
) {
    conn.prepareStatement("UPDATE users SET email = ?, updated_at = CURRENT_TIMESTAMP() WHERE id = ?").use { ps ->
        ps.setString(1, email)
        ps.setLong(2, id)
        ps.executeUpdate()
    }
}


fun deleteUser(
    conn: Connection,
    id: Long,
) {
    conn.prepareStatement("DELETE FROM users WHERE id = ?").use { ps ->
        ps.setLong(1, id)
        ps.executeUpdate()
    }
}


data class SearchUsersRow(
    val id: Long,
    val name: String,
    val email: String?,
)


fun searchUsers(
    conn: Connection,
    name: String,
): List<SearchUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE name LIKE ?").use { ps ->
        ps.setString(1, name)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<SearchUsersRow>()
            while (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                result.add(
                    SearchUsersRow(
                        id = rs.getLong("id"),
                        name = rs.getString("name"),
                        email = email,
                    ),
                )
            }
            return result
        }
    }
}

