package generated

import java.math.BigDecimal
import java.sql.Connection
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.OffsetTime
import java.util.UUID


data class CreateOrderRow(
    val id: Int,
    val user_id: Int,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.LocalDateTime,
)


fun createOrder(
    conn: Connection,
    id: Int,
    user_id: Int,
    total: java.math.BigDecimal,
    notes: String?,
): CreateOrderRow? {
    conn.prepareStatement("INSERT INTO orders (id, user_id, total, notes) OUTPUT INSERTED.id, INSERTED.user_id, INSERTED.total, INSERTED.notes, INSERTED.created_at VALUES (?, ?, ?, ?)").use { ps ->
        ps.setInt(1, id)
        ps.setInt(2, user_id)
        ps.setBigDecimal(3, total)
        ps.setString(4, notes)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val notesValue = rs.getString("notes")
                val notes = if (rs.wasNull()) null else notesValue
                CreateOrderRow(
                    id = rs.getInt("id"),
                    user_id = rs.getInt("user_id"),
                    total = rs.getBigDecimal("total"),
                    notes = notes,
                    created_at = rs.getObject("created_at", LocalDateTime::class.java),
                )
            } else {
                null
            }
        }
    }
}


data class GetOrdersByUserRow(
    val id: Int,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.LocalDateTime,
)


fun getOrdersByUser(
    conn: Connection,
    user_id: Int,
): List<GetOrdersByUserRow> {
    conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC").use { ps ->
        ps.setInt(1, user_id)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<GetOrdersByUserRow>()
            while (rs.next()) {
                val notesValue = rs.getString("notes")
                val notes = if (rs.wasNull()) null else notesValue
                result.add(
                    GetOrdersByUserRow(
                        id = rs.getInt("id"),
                        total = rs.getBigDecimal("total"),
                        notes = notes,
                        created_at = rs.getObject("created_at", LocalDateTime::class.java),
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
    user_id: Int,
): GetOrderTotalRow? {
    conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setInt(1, user_id)
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
    user_id: Int,
): Int {
    return conn.prepareStatement("DELETE FROM orders WHERE user_id = ?").use { ps ->
        ps.setInt(1, user_id)
        ps.executeUpdate()
    }
}


data class GetUserByIdRow(
    val id: Int,
    val name: String,
    val email: String?,
    val active: Boolean,
    val external_id: java.util.UUID?,
    val created_at: java.time.LocalDateTime,
)


fun getUserById(
    conn: Connection,
    id: Int,
): GetUserByIdRow? {
    conn.prepareStatement("SELECT id, name, email, active, external_id, created_at FROM users WHERE id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                val external_idValue = rs.getObject("external_id")
                val external_id = if (rs.wasNull()) null else external_idValue
                GetUserByIdRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    active = rs.getBoolean("active"),
                    external_id = external_id,
                    created_at = rs.getObject("created_at", LocalDateTime::class.java),
                )
            } else {
                null
            }
        }
    }
}


data class ListActiveUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


fun listActiveUsers(conn: Connection): List<ListActiveUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE active = CAST(1 AS BIT)").use { ps ->
        ps.executeQuery().use { rs ->
            val result = mutableListOf<ListActiveUsersRow>()
            while (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                result.add(
                    ListActiveUsersRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        email = email,
                    ),
                )
            }
            return result
        }
    }
}


data class CreateUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val active: Boolean,
    val created_at: java.time.LocalDateTime,
)


fun createUser(
    conn: Connection,
    id: Int,
    name: String,
    email: String?,
    active: Boolean,
): CreateUserRow? {
    conn.prepareStatement("INSERT INTO users (id, name, email, active) OUTPUT INSERTED.id, INSERTED.name, INSERTED.email, INSERTED.active, INSERTED.created_at VALUES (?, ?, ?, ?)").use { ps ->
        ps.setInt(1, id)
        ps.setString(2, name)
        ps.setString(3, email)
        ps.setBoolean(4, active)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                CreateUserRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    active = rs.getBoolean("active"),
                    created_at = rs.getObject("created_at", LocalDateTime::class.java),
                )
            } else {
                null
            }
        }
    }
}


fun updateUserEmail(
    conn: Connection,
    email: String,
    id: Int,
) {
    conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?").use { ps ->
        ps.setString(1, email)
        ps.setInt(2, id)
        ps.executeUpdate()
    }
}


fun deleteUser(
    conn: Connection,
    id: Int,
) {
    conn.prepareStatement("DELETE FROM users WHERE id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeUpdate()
    }
}


data class SearchUsersRow(
    val id: Int,
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
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        email = email,
                    ),
                )
            }
            return result
        }
    }
}
