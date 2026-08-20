// scythe:provenance v=0.16.1 backend=kotlin-r2dbc engine=postgresql schema=sch1:c247390d575b8f71 queries=q1:a78685f58b075ff5 options=opt1:cbf29ce484222325
package generated

import io.r2dbc.spi.ConnectionFactory
import io.r2dbc.spi.Statement
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.reactive.asFlow
import kotlinx.coroutines.reactive.awaitFirst
import kotlinx.coroutines.reactive.awaitFirstOrNull
import reactor.core.publisher.Flux
import reactor.core.publisher.Mono
import java.math.BigDecimal
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


data class GetOrdersByUserRow(
    val id: Int,
    val total: java.math.BigDecimal,
    val notes: String?,
    val created_at: java.time.OffsetDateTime,
)


data class GetOrderTotalRow(
    val total_sum: java.math.BigDecimal?,
)


data class GetOrderWeightTotalRow(
    val weight_total: Double?,
)


data class GetUserByIdRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


data class ListActiveUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


data class CreateUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


data class GetUserOrdersRow(
    val id: Int,
    val name: String,
    val total: java.math.BigDecimal?,
    val notes: String?,
)


data class CountUsersByStatusRow(
    val status: UserStatus,
    val user_count: Long,
)


data class GetUserWithTagsRow(
    val id: Int,
    val name: String,
    val tag_name: String,
)


data class SearchUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


