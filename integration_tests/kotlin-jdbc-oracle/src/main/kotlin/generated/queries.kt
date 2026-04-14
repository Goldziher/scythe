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
    val id: Long,
    val user_id: Long,
    val total: Long,
    val notes: String?,
    val created_at: java.time.LocalDateTime,
)


fun createOrder(
    conn: Connection,
    user_id: Long,
    total: Long,
    notes: String?,
): CreateOrderRow? {
    conn.prepareCall("BEGIN INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at INTO ?, ?, ?, ?, ?; END;").use { cs ->
        cs.setLong(1, user_id)
        cs.setLong(2, total)
        cs.setString(3, notes)
        cs.registerOutParameter(4, java.sql.Types.NUMERIC)
        cs.registerOutParameter(5, java.sql.Types.NUMERIC)
        cs.registerOutParameter(6, java.sql.Types.NUMERIC)
        cs.registerOutParameter(7, java.sql.Types.VARCHAR)
        cs.registerOutParameter(8, java.sql.Types.TIMESTAMP)
        cs.execute()
        return CreateOrderRow(
            id = cs.getLong(4),
            user_id = cs.getLong(5),
            total = cs.getLong(6),
            notes = cs.getString(7),
            created_at = cs.getObject(8, LocalDateTime::class.java),
        )
    }
}


data class GetOrdersByUserRow(
    val id: Long,
    val total: Long,
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
                        total = rs.getLong("total"),
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
    val total_sum: Long?,
)


fun getOrderTotal(
    conn: Connection,
    user_id: Long,
): GetOrderTotalRow? {
    conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setLong(1, user_id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val total_sumValue = rs.getLong("total_sum")
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
    return conn.prepareStatement("DELETE FROM orders WHERE user_id = ?").use { ps ->
        ps.setLong(1, user_id)
        ps.executeUpdate()
    }
}


data class GetUserByIdRow(
    val id: Long,
    val name: String,
    val email: String?,
    val active: Long,
    val created_at: java.time.LocalDateTime,
)


fun getUserById(
    conn: Connection,
    id: Long,
): GetUserByIdRow? {
    conn.prepareStatement("SELECT id, name, email, active, created_at FROM users WHERE id = ?").use { ps ->
        ps.setLong(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserByIdRow(
                    id = rs.getLong("id"),
                    name = rs.getString("name"),
                    email = email,
                    active = rs.getLong("active"),
                    created_at = rs.getObject("created_at", LocalDateTime::class.java),
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
    conn.prepareStatement("SELECT id, name, email FROM users WHERE active = 1").use { ps ->
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


data class CreateUserRow(
    val id: Long,
    val name: String,
    val email: String?,
    val active: Long,
    val created_at: java.time.LocalDateTime,
)


fun createUser(
    conn: Connection,
    name: String,
    email: String?,
    active: Long,
): CreateUserRow? {
    conn.prepareCall("BEGIN INSERT INTO users (name, email, active) VALUES (?, ?, ?) RETURNING id, name, email, active, created_at INTO ?, ?, ?, ?, ?; END;").use { cs ->
        cs.setString(1, name)
        cs.setString(2, email)
        cs.setLong(3, active)
        cs.registerOutParameter(4, java.sql.Types.NUMERIC)
        cs.registerOutParameter(5, java.sql.Types.VARCHAR)
        cs.registerOutParameter(6, java.sql.Types.VARCHAR)
        cs.registerOutParameter(7, java.sql.Types.NUMERIC)
        cs.registerOutParameter(8, java.sql.Types.TIMESTAMP)
        cs.execute()
        return CreateUserRow(
            id = cs.getLong(4),
            name = cs.getString(5),
            email = cs.getString(6),
            active = cs.getLong(7),
            created_at = cs.getObject(8, LocalDateTime::class.java),
        )
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
