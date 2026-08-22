// scythe:provenance v=0.18.1 backend=kotlin-exposed engine=postgresql schema=sch2:59e0edaa3ac94824 queries=q1:861cdfc5df3ece62 options=opt1:cbf29ce484222325
package generated

import org.jetbrains.exposed.dao.id.IntIdTable
import org.jetbrains.exposed.dao.id.LongIdTable
import org.jetbrains.exposed.dao.id.UUIDTable
import org.jetbrains.exposed.sql.BinaryColumnType
import org.jetbrains.exposed.sql.BooleanColumnType
import org.jetbrains.exposed.sql.ByteColumnType
import org.jetbrains.exposed.sql.DecimalColumnType
import org.jetbrains.exposed.sql.IColumnType
import org.jetbrains.exposed.sql.DoubleColumnType
import org.jetbrains.exposed.sql.FloatColumnType
import org.jetbrains.exposed.sql.IntegerColumnType
import org.jetbrains.exposed.sql.LongColumnType
import org.jetbrains.exposed.sql.ShortColumnType
import org.jetbrains.exposed.sql.TextColumnType
import org.jetbrains.exposed.sql.UUIDColumnType
import org.jetbrains.exposed.sql.javatime.JavaLocalDateColumnType
import org.jetbrains.exposed.sql.javatime.JavaLocalDateTimeColumnType
import org.jetbrains.exposed.sql.javatime.JavaLocalTimeColumnType
import org.jetbrains.exposed.sql.javatime.JavaOffsetDateTimeColumnType
import org.jetbrains.exposed.sql.statements.StatementType
import org.jetbrains.exposed.sql.transactions.transaction


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


