import generated.*
import java.math.BigDecimal
import java.nio.file.Path
import java.sql.DriverManager
import kotlin.io.path.readText
import kotlin.system.exitProcess

var passed = 0
var failed = 0

fun pass(name: String) {
    println("PASS: $name")
    passed++
}

fun fail(name: String, message: String) {
    println("FAIL: $name - $message")
    failed++
}

fun fail(name: String, e: Exception) {
    println("FAIL: $name - ${e.message}")
    failed++
}

var createdUserId = 0

fun main() {
    val databaseUrl = System.getenv("DATABASE_URL")
    if (databaseUrl.isNullOrEmpty()) {
        System.err.println("DATABASE_URL environment variable is required")
        exitProcess(1)
    }

    val uri = java.net.URI(databaseUrl)
    val userInfo = uri.userInfo?.split(":") ?: listOf("", "")
    val user = userInfo[0]
    val password = if (userInfo.size > 1) userInfo[1] else ""
    val jdbcUrl = "jdbc:postgresql://${uri.host}:${uri.port}${uri.path}"

    DriverManager.getConnection(jdbcUrl, user, password).use { conn ->
        runMigration(conn)

        testCreateUser(conn)
        testGetUserById(conn)
        testUpdateUserEmail(conn)
        testCreateOrder(conn)
        testGetOrdersByUser(conn)
        testGetOrderTotal(conn)
        testListActiveUsers(conn)
        testGetUserOrders(conn)
        testCountUsersByStatus(conn)
        testSearchUsers(conn)
        testGetUserProfileNullable(conn)
        testDeleteOrdersByUser(conn)
        testDeleteUser(conn)
    }

    println()
    println("Results: $passed passed, $failed failed")
    if (failed > 0) {
        exitProcess(1)
    }
    println("ALL TESTS PASSED")
}

fun runMigration(conn: java.sql.Connection) {
    val schemaPath = Path.of(System.getProperty("user.dir"))
        .resolve("../sql/pg/schema.sql")
        .normalize()
    val schema = schemaPath.readText()

    val dropSql = """
        DROP TABLE IF EXISTS user_tags CASCADE;
        DROP TABLE IF EXISTS tags CASCADE;
        DROP TABLE IF EXISTS orders CASCADE;
        DROP TABLE IF EXISTS users CASCADE;
        DROP TYPE IF EXISTS user_status CASCADE;
        DROP TYPE IF EXISTS user_address CASCADE;
    """.trimIndent()

    conn.createStatement().use { stmt ->
        stmt.execute(dropSql)
        stmt.execute(schema)
    }
}

