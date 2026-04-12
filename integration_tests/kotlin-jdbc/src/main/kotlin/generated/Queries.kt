package generated

import java.math.BigDecimal
import java.sql.Connection
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.OffsetTime
import java.util.UUID


enum class UserStatus(val value: String) {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");
}


data class CreateOrderRow(
    val id: Int,
    val user_id: Int,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.OffsetDateTime,
)


fun createOrder(
    conn: Connection,
    user_id: Int,
    total: java.math.BigDecimal,
    notes: String?,
): CreateOrderRow? {
    conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at").use { ps ->
        ps.setInt(1, user_id)
        ps.setBigDecimal(2, total)
        ps.setString(3, notes)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val notesValue = rs.getString("notes")
                val notes = if (rs.wasNull()) null else notesValue
                CreateOrderRow(
                    id = rs.getInt("id"),
                    user_id = rs.getInt("user_id"),
                    total = rs.getBigDecimal("total"),
                    notes = notes,
                    created_at = rs.getObject("created_at", OffsetDateTime::class.java),
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
    val created_at: java.time.OffsetDateTime,
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
                        created_at = rs.getObject("created_at", OffsetDateTime::class.java),
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
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


fun getUserById(
    conn: Connection,
    id: Int,
): GetUserByIdRow? {
    conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserByIdRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UserStatus.valueOf(rs.getString("status").uppercase()),
                    created_at = rs.getObject("created_at", OffsetDateTime::class.java),
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


fun listActiveUsers(
    conn: Connection,
    status: UserStatus,
): List<ListActiveUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?").use { ps ->
        ps.setObject(1, status.value, java.sql.Types.OTHER)
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
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


fun createUser(
    conn: Connection,
    name: String,
    email: String?,
    status: UserStatus,
): CreateUserRow? {
    conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email, status, created_at").use { ps ->
        ps.setString(1, name)
        ps.setString(2, email)
        ps.setObject(3, status.value, java.sql.Types.OTHER)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                CreateUserRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UserStatus.valueOf(rs.getString("status").uppercase()),
                    created_at = rs.getObject("created_at", OffsetDateTime::class.java),
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


data class GetUserOrdersRow(
    val id: Int,
    val name: String,
    val total: java.math.BigDecimal?,
    val notes: String?,
)


fun getUserOrders(
    conn: Connection,
    status: UserStatus,
): List<GetUserOrdersRow> {
    conn.prepareStatement("SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = ?").use { ps ->
        ps.setObject(1, status.value, java.sql.Types.OTHER)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<GetUserOrdersRow>()
            while (rs.next()) {
                val totalValue = rs.getBigDecimal("total")
                val total = if (rs.wasNull()) null else totalValue
                val notesValue = rs.getString("notes")
                val notes = if (rs.wasNull()) null else notesValue
                result.add(
                    GetUserOrdersRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        total = total,
                        notes = notes,
                    ),
                )
            }
            return result
        }
    }
}


data class CountUsersByStatusRow(
    val status: UserStatus,
    val user_count: Long,
)


fun countUsersByStatus(
    conn: Connection,
    status: UserStatus,
): CountUsersByStatusRow? {
    conn.prepareStatement("SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ?").use { ps ->
        ps.setObject(1, status.value, java.sql.Types.OTHER)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                CountUsersByStatusRow(
                    status = UserStatus.valueOf(rs.getString("status").uppercase()),
                    user_count = rs.getLong("user_count"),
                )
            } else {
                null
            }
        }
    }
}


data class GetUserWithTagsRow(
    val id: Int,
    val name: String,
    val tag_name: String,
)


fun getUserWithTags(
    conn: Connection,
    id: Int,
): List<GetUserWithTagsRow> {
    conn.prepareStatement("SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<GetUserWithTagsRow>()
            while (rs.next()) {
                result.add(
                    GetUserWithTagsRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        tag_name = rs.getString("tag_name"),
                    ),
                )
            }
            return result
        }
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