fun createOrder(user_id: Int, total: java.math.BigDecimal, notes: String?): CreateOrderRow =
    transaction {
        exec("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to user_id, DecimalColumnType(10, 2) to total, TextColumnType() to notes), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val notesValue = rs.getString("notes")
                val notes = if (rs.wasNull()) null else notesValue
                CreateOrderRow(
                    id = rs.getInt("id"),
                    user_id = rs.getInt("user_id"),
                    total = rs.getBigDecimal("total"),
                    notes = notes,
                    created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("createOrder: no rows returned")
    }


data class GetOrdersByUserRow(
    val id: Int,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.OffsetDateTime,
)


fun getOrdersByUser(user_id: Int): List<GetOrdersByUserRow> =
    transaction {
        val result = mutableListOf<GetOrdersByUserRow>()
        exec("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to user_id), explicitStatementType = StatementType.SELECT) { rs ->
            while (rs.next()) {
                val notesValue = rs.getString("notes")
                val notes = if (rs.wasNull()) null else notesValue
                result.add(
                    GetOrdersByUserRow(
                        id = rs.getInt("id"),
                        total = rs.getBigDecimal("total"),
                        notes = notes,
                        created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
                    ),
                )
            }
        }
        result
    }


data class GetOrderTotalRow(
    val total_sum: java.math.BigDecimal?,
)


fun getOrderTotal(user_id: Int): GetOrderTotalRow =
    transaction {
        exec("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to user_id), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val total_sumValue = rs.getBigDecimal("total_sum")
                val total_sum = if (rs.wasNull()) null else total_sumValue
                GetOrderTotalRow(
                    total_sum = total_sum,
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("getOrderTotal: no rows returned")
    }


data class GetOrderWeightTotalRow(
    val weight_total: Double?,
)


fun getOrderWeightTotal(user_id: Int): GetOrderWeightTotalRow =
    transaction {
        exec("SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = ?", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to user_id), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val weight_totalValue = rs.getDouble("weight_total")
                val weight_total = if (rs.wasNull()) null else weight_totalValue
                GetOrderWeightTotalRow(
                    weight_total = weight_total,
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("getOrderWeightTotal: no rows returned")
    }


fun deleteOrdersByUser(user_id: Int): Int =
    transaction {
        val stmt = connection.prepareStatement("DELETE FROM orders WHERE user_id = ?", false)
        stmt.fillParameters(listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to user_id))
        stmt.executeUpdate()
    }


data class GetUserByIdRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


fun getUserById(id: Int): GetUserByIdRow =
    transaction {
        exec("SELECT id, name, email, status, created_at FROM users WHERE id = ?", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to id), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserByIdRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UserStatus.fromValue(rs.getString("status")),
                    created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("getUserById: no rows returned")
    }


data class ListActiveUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


fun listActiveUsers(status: UserStatus): List<ListActiveUsersRow> =
    transaction {
        val result = mutableListOf<ListActiveUsersRow>()
        exec("SELECT id, name, email FROM users WHERE status = ?::user_status", listOf<Pair<IColumnType<*>, Any?>>(TextColumnType() to status?.value), explicitStatementType = StatementType.SELECT) { rs ->
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
        }
        result
    }


data class CreateUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


fun createUser(name: String, email: String?, status: UserStatus): CreateUserRow =
    transaction {
        exec("INSERT INTO users (name, email, status) VALUES (?, ?, ?::user_status) RETURNING id, name, email, status, created_at", listOf<Pair<IColumnType<*>, Any?>>(TextColumnType() to name, TextColumnType() to email, TextColumnType() to status?.value), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                CreateUserRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UserStatus.fromValue(rs.getString("status")),
                    created_at = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("createUser: no rows returned")
    }


fun updateUserEmail(email: String, id: Int) =
    transaction {
        exec("UPDATE users SET email = ? WHERE id = ?", listOf<Pair<IColumnType<*>, Any?>>(TextColumnType() to email, IntegerColumnType() to id))
    }


fun deleteUser(id: Int) =
    transaction {
        exec("DELETE FROM users WHERE id = ?", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to id))
    }


data class GetUserOrdersRow(
    val id: Int,
    val name: String,
    val total: java.math.BigDecimal?,
    val notes: String?,
)


fun getUserOrders(status: UserStatus): List<GetUserOrdersRow> =
    transaction {
        val result = mutableListOf<GetUserOrdersRow>()
        exec("SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = ?::user_status", listOf<Pair<IColumnType<*>, Any?>>(TextColumnType() to status?.value), explicitStatementType = StatementType.SELECT) { rs ->
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
        }
        result
    }


data class CountUsersByStatusRow(
    val status: UserStatus,
    val user_count: Long,
)


fun countUsersByStatus(status: UserStatus): CountUsersByStatusRow =
    transaction {
        exec("SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ?::user_status", listOf<Pair<IColumnType<*>, Any?>>(TextColumnType() to status?.value), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                CountUsersByStatusRow(
                    status = UserStatus.fromValue(rs.getString("status")),
                    user_count = rs.getLong("user_count"),
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("countUsersByStatus: no rows returned")
    }


data class GetUserWithTagsRow(
    val id: Int,
    val name: String,
    val tag_name: String,
)


fun getUserWithTags(id: Int): List<GetUserWithTagsRow> =
    transaction {
        val result = mutableListOf<GetUserWithTagsRow>()
        exec("SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = ?", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to id), explicitStatementType = StatementType.SELECT) { rs ->
            while (rs.next()) {
                result.add(
                    GetUserWithTagsRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        tag_name = rs.getString("tag_name"),
                    ),
                )
            }
        }
        result
    }


data class SearchUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


fun searchUsers(name: String): List<SearchUsersRow> =
    transaction {
        val result = mutableListOf<SearchUsersRow>()
        exec("SELECT id, name, email FROM users WHERE name LIKE ?", listOf<Pair<IColumnType<*>, Any?>>(TextColumnType() to name), explicitStatementType = StatementType.SELECT) { rs ->
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
        }
        result
    }


data class UserAddress(
    val street: String?,
    val city: String?,
    val zip: String?,
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
                f[0]?.let { value -> value },
                f[1]?.let { value -> value },
                f[2]?.let { value -> value },
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


fun getUserProfile(id: Int): GetUserProfileRow =
    transaction {
        exec("SELECT id, secondary_status, address FROM users WHERE id = ?", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to id), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val secondary_statusValue = rs.getString("secondary_status")
                val secondary_status = if (secondary_statusValue == null) null else UserStatus.fromValue(secondary_statusValue)
                GetUserProfileRow(
                    id = rs.getInt("id"),
                    secondary_status = secondary_status,
                    address = UserAddress.fromText(rs.getString("address")),
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("getUserProfile: no rows returned")
    }


data class RoundTripUserAddressRow(
    val address: UserAddress?,
)


fun roundTripUserAddress(address: UserAddress?): RoundTripUserAddressRow =
    transaction {
        exec("INSERT INTO users (name, status, address) VALUES ('Composite Parameter Round Trip', 'active', (?::text::user_address)) RETURNING address", listOf<Pair<IColumnType<*>, Any?>>(TextColumnType() to address?.toPgText()), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                RoundTripUserAddressRow(
                    address = UserAddress.fromText(rs.getString("address")),
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("roundTripUserAddress: no rows returned")
    }


data class GetUserAsJsonRow(
    val payload: String?,
)


fun getUserAsJson(id: Int): GetUserAsJsonRow =
    transaction {
        exec("SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = ?", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to id), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val payloadValue = rs.getString("payload")
                val payload = if (rs.wasNull()) null else payloadValue
                GetUserAsJsonRow(
                    payload = payload,
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("getUserAsJson: no rows returned")
    }


data class GetUsersAsJsonRow(
    val payload: String?,
)


fun getUsersAsJson(): GetUsersAsJsonRow =
    transaction {
        exec("SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u", explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val payloadValue = rs.getString("payload")
                val payload = if (rs.wasNull()) null else payloadValue
                GetUsersAsJsonRow(
                    payload = payload,
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("getUsersAsJson: no rows returned")
    }


data class GetUserOrdersAsJsonRow(
    val payload: String?,
)


fun getUserOrdersAsJson(id: Int): GetUserOrdersAsJsonRow =
    transaction {
        exec("SELECT json_agg(o.* ORDER BY o.id) AS payload FROM users u LEFT JOIN orders o ON o.user_id = u.id WHERE u.id = ? GROUP BY u.id", listOf<Pair<IColumnType<*>, Any?>>(IntegerColumnType() to id), explicitStatementType = StatementType.SELECT) { rs ->
            if (rs.next()) {
                val payloadValue = rs.getString("payload")
                val payload = if (rs.wasNull()) null else payloadValue
                GetUserOrdersAsJsonRow(
                    payload = payload,
                )
            } else {
                null
            }
        } ?: throw NoSuchElementException("getUserOrdersAsJson: no rows returned")
    }

