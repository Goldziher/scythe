package generated

import java.math.BigDecimal
import java.sql.Connection
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.OffsetTime


enum class UsersStatus(val value: String) {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");
}


data class CreateOrderRow(
    val id: Int,
    val user_id: String,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.LocalDateTime,
)


fun createOrder(
    conn: Connection,
    user_id: String,
    total: java.math.BigDecimal,
    notes: String?,
): CreateOrderRow? {
    conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at").use { ps ->
        ps.setString(1, user_id)
        ps.setBigDecimal(2, total)
        ps.setString(3, notes)
        ps.execute()
        val rs = ps.resultSet
        if (rs != null && rs.next()) {
            return CreateOrderRow(
                id = rs.getInt("id"),
                user_id = rs.getString("user_id"),
                total = rs.getBigDecimal("total"),
                notes = rs.getString("notes"),
                created_at = rs.getObject("created_at", LocalDateTime::class.java),
            )
        }
        return null
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
    user_id: String,
): List<GetOrdersByUserRow> {
    conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC").use { ps ->
        ps.setString(1, user_id)
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
    user_id: String,
): GetOrderTotalRow? {
    conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setString(1, user_id)
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
    user_id: String,
): Int {
    return conn.prepareStatement("DELETE FROM orders WHERE user_id = ?").use { ps ->
        ps.setString(1, user_id)
        ps.executeUpdate()
    }
}


data class GetUserByIdRow(
    val id: String,
    val name: String,
    val email: String?,
    val status: UsersStatus,
    val created_at: java.time.LocalDateTime,
)


fun getUserById(
    conn: Connection,
    id: String,
): GetUserByIdRow? {
    conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?").use { ps ->
        ps.setString(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserByIdRow(
                    id = rs.getString("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UsersStatus.valueOf(rs.getString("status").uppercase()),
                    created_at = rs.getObject("created_at", LocalDateTime::class.java),
                )
            } else {
                null
            }
        }
    }
}


data class ListActiveUsersRow(
    val id: String,
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
                val email = if (rs.wasNull()) null else emailValue
                result.add(
                    ListActiveUsersRow(
                        id = rs.getString("id"),
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
    val id: String,
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
        val rs = ps.resultSet
        if (rs != null && rs.next()) {
            return CreateUserRow(
                id = rs.getString("id"),
                name = rs.getString("name"),
                email = rs.getString("email"),
            )
        }
        return null
    }
}


fun updateUserEmail(
    conn: Connection,
    email: String,
    id: String,
) {
    conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?").use { ps ->
        ps.setString(1, email)
        ps.setString(2, id)
        ps.executeUpdate()
    }
}


fun deleteUser(
    conn: Connection,
    id: String,
) {
    conn.prepareStatement("DELETE FROM users WHERE id = ? RETURNING id").use { ps ->
        ps.setString(1, id)
        ps.executeUpdate()
    }
}


data class SearchUsersRow(
    val id: String,
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
                        id = rs.getString("id"),
                        name = rs.getString("name"),
                        email = email,
                    ),
                )
            }
            return result
        }
    }
}

