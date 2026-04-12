import generated.*
import java.nio.file.Path
import java.sql.DriverManager
import java.sql.SQLException
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

var createdUserId: Long = 0

fun main() {
    val oracleUrl = System.getenv("ORACLE_URL")
    if (oracleUrl.isNullOrEmpty()) {
        System.err.println("ORACLE_URL environment variable is required")
        exitProcess(1)
    }

    // Convert oracle://user:pass@host:port/service to JDBC thin format
    val uri = java.net.URI(oracleUrl)
    val userInfo = uri.userInfo?.split(":") ?: listOf("", "")
    val user = userInfo[0]
    val password = if (userInfo.size > 1) userInfo[1] else ""
    val jdbcUrl = "jdbc:oracle:thin:@${uri.host}:${uri.port}${uri.path}"

    DriverManager.getConnection(jdbcUrl, user, password).use { conn ->
        runMigration(conn)

        testCreateUser(conn)
        testGetUserById(conn)
        testListActiveUsers(conn)
        testCreateOrder(conn)
        testGetOrdersByUser(conn)
        testGetOrderTotal(conn)
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
    // Drop tables and sequences, ignoring errors if they do not exist
    val drops = listOf(
        "DROP TABLE user_tags",
        "DROP TABLE tags",
        "DROP TABLE orders",
        "DROP TABLE users",
        "DROP SEQUENCE tags_seq",
        "DROP SEQUENCE orders_seq",
        "DROP SEQUENCE users_seq"
    )
    for (drop in drops) {
        try {
            conn.createStatement().use { stmt -> stmt.execute(drop) }
        } catch (ignored: SQLException) {
            // object may not exist
        }
    }

    val schemaPath = Path.of(System.getProperty("user.dir"))
        .resolve("../sql/oracle/schema_full.sql")
        .normalize()
    val schema = schemaPath.readText()

    // Oracle PL/SQL blocks are delimited by /\n
    for (block in schema.split("/\n")) {
        val trimmed = block.trim()
        if (trimmed.isNotEmpty()) {
            conn.createStatement().use { stmt -> stmt.execute(trimmed) }
        }
    }
}

fun testCreateUser(conn: java.sql.Connection) {
    val name = "CreateUser"
    try {
        val user = createUser(conn, "Alice", "alice@example.com", 1)
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
        val order = createOrder(conn, createdUserId, 9999L, "Test order")
        if (order == null) {
            fail(name, "returned null")
            return
        }
        if (order.user_id != createdUserId) {
            fail(name, "expected user_id $createdUserId, got ${order.user_id}")
            return
        }
        if (order.total != 9999L) {
            fail(name, "expected total 9999, got ${order.total}")
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
        if (result == null) {
            fail(name, "returned null")
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
