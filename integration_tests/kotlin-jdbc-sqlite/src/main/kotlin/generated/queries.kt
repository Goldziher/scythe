// scythe:provenance v=0.16.0 backend=kotlin-jdbc engine=sqlite schema=sch1:588fb635332179bc queries=q1:f7199f36438b6396 options=opt1:cbf29ce484222325
package generated

import java.math.BigDecimal
import java.sql.Connection
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.OffsetTime


fun createOrder(
    conn: Connection,
    user_id: Long,
    total: Double,
    notes: String?,
) {
    conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)").use { ps ->
        ps.setLong(1, user_id)
        ps.setDouble(2, total)
        ps.setString(3, notes)
        ps.executeUpdate()
    }
}


data class GetOrdersByUserRow(
    val id: Long,
    val total: Double,
    val notes: String?,
    val created_at: String,
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
                        total = rs.getDouble("total"),
                        notes = notes,
                        created_at = rs.getString("created_at"),
                    ),
                )
            }
            return result
        }
    }
}


data class GetOrderTotalRow(
    val total_sum: Double?,
)


fun getOrderTotal(
    conn: Connection,
    user_id: Long,
): GetOrderTotalRow {
    conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setLong(1, user_id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val total_sumValue = rs.getDouble("total_sum")
                val total_sum = if (rs.wasNull()) null else total_sumValue
                GetOrderTotalRow(
                    total_sum = total_sum,
                )
            } else {
                throw NoSuchElementException("getOrderTotal: no rows returned")
            }
        }
    }
}


fun deleteOrdersByUser(
    conn: Connection,
    user_id: Long,
): Int {
    return conn.prepareStatement("DELETE FROM orders WHERE user_id = ?").use { ps ->
        ps.setLong(1, user_id)
        ps.executeUpdate()
    }
}


data class GetUserByIdRow(
    val id: Long,
    val name: String,
    val email: String?,
    val status: String,
    val created_at: String,
)


fun getUserById(
    conn: Connection,
    id: Long,
): GetUserByIdRow {
    conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?").use { ps ->
        ps.setLong(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserByIdRow(
                    id = rs.getLong("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = rs.getString("status"),
                    created_at = rs.getString("created_at"),
                )
            } else {
                throw NoSuchElementException("getUserById: no rows returned")
            }
        }
    }
}


data class ListActiveUsersRow(
    val id: Long,
    val name: String,
    val email: String?,
)


fun listActiveUsers(
    conn: Connection,
    status: String,
): List<ListActiveUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?").use { ps ->
        ps.setString(1, status)
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
    status: String,
) {
    conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?)").use { ps ->
        ps.setString(1, name)
        ps.setString(2, email)
        ps.setString(3, status)
        ps.executeUpdate()
    }
}


fun updateUserEmail(
    conn: Connection,
    email: String,
    id: Long,
) {
    conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?").use { ps ->
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

