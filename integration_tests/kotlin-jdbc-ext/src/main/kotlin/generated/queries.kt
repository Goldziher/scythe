// scythe:provenance v=0.16.1 backend=kotlin-jdbc engine=postgresql schema=sch2:c247390d575b8f71 queries=q1:b6aca93cc722fe32 options=opt1:b3ed1d9e36490c4f
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

    companion object {
        fun fromValue(value: String): UserStatus =
            values().firstOrNull { it.value == value }
                ?: throw IllegalArgumentException("Unknown UserStatus value: $value")
    }
}


data class CreateOrderRow(
    val id: Int,
    val user_id: Int,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.OffsetDateTime,
)


fun Connection.createOrder(
    user_id: Int,
    total: java.math.BigDecimal,
    notes: String?,
): CreateOrderRow =
    this.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at").use { ps ->
        ps.setInt(1, user_id)
        ps.setBigDecimal(2, total)
        ps.setString(3, notes)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
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
                throw NoSuchElementException("createOrder: no rows returned")
            }
        }
    }


data class GetOrdersByUserRow(
    val id: Int,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.OffsetDateTime,
)


fun Connection.getOrdersByUser(
    user_id: Int,
): List<GetOrdersByUserRow> =
    this.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC").use { ps ->
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
            result
        }
    }


data class GetOrderTotalRow(
    val total_sum: java.math.BigDecimal?,
)


fun Connection.getOrderTotal(
    user_id: Int,
): GetOrderTotalRow =
    this.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?").use { ps ->
        ps.setInt(1, user_id)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val total_sumValue = rs.getBigDecimal("total_sum")
                val total_sum = if (rs.wasNull()) null else total_sumValue
                GetOrderTotalRow(
                    total_sum = total_sum,
                )
            } else {
                throw NoSuchElementException("getOrderTotal: no rows returned")
            }
        }
    }


data class GetOrderWeightTotalRow(
    val weight_total: Double?,
)


fun Connection.getOrderWeightTotal(
    user_id: Int,
): GetOrderWeightTotalRow =
    this.prepareStatement("SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = ?").use { ps ->
        ps.setInt(1, user_id)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val weight_totalValue = rs.getDouble("weight_total")
                val weight_total = if (rs.wasNull()) null else weight_totalValue
                GetOrderWeightTotalRow(
                    weight_total = weight_total,
                )
            } else {
                throw NoSuchElementException("getOrderWeightTotal: no rows returned")
            }
        }
    }


fun Connection.deleteOrdersByUser(
    user_id: Int,
): Int =
    this.prepareStatement("DELETE FROM orders WHERE user_id = ?").use { ps ->
        ps.setInt(1, user_id)
        ps.executeUpdate()
    }


data class GetUserByIdRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


fun Connection.getUserById(
    id: Int,
): GetUserByIdRow =
    this.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserByIdRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UserStatus.fromValue(rs.getString("status")),
                    created_at = rs.getObject("created_at", OffsetDateTime::class.java),
                )
            } else {
                throw NoSuchElementException("getUserById: no rows returned")
            }
        }
    }


data class ListActiveUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


fun Connection.listActiveUsers(
    status: UserStatus,
): List<ListActiveUsersRow> =
    this.prepareStatement("SELECT id, name, email FROM users WHERE status = ?").use { ps ->
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
            result
        }
    }


data class CreateUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


fun Connection.createUser(
    name: String,
    email: String?,
    status: UserStatus,
): CreateUserRow =
    this.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email, status, created_at").use { ps ->
        ps.setString(1, name)
        ps.setString(2, email)
        ps.setObject(3, status.value, java.sql.Types.OTHER)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                CreateUserRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UserStatus.fromValue(rs.getString("status")),
                    created_at = rs.getObject("created_at", OffsetDateTime::class.java),
                )
            } else {
                throw NoSuchElementException("createUser: no rows returned")
            }
        }
    }


fun Connection.updateUserEmail(
    email: String,
    id: Int,
) {
    this.prepareStatement("UPDATE users SET email = ? WHERE id = ?").use { ps ->
        ps.setString(1, email)
        ps.setInt(2, id)
        ps.executeUpdate()
    }
}


fun Connection.deleteUser(
    id: Int,
) {
    this.prepareStatement("DELETE FROM users WHERE id = ?").use { ps ->
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


fun Connection.getUserOrders(
    status: UserStatus,
): List<GetUserOrdersRow> =
    this.prepareStatement("SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = ?").use { ps ->
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
            result
        }
    }


data class CountUsersByStatusRow(
    val status: UserStatus,
    val user_count: Long,
)


fun Connection.countUsersByStatus(
    status: UserStatus,
): CountUsersByStatusRow =
    this.prepareStatement("SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ?").use { ps ->
        ps.setObject(1, status.value, java.sql.Types.OTHER)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                CountUsersByStatusRow(
                    status = UserStatus.fromValue(rs.getString("status")),
                    user_count = rs.getLong("user_count"),
                )
            } else {
                throw NoSuchElementException("countUsersByStatus: no rows returned")
            }
        }
    }


data class GetUserWithTagsRow(
    val id: Int,
    val name: String,
    val tag_name: String,
)


fun Connection.getUserWithTags(
    id: Int,
): List<GetUserWithTagsRow> =
    this.prepareStatement("SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = ?").use { ps ->
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
            result
        }
    }


data class SearchUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


fun Connection.searchUsers(
    name: String,
): List<SearchUsersRow> =
    this.prepareStatement("SELECT id, name, email FROM users WHERE name LIKE ?").use { ps ->
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
            result
        }
    }


