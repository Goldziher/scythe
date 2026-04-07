```kotlin title="Kotlin (JDBC)"
import java.sql.Connection

enum class UserStatus(val value: String) {
    active("active"),
    inactive("inactive"),
    banned("banned");
}

data class GetUserByIdRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)

fun getUserById(conn: Connection, id: Int): GetUserByIdRow? {
    conn.prepareStatement(
        "SELECT id, name, email, status, created_at " +
        "FROM users WHERE id = ?"
    ).use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                GetUserByIdRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = rs.getString("email"),
                    status = rs.getObject("status"),
                    created_at = rs.getObject("created_at"),
                )
            } else null
        }
    }
}

data class ListActiveUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)

fun listActiveUsers(
    conn: Connection, status: UserStatus
): List<ListActiveUsersRow> {
    conn.prepareStatement(
        "SELECT id, name, email FROM users " +
        "WHERE status = ?"
    ).use { ps ->
        ps.setObject(1, status)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<ListActiveUsersRow>()
            while (rs.next()) {
                result.add(ListActiveUsersRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = rs.getString("email"),
                ))
            }
            return result
        }
    }
}

fun updateUserEmail(
    conn: Connection, email: String, id: Int
) {
    conn.prepareStatement(
        "UPDATE users SET email = ? WHERE id = ?"
    ).use { ps ->
        ps.setString(1, email)
        ps.setInt(2, id)
        ps.executeUpdate()
    }
}
```
