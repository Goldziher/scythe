import generated.*
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
    val mssqlUrl = System.getenv("MSSQL_URL")
    if (mssqlUrl.isNullOrEmpty()) {
        System.err.println("MSSQL_URL environment variable is required")
        exitProcess(1)
    }

    // Convert sqlserver://user:pass@host:port?database=db to JDBC format
    val uri = java.net.URI(mssqlUrl)
    val userInfo = uri.userInfo?.split(":") ?: listOf("", "")
    val user = userInfo[0]
    val password = if (userInfo.size > 1) userInfo[1] else ""
    val database = if (uri.query != null) uri.query!!.replaceFirst("database=".toRegex(), "") else ""
    val jdbcUrl = "jdbc:sqlserver://${uri.host}:${uri.port};databaseName=$database;trustServerCertificate=true"

    DriverManager.getConnection(jdbcUrl, user, password).use { conn ->
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
        .resolve("../sql/mssql/schema.sql")
        .normalize()
    val schema = schemaPath.readText()

    conn.createStatement().use { stmt ->
        stmt.execute("IF OBJECT_ID('user_tags', 'U') IS NOT NULL DROP TABLE user_tags")
        stmt.execute("IF OBJECT_ID('tags', 'U') IS NOT NULL DROP TABLE tags")
        stmt.execute("IF OBJECT_ID('orders', 'U') IS NOT NULL DROP TABLE orders")
        stmt.execute("IF OBJECT_ID('users', 'U') IS NOT NULL DROP TABLE users")
    }

    // MSSQL requires executing statements one at a time
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
        val user = createUser(conn, "Alice", "alice@example.com", true)
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

fun testListActiveUsers(conn: java.sql.Connection) {
    val name = "ListActiveUsers"
    try {
        val users = listActiveUsers(conn, true)
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
        val order = createOrder(conn, createdUserId, "99.95", "Test order")
        if (order == null) {
            fail(name, "returned null")
            return
        }
        if (order.user_id != createdUserId) {
            fail(name, "expected user_id $createdUserId, got ${order.user_id}")
            return
        }
        if (order.total != "99.95") {
            fail(name, "expected total 99.95, got ${order.total}")
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