data class UserAddress(
    val street: String,
    val city: String,
    val zip: String,
) {
    fun toPgText(): String = listOf(street, city, zip).joinToString(",", "(", ")") { encodeCompositeField(it) }

    companion object {
        private fun encodeCompositeField(value: Any?): String {
            if (value == null) return ""
            val raw = value.toString()
            val quote = raw.isEmpty() || raw.any { it in charArrayOf('(', ')', ',', '"', '\\') } || raw != raw.trim()
            if (!quote) return raw
            return "\"" + raw.replace("\\", "\\\\").replace("\"", "\"\"") + "\""
        }

        /**
         * ~keep board #196: pgjdbc registers no `getObject(col, UserAddress::class.java)`
         * type map for this composite -- it throws `PSQLException: conversion to
         * class UserAddress` at runtime. Parse the driver's composite text form instead.
         */
        fun fromText(text: String?): UserAddress? {
            if (text == null) {
                return null
            }
            val f = parseCompositeFields(text)
            return UserAddress(
                f[0]!!,
                f[1]!!,
                f[2]!!,
            )
        }

        /**
         * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
         * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as `null`);
         * a field needing quoting (containing a comma, paren, quote, backslash, or
         * leading/trailing space, or the empty string) is wrapped in double quotes with `"` and
         * `\` backslash-escaped inside; every other field is unquoted and taken literally. A
         * nested composite's own "(x,y)" text form always contains parens, so it always comes
         * back quoted here, ready for that type's own `fromText` to parse recursively.
         */
        private fun parseCompositeFields(text: String): List<String?> {
            val fields = mutableListOf<String?>()
            val inner = text.substring(1, text.length - 1)
            var i = 0
            val n = inner.length
            while (true) {
                val field = StringBuilder()
                var isNull = false
                if (i < n && inner[i] == '"') {
                    i++
                    while (i < n) {
                        val c = inner[i]
                        if (c == '\\' && i + 1 < n) {
                            field.append(inner[i + 1])
                            i += 2
                        } else if (c == '"' && i + 1 < n && inner[i + 1] == '"') {
                            field.append('"')
                            i += 2
                        } else if (c == '"') {
                            i++
                            break
                        } else {
                            field.append(c)
                            i++
                        }
                    }
                } else {
                    val start = i
                    while (i < n && inner[i] != ',') {
                        i++
                    }
                    field.append(inner, start, i)
                    isNull = field.isEmpty()
                }
                fields.add(if (isNull) null else field.toString())
                if (i < n && inner[i] == ',') {
                    i++
                    continue
                }
                break
            }
            return fields
        }
    }
}


data class GetUserProfileRow(
    val id: Int,
    val secondary_status: UserStatus?,
    val address: UserAddress?,
)


fun Connection.getUserProfile(
    id: Int,
): GetUserProfileRow =
    this.prepareStatement("SELECT id, secondary_status, address FROM users WHERE id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val secondary_statusValue = rs.getString("secondary_status")
                val secondary_status = if (secondary_statusValue == null) null else UserStatus.fromValue(secondary_statusValue)
                GetUserProfileRow(
                    id = rs.getInt("id"),
                    secondary_status = secondary_status,
                    address = UserAddress.fromText(rs.getString("address")),
                )
            } else {
                throw NoSuchElementException("getUserProfile: no rows returned")
            }
        }
    }


data class RoundTripUserAddressRow(
    val address: UserAddress?,
)


fun Connection.roundTripUserAddress(
    address: UserAddress?,
): RoundTripUserAddressRow =
    this.prepareStatement("INSERT INTO users (name, status, address) VALUES ('Composite Parameter Round Trip', 'active', ?::text::user_address) RETURNING address").use { ps ->
        ps.setString(1, address?.toPgText())
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                RoundTripUserAddressRow(
                    address = UserAddress.fromText(rs.getString("address")),
                )
            } else {
                throw NoSuchElementException("roundTripUserAddress: no rows returned")
            }
        }
    }


data class GetUserAsJsonRow(
    val payload: String?,
)


fun Connection.getUserAsJson(
    id: Int,
): GetUserAsJsonRow =
    this.prepareStatement("SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val payloadValue = rs.getString("payload")
                val payload = if (rs.wasNull()) null else payloadValue
                GetUserAsJsonRow(
                    payload = payload,
                )
            } else {
                throw NoSuchElementException("getUserAsJson: no rows returned")
            }
        }
    }


data class GetUsersAsJsonRow(
    val payload: String?,
)


fun Connection.getUsersAsJson(): GetUsersAsJsonRow =
    this.prepareStatement("SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u").use { ps ->
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val payloadValue = rs.getString("payload")
                val payload = if (rs.wasNull()) null else payloadValue
                GetUsersAsJsonRow(
                    payload = payload,
                )
            } else {
                throw NoSuchElementException("getUsersAsJson: no rows returned")
            }
        }
    }


data class GetUserOrdersAsJsonRow(
    val payload: String?,
)


fun Connection.getUserOrdersAsJson(
    id: Int,
): GetUserOrdersAsJsonRow =
    this.prepareStatement("SELECT json_agg(o.* ORDER BY o.id) AS payload FROM users u LEFT JOIN orders o ON o.user_id = u.id WHERE u.id = ? GROUP BY u.id").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            if (rs.next()) {
                val payloadValue = rs.getString("payload")
                val payload = if (rs.wasNull()) null else payloadValue
                GetUserOrdersAsJsonRow(
                    payload = payload,
                )
            } else {
                throw NoSuchElementException("getUserOrdersAsJson: no rows returned")
            }
        }
    }