data class UserAddress(
    val street: String,
    val city: String,
    val zip: String,
) {
    companion object {
        /**
         * ~keep board #196: r2dbc-postgresql has no codec for this composite -- an
         * unregistered `row.get(col, UserAddress::class.java)` is driver-codec-dependent and
         * throws at runtime. Parse the driver's composite text form instead.
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


private fun bindNullable(stmt: Statement, index: Int, value: Any?, type: Class<*>) {
    if (value == null) {
        stmt.bindNull(index, type)
    } else {
        stmt.bind(index, value)
    }
}

suspend fun createOrder(
    cf: ConnectionFactory,
    user_id: Int,
    total: java.math.BigDecimal,
    notes: String?,
): CreateOrderRow {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("INSERT INTO orders (user_id, total, notes) VALUES (\$1, \$2, \$3) RETURNING id, user_id, total, notes, created_at")
        stmt.bind(0, user_id)
        stmt.bind(1, total)
        bindNullable(stmt, 2, notes, String::class.java)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        CreateOrderRow(
                            id = row.get("id", Int::class.javaObjectType),
                            user_id = row.get("user_id", Int::class.javaObjectType),
                            total = row.get("total", java.math.BigDecimal::class.java),
                            notes = row.get("notes", String::class.java),
                            created_at = row.get("created_at", java.time.OffsetDateTime::class.java),
                        )
                    },
                )
            }
            .switchIfEmpty(Mono.error(java.util.NoSuchElementException("createOrder: no rows returned")))
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


fun getOrdersByUser(
    cf: ConnectionFactory,
    user_id: Int,
): Flow<GetOrdersByUserRow> =
    Flux
        .usingWhen(
            cf.create(),
            { conn ->
                val stmt = conn.createStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = \$1 ORDER BY created_at DESC")
                stmt.bind(0, user_id)
                Flux
                    .from(stmt.execute())
                    .flatMap { result ->
                        result.map { row, _ ->
                            GetOrdersByUserRow(
                                id = row.get("id", Int::class.javaObjectType),
                                total = row.get("total", java.math.BigDecimal::class.java),
                                notes = row.get("notes", String::class.java),
                                created_at = row.get("created_at", java.time.OffsetDateTime::class.java),
                            )
                        }
                    }
            },
            { conn -> Mono.from(conn.close()) },
        ).asFlow()


suspend fun getOrderTotal(
    cf: ConnectionFactory,
    user_id: Int,
): GetOrderTotalRow {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = \$1")
        stmt.bind(0, user_id)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        GetOrderTotalRow(
                            total_sum = row.get("total_sum", java.math.BigDecimal::class.java),
                        )
                    },
                )
            }
            .switchIfEmpty(Mono.error(java.util.NoSuchElementException("getOrderTotal: no rows returned")))
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


suspend fun getOrderWeightTotal(
    cf: ConnectionFactory,
    user_id: Int,
): GetOrderWeightTotalRow {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = \$1")
        stmt.bind(0, user_id)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        GetOrderWeightTotalRow(
                            weight_total = row.get("weight_total", Double::class.javaObjectType),
                        )
                    },
                )
            }
            .switchIfEmpty(Mono.error(java.util.NoSuchElementException("getOrderWeightTotal: no rows returned")))
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


suspend fun deleteOrdersByUser(
    cf: ConnectionFactory,
    user_id: Int,
): Long {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("DELETE FROM orders WHERE user_id = \$1")
        stmt.bind(0, user_id)
        return Mono
            .from(stmt.execute())
            .flatMap { result -> Mono.from(result.rowsUpdated) }
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


suspend fun getUserById(
    cf: ConnectionFactory,
    id: Int,
): GetUserByIdRow {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("SELECT id, name, email, status, created_at FROM users WHERE id = \$1")
        stmt.bind(0, id)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        GetUserByIdRow(
                            id = row.get("id", Int::class.javaObjectType),
                            name = row.get("name", String::class.java),
                            email = row.get("email", String::class.java),
                            status = UserStatus.fromValue(row.get("status", String::class.java)),
                            created_at = row.get("created_at", java.time.OffsetDateTime::class.java),
                        )
                    },
                )
            }
            .switchIfEmpty(Mono.error(java.util.NoSuchElementException("getUserById: no rows returned")))
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


fun listActiveUsers(
    cf: ConnectionFactory,
    status: UserStatus,
): Flow<ListActiveUsersRow> =
    Flux
        .usingWhen(
            cf.create(),
            { conn ->
                val stmt = conn.createStatement("SELECT id, name, email FROM users WHERE status = \$1::user_status")
                stmt.bind(0, status.value)
                Flux
                    .from(stmt.execute())
                    .flatMap { result ->
                        result.map { row, _ ->
                            ListActiveUsersRow(
                                id = row.get("id", Int::class.javaObjectType),
                                name = row.get("name", String::class.java),
                                email = row.get("email", String::class.java),
                            )
                        }
                    }
            },
            { conn -> Mono.from(conn.close()) },
        ).asFlow()


suspend fun createUser(
    cf: ConnectionFactory,
    name: String,
    email: String?,
    status: UserStatus,
): CreateUserRow {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("INSERT INTO users (name, email, status) VALUES (\$1, \$2, \$3::user_status) RETURNING id, name, email, status, created_at")
        stmt.bind(0, name)
        bindNullable(stmt, 1, email, String::class.java)
        stmt.bind(2, status.value)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        CreateUserRow(
                            id = row.get("id", Int::class.javaObjectType),
                            name = row.get("name", String::class.java),
                            email = row.get("email", String::class.java),
                            status = UserStatus.fromValue(row.get("status", String::class.java)),
                            created_at = row.get("created_at", java.time.OffsetDateTime::class.java),
                        )
                    },
                )
            }
            .switchIfEmpty(Mono.error(java.util.NoSuchElementException("createUser: no rows returned")))
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


suspend fun updateUserEmail(
    cf: ConnectionFactory,
    email: String,
    id: Int,
) {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("UPDATE users SET email = \$1 WHERE id = \$2")
        stmt.bind(0, email)
        stmt.bind(1, id)
        Mono.from(stmt.execute()).flatMap { result -> Mono.from(result.rowsUpdated) }.awaitFirstOrNull()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


suspend fun deleteUser(
    cf: ConnectionFactory,
    id: Int,
) {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("DELETE FROM users WHERE id = \$1")
        stmt.bind(0, id)
        Mono.from(stmt.execute()).flatMap { result -> Mono.from(result.rowsUpdated) }.awaitFirstOrNull()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


fun getUserOrders(
    cf: ConnectionFactory,
    status: UserStatus,
): Flow<GetUserOrdersRow> =
    Flux
        .usingWhen(
            cf.create(),
            { conn ->
                val stmt = conn.createStatement("SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = \$1::user_status")
                stmt.bind(0, status.value)
                Flux
                    .from(stmt.execute())
                    .flatMap { result ->
                        result.map { row, _ ->
                            GetUserOrdersRow(
                                id = row.get("id", Int::class.javaObjectType),
                                name = row.get("name", String::class.java),
                                total = row.get("total", java.math.BigDecimal::class.java),
                                notes = row.get("notes", String::class.java),
                            )
                        }
                    }
            },
            { conn -> Mono.from(conn.close()) },
        ).asFlow()


suspend fun countUsersByStatus(
    cf: ConnectionFactory,
    status: UserStatus,
): CountUsersByStatusRow {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = \$1::user_status")
        stmt.bind(0, status.value)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        CountUsersByStatusRow(
                            status = UserStatus.fromValue(row.get("status", String::class.java)),
                            user_count = row.get("user_count", Long::class.javaObjectType),
                        )
                    },
                )
            }
            .switchIfEmpty(Mono.error(java.util.NoSuchElementException("countUsersByStatus: no rows returned")))
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}


fun getUserWithTags(
    cf: ConnectionFactory,
    id: Int,
): Flow<GetUserWithTagsRow> =
    Flux
        .usingWhen(
            cf.create(),
            { conn ->
                val stmt = conn.createStatement("SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = \$1")
                stmt.bind(0, id)
                Flux
                    .from(stmt.execute())
                    .flatMap { result ->
                        result.map { row, _ ->
                            GetUserWithTagsRow(
                                id = row.get("id", Int::class.javaObjectType),
                                name = row.get("name", String::class.java),
                                tag_name = row.get("tag_name", String::class.java),
                            )
                        }
                    }
            },
            { conn -> Mono.from(conn.close()) },
        ).asFlow()


fun searchUsers(
    cf: ConnectionFactory,
    name: String,
): Flow<SearchUsersRow> =
    Flux
        .usingWhen(
            cf.create(),
            { conn ->
                val stmt = conn.createStatement("SELECT id, name, email FROM users WHERE name LIKE \$1")
                stmt.bind(0, name)
                Flux
                    .from(stmt.execute())
                    .flatMap { result ->
                        result.map { row, _ ->
                            SearchUsersRow(
                                id = row.get("id", Int::class.javaObjectType),
                                name = row.get("name", String::class.java),
                                email = row.get("email", String::class.java),
                            )
                        }
                    }
            },
            { conn -> Mono.from(conn.close()) },
        ).asFlow()


suspend fun getUserProfile(
    cf: ConnectionFactory,
    id: Int,
): GetUserProfileRow {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("SELECT id, secondary_status, address FROM users WHERE id = \$1")
        stmt.bind(0, id)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        GetUserProfileRow(
                            id = row.get("id", Int::class.javaObjectType),
                            secondary_status = UserStatus.fromValue(row.get("secondary_status", String::class.java)),
                            address = UserAddress.fromText(row.get("address", String::class.java)),
                        )
                    },
                )
            }
            .switchIfEmpty(Mono.error(java.util.NoSuchElementException("getUserProfile: no rows returned")))
            .awaitFirst()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}

