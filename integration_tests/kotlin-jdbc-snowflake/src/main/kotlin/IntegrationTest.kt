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

var createdUserId = 0L

fun main() {
    val snowflakeUrl = System.getenv("SNOWFLAKE_URL")
    if (snowflakeUrl.isNullOrEmpty()) {
        System.err.println("SNOWFLAKE_URL environment variable is required")
        exitProcess(1)
    }

    // Parse snowflake://user:pass@host:port/database/schema?account=X
    val uri = java.net.URI(snowflakeUrl)
    val userInfo = uri.userInfo?.split(":") ?: listOf("", "")
    val user = userInfo[0]
    val password = if (userInfo.size > 1) userInfo[1] else ""

    val pathParts = uri.path.split("/")
    val database = if (pathParts.size > 1) pathParts[1] else ""
    val schema = if (pathParts.size > 2) pathParts[2] else ""

    // Parse account from query params
    var account = ""
    if (!uri.query.isNullOrEmpty()) {
        for (param in uri.query.split("&")) {
            if (param.startsWith("account=")) {
                account = param.substring("account=".length)
                break
            }
        }
    }

    val jdbcUrl = "jdbc:snowflake://${uri.host}:${uri.port}/?account=$account&db=$database&schema=$schema&user=$user&password=$password"

    DriverManager.getConnection(jdbcUrl).use { conn ->
        runMigration(conn)

        testCreateUser(conn)
        testGetUserById(conn)
        testListActiveUsers(conn)
        testCreateOrder(conn)
        testGetOrdersByUser(conn)
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
        .resolve("../sql/snowflake/schema.sql")
        .normalize()
    val schema = schemaPath.readText()

    conn.createStatement().use { stmt ->
        stmt.execute("DROP TABLE IF EXISTS user_tags")
        stmt.execute("DROP TABLE IF EXISTS tags")
        stmt.execute("DROP TABLE IF EXISTS orders")
        stmt.execute("DROP TABLE IF EXISTS users")
    }

    // Snowflake requires executing statements one at a time
    for (sql in schema.split(";")) {
        val trimmed = sql.trim()
        if (trimmed.isNotEmpty()) {
            conn.createStatement().use { stmt ->
                stmt.execute(trimmed)
            }
        }
    }
}

fun testCreateUser(conn: java.sql.Connection) {
    val name = "CreateUser"
    try {
        createUser(conn, "Alice", "alice@example.com")
        val user = getUserById(conn, 1L)
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
        createdUserId = 1L
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

fun testListActiveUsers(conn: java.sql.Connection) {
    val name = "ListActiveUsers"
    try {
        val users = listActiveUsers(conn)
        if (users.isEmpty()) {
            fail(name, "expected at least 1 active user")
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
        createOrder(conn, createdUserId, BigDecimal("99.99"), "Test order")
        val orders = getOrdersByUser(conn, createdUserId)
        if (orders.isEmpty()) {
            fail(name, "returned no orders")
            return
        }
        val order = orders[0]
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
