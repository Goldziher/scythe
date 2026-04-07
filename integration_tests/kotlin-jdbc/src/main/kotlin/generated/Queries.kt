package generated

import java.sql.Connection
import java.sql.Types


enum class UserStatus(val value: String) {
    active("active"),
    inactive("inactive"),
    banned("banned");

    companion object {
        fun fromString(text: String): UserStatus = entries.first { it.value == text }
    }
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
    notes: String,
): CreateOrderRow? {
    conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at").use { ps ->
        ps.setInt(1, user_id)
        ps.setBigDecimal(2, total)
        ps.setString(3, notes)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                CreateOrderRow(
                    id = rs.getInt("id"),
                    user_id = rs.getInt("user_id"),
                    total = rs.getBigDecimal("total"),
                    notes = rs.getString("notes"),
                    created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
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
                result.add(
                    GetOrdersByUserRow(
                        id = rs.getInt("id"),
                        total = rs.getBigDecimal("total"),
                        notes = rs.getString("notes"),
                        created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
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
                GetOrderTotalRow(
                    total_sum = rs.getBigDecimal("total_sum"),
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
                GetUserByIdRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = rs.getString("email"),
                    status = UserStatus.fromString(rs.getString("status")),
                    created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
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
        ps.setObject(1, status.value, Types.OTHER)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<ListActiveUsersRow>()
            while (rs.next()) {
                result.add(
                    ListActiveUsersRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        email = rs.getString("email"),
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
    email: String,
    status: UserStatus,
): CreateUserRow? {
    conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email, status, created_at").use { ps ->
        ps.setString(1, name)
        ps.setString(2, email)
        ps.setObject(3, status.value, Types.OTHER)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                CreateUserRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = rs.getString("email"),
                    status = UserStatus.fromString(rs.getString("status")),
                    created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
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
        ps.setObject(1, status.value, Types.OTHER)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<GetUserOrdersRow>()
            while (rs.next()) {
                result.add(
                    GetUserOrdersRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        total = rs.getBigDecimal("total"),
                        notes = rs.getString("notes"),
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
        ps.setObject(1, status.value, Types.OTHER)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                CountUsersByStatusRow(
                    status = UserStatus.fromString(rs.getString("status")),
                    user_count = rs.getLong("user_count"),
                )
            } else {
                null
            }
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
                result.add(
                    SearchUsersRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        email = rs.getString("email"),
                    ),
                )
            }
            return result
        }
    }
}
