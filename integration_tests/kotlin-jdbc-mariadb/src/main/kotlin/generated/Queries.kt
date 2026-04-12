package generated

import java.math.BigDecimal
import java.sql.Connection
import java.time.LocalDateTime
import java.util.UUID


enum class UsersStatus(val value: String) {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");
}


data class CreateOrderRow(
    val id: Int,
    val user_id: UUID,
    val total: BigDecimal,
    val notes: String?,
    val created_at: LocalDateTime,
)


fun createOrder(
    conn: Connection,
    user_id: UUID,
    total: BigDecimal,
    notes: String?,
): CreateOrderRow? {
    conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at").use { ps ->
        ps.setString(1, user_id.toString())
        ps.setBigDecimal(2, total)
        ps.setString(3, notes)
        ps.execute()
        val rs = ps.resultSet ?: return null
        return if (rs.next()) {
            val notesValue = rs.getString("notes")
            CreateOrderRow(
                id = rs.getInt("id"),
                user_id = UUID.fromString(rs.getString("user_id")),
                total = rs.getBigDecimal("total"),
                notes = if (rs.wasNull()) null else notesValue,
                created_at = rs.getObject("created_at", LocalDateTime::class.java),
            )
        } else {
            null
        }
    }
}


data class GetOrdersByUserRow(
    val id: Int,
    val total: BigDecimal,
    val notes: String?,
    val created_at: LocalDateTime,
)


fun getOrdersByUser(
    conn: Connection,
    user_id: UUID,
): List<GetOrdersByUserRow> {
    conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC").use { ps ->
        ps.setString(1, user_id.toString())
        ps.executeQuery().use { rs ->
            val result = mutableListOf<GetOrdersByUserRow>()
            while (rs.next()) {
                val notesValue = rs.getString("notes")
                result.add(
                    GetOrdersByUserRow(
                        id = rs.getInt("id"),
                        total = rs.getBigDecimal("total"),
                        notes = if (rs.wasNull()) null else notesValue,
                        created_at = rs.getObject("created_at", LocalDateTime::class.java),
                    ),
                )
            }
            return result
        }
    }
}


data class GetOrderTotalRow(
    val total_sum: BigDecimal?,
)


fun getOrderTotal(
    conn: Connection,
    user_id: UUID,
): GetOrderTotalRow? {
    conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setString(1, user_id.toString())
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val total_sumValue = rs.getBigDecimal("total_sum")
                GetOrderTotalRow(
                    total_sum = if (rs.wasNull()) null else total_sumValue,
                )
            } else {
                null
            }
        }
    }
}


fun deleteOrdersByUser(
    conn: Connection,
    user_id: UUID,
): Int {
    return conn.prepareStatement("DELETE FROM orders WHERE user_id = ?").use { ps ->
        ps.setString(1, user_id.toString())
        ps.executeUpdate()
    }
}


data class GetUserByIdRow(
    val id: UUID,
    val name: String,
    val email: String?,
    val status: UsersStatus,
    val created_at: LocalDateTime,
)


fun getUserById(
    conn: Connection,
    id: UUID,
): GetUserByIdRow? {
    conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?").use { ps ->
        ps.setString(1, id.toString())
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                GetUserByIdRow(
                    id = UUID.fromString(rs.getString("id")),
                    name = rs.getString("name"),
                    email = if (rs.wasNull()) null else emailValue,
                    status = UsersStatus.entries.first { it.value == rs.getString("status") },
                    created_at = rs.getObject("created_at", LocalDateTime::class.java),
                )
            } else {
                null
            }
        }
    }
}


data class ListActiveUsersRow(
    val id: UUID,
    val name: String,
    val email: String?,
)


fun listActiveUsers(
    conn: Connection,
    status: UsersStatus,
): List<ListActiveUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?").use { ps ->
        ps.setString(1, status.value)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<ListActiveUsersRow>()
            while (rs.next()) {
                val emailValue = rs.getString("email")
                result.add(
                    ListActiveUsersRow(
                        id = UUID.fromString(rs.getString("id")),
                        name = rs.getString("name"),
                        email = if (rs.wasNull()) null else emailValue,
                    ),
                )
            }
            return result
        }
    }
}


data class CreateUserRow(
    val id: UUID,
    val name: String,
    val email: String?,
)


fun createUser(
    conn: Connection,
    name: String,
    email: String?,
    status: UsersStatus,
): CreateUserRow? {
    conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email").use { ps ->
        ps.setString(1, name)
        ps.setString(2, email)
        ps.setString(3, status.value)
        ps.execute()
        val rs = ps.resultSet ?: return null
        return if (rs.next()) {
            val emailValue = rs.getString("email")
            CreateUserRow(
                id = UUID.fromString(rs.getString("id")),
                name = rs.getString("name"),
                email = if (rs.wasNull()) null else emailValue,
            )
        } else {
            null
        }
    }
}


fun updateUserEmail(
    conn: Connection,
    email: String,
    id: UUID,
) {
    conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?").use { ps ->
        ps.setString(1, email)
        ps.setString(2, id.toString())
        ps.executeUpdate()
    }
}


fun deleteUser(
    conn: Connection,
    id: UUID,
) {
    conn.prepareStatement("DELETE FROM users WHERE id = ? RETURNING id").use { ps ->
        ps.setString(1, id.toString())
        ps.execute()
    }
}


data class SearchUsersRow(
    val id: UUID,
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
                result.add(
                    SearchUsersRow(
                        id = UUID.fromString(rs.getString("id")),
                        name = rs.getString("name"),
                        email = if (rs.wasNull()) null else emailValue,
                    ),
                )
            }
            return result
        }
    }
}
