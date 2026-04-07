import java.sql.Connection


fun createOrder(
    conn: Connection,
    user_id: Int,
    total: Double,
    notes: String,
) {
    conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)").use { ps ->
        ps.setInt(1, user_id)
        ps.setDouble(2, total)
        ps.setString(3, notes)
        ps.executeUpdate()
    }
}


data class GetOrdersByUserRow(
    val id: Int,
    val total: Double,
    val notes: String?,
    val created_at: String,
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
                        total = rs.getDouble("total"),
                        notes = rs.getString("notes"),
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
    user_id: Int,
): GetOrderTotalRow? {
    conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setInt(1, user_id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                GetOrderTotalRow(
                    total_sum = rs.getDouble("total_sum"),
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
    val status: String,
    val created_at: String,
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
                    status = rs.getString("status"),
                    created_at = rs.getString("created_at"),
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
    status: String,
): List<ListActiveUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?").use { ps ->
        ps.setString(1, status)
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


fun createUser(
    conn: Connection,
    name: String,
    email: String,
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
