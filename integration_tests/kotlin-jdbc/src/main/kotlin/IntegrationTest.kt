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
    """.trimIndent()

    conn.createStatement().use { stmt ->
        stmt.execute(dropSql)
        stmt.execute(schema)
    }
}

fun testCreateUser(conn: java.sql.Connection) {
    val name = "CreateUser"
    try {
        val user = createUser(conn, "Alice", "alice@example.com", UserStatus.active)
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
        val users = listActiveUsers(conn, UserStatus.active)
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
        val results = getUserOrders(conn, UserStatus.active)
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
        val result = countUsersByStatus(conn, UserStatus.active)
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
        val user = getUserById(conn, createdUserId)
        if (user != null) {
            fail(name, "expected null after deletion, but user still exists")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}