fun testCreateUser(conn: java.sql.Connection) {
    val name = "CreateUser"
    try {
        val user = createUser(conn, "Alice", "alice@example.com", UserStatus.ACTIVE)
        if (user == null) {
            fail(name, "returned null")
            return
        }
        if (user.name != "Alice") {
            fail(name, "expected name Alice, got ${user.name}")
            return
        }
        if (user.email != "alice@example.com") {
            fail(name, "expected email alice@example.com, got ${user.email}")
            return
        }
        createdUserId = user.id
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetUserById(conn: java.sql.Connection) {
    val name = "GetUserById"
    try {
        val user = getUserById(conn, createdUserId)
        if (user == null) {
            fail(name, "returned null")
            return
        }
        if (user.name != "Alice") {
            fail(name, "expected name Alice, got ${user.name}")
            return
        }
        if (user.id != createdUserId) {
            fail(name, "expected id $createdUserId, got ${user.id}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testUpdateUserEmail(conn: java.sql.Connection) {
    val name = "UpdateUserEmail"
    try {
        updateUserEmail(conn, "alice-updated@example.com", createdUserId)
        val user = getUserById(conn, createdUserId)
        if (user == null) {
            fail(name, "user not found after update")
            return
        }
        if (user.email != "alice-updated@example.com") {
            fail(name, "expected updated email, got ${user.email}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testCreateOrder(conn: java.sql.Connection) {
    val name = "CreateOrder"
    try {
        val order = createOrder(conn, createdUserId, BigDecimal("99.99"), "Test order")
        if (order == null) {
            fail(name, "returned null")
            return
        }
        if (order.user_id != createdUserId) {
            fail(name, "expected user_id $createdUserId, got ${order.user_id}")
            return
        }
        if (order.total.compareTo(BigDecimal("99.99")) != 0) {
            fail(name, "expected total 99.99, got ${order.total}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetOrdersByUser(conn: java.sql.Connection) {
    val name = "GetOrdersByUser"
    try {
        val orders = getOrdersByUser(conn, createdUserId)
        if (orders.size != 1) {
            fail(name, "expected 1 order, got ${orders.size}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetOrderTotal(conn: java.sql.Connection) {
    val name = "GetOrderTotal"
    try {
        val result = getOrderTotal(conn, createdUserId)
        if (result == null || result.total_sum == null) {
            fail(name, "returned null")
            return
        }
        if (result.total_sum.compareTo(BigDecimal("99.99")) != 0) {
            fail(name, "expected total_sum 99.99, got ${result.total_sum}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testListActiveUsers(conn: java.sql.Connection) {
    val name = "ListActiveUsers"
    try {
        val users = listActiveUsers(conn, UserStatus.ACTIVE)
        if (users.isEmpty()) {
            fail(name, "expected at least 1 active user")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetUserOrders(conn: java.sql.Connection) {
    val name = "GetUserOrders"
    try {
        val results = getUserOrders(conn, UserStatus.ACTIVE)
        if (results.isEmpty()) {
            fail(name, "expected at least 1 result")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testCountUsersByStatus(conn: java.sql.Connection) {
    val name = "CountUsersByStatus"
    try {
        val result = countUsersByStatus(conn, UserStatus.ACTIVE)
        if (result == null) {
            fail(name, "returned null")
            return
        }
        if (result.user_count < 1) {
            fail(name, "expected count >= 1, got ${result.user_count}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testSearchUsers(conn: java.sql.Connection) {
    val name = "SearchUsers"
    try {
        val users = searchUsers(conn, "%Alice%")
        if (users.isEmpty()) {
            fail(name, "expected at least 1 user matching Alice")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

// board #197: a nullable enum and a nullable composite column, each
// observed both present and as SQL NULL. Seeded via raw SQL because a
// composite VALUES literal is outside this generator's parameter-binding
// surface -- the point is the *read* path, which runs through generated
// code (getUserProfile).
fun testGetUserProfileNullable(conn: java.sql.Connection) {
    val name = "GetUserProfile"
    try {
        var presentId: Int
        var absentId: Int
        conn.createStatement().use { stmt ->
            stmt.executeQuery(
                "INSERT INTO users (name, email, status, secondary_status, address) " +
                    "VALUES ('Carol', 'carol@example.com', 'active', 'inactive', " +
                    "ROW('1 Main St', 'Springfield', '12345')) RETURNING id"
            ).use { rs ->
                rs.next()
                presentId = rs.getInt(1)
            }
        }
        conn.createStatement().use { stmt ->
            stmt.executeQuery(
                "INSERT INTO users (name, email, status, secondary_status, address) " +
                    "VALUES ('Dave', 'dave@example.com', 'active', NULL, NULL) RETURNING id"
            ).use { rs ->
                rs.next()
                absentId = rs.getInt(1)
            }
        }

        val profile = getUserProfile(conn, presentId)
        // Fails if a nullable enum reader zero-decodes instead of returning the value.
        if (profile.secondary_status != UserStatus.INACTIVE) {
            fail(name, "expected secondary_status INACTIVE, got ${profile.secondary_status}")
            return
        }
        // Fails if a nullable composite reader throws or returns null/zero fields on a present value.
        val address = profile.address
        if (address == null) {
            fail(name, "expected address to be present")
            return
        }
        if (address.street != "1 Main St") {
            fail(name, "expected address.street '1 Main St', got ${address.street}")
            return
        }
        if (address.city != "Springfield") {
            fail(name, "expected address.city 'Springfield', got ${address.city}")
            return
        }
        if (address.zip != "12345") {
            fail(name, "expected address.zip '12345', got ${address.zip}")
            return
        }

        val nullProfile = getUserProfile(conn, absentId)
        // Fails if a nullable enum reader decodes SQL NULL as a zero/empty variant instead of null.
        if (nullProfile.secondary_status != null) {
            fail(name, "expected secondary_status null, got ${nullProfile.secondary_status}")
            return
        }
        // Fails if a nullable composite reader decodes SQL NULL as a non-null all-default object.
        if (nullProfile.address != null) {
            fail(name, "expected address null, got ${nullProfile.address}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}


fun testDeleteOrdersByUser(conn: java.sql.Connection) {
    val name = "DeleteOrdersByUser"
    try {
        val count = deleteOrdersByUser(conn, createdUserId)
        if (count != 1) {
            fail(name, "expected 1 deleted order, got $count")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testDeleteUser(conn: java.sql.Connection) {
    val name = "DeleteUser"
    try {
        deleteUser(conn, createdUserId)
        // GetUserById is `:one`, so a missing row throws
        // NoSuchElementException rather than returning null.
        try {
            getUserById(conn, createdUserId)
            fail(name, "expected getUserById to throw after deletion, but it returned a row")
            return
        } catch (expected: NoSuchElementException) {
            // expected: the user was deleted
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}
