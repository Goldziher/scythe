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
    val mysqlUrl = System.getenv("MYSQL_URL")
    if (mysqlUrl.isNullOrEmpty()) {
        System.err.println("MYSQL_URL environment variable is required")
        exitProcess(1)
    }

    val uri = java.net.URI(mysqlUrl)
    val userInfo = uri.userInfo?.split(":") ?: listOf("", "")
    val user = userInfo[0]
    val password = if (userInfo.size > 1) userInfo[1] else ""
    val jdbcUrl = "jdbc:mysql://${uri.host}:${uri.port}${uri.path}"

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

// Splits a SQL script into statements on top-level ';' only -- unlike a
// naive `schema.split(";")`, this tracks single- and double-quoted spans,
// PostgreSQL dollar-quoted bodies, and "--" line comments (an apostrophe
// in a comment must not open a phantom string -- board #224 follow-up) so
// a ';' inside a string literal, a `$$ ... $$` function body, or a
// comment does not split the statement in half. "/* ... */" block
// comments are not handled -- no schema under integration_tests/sql/ uses
// them today.
fun splitSqlStatements(sql: String): List<String> {
    val statements = mutableListOf<String>()
    val current = StringBuilder()
    var inSingle = false
    var inDouble = false
    var inLineComment = false
    var dollarTag: String? = null
    var i = 0
    while (i < sql.length) {
        val ch = sql[i]
        val tag = dollarTag
        if (inLineComment) {
            current.append(ch)
            if (ch == '\n') inLineComment = false
            i++
            continue
        }
        if (tag != null) {
            current.append(ch)
            if (ch == '$' && sql.regionMatches(i, tag, 0, tag.length)) {
                current.append(tag.substring(1))
                i += tag.length
                dollarTag = null
                continue
            }
            i++
            continue
        }
        if (inSingle) {
            current.append(ch)
            if (ch == '\'') inSingle = false
            i++
            continue
        }
        if (inDouble) {
            current.append(ch)
            if (ch == '"') inDouble = false
            i++
            continue
        }
        if (ch == '-' && i + 1 < sql.length && sql[i + 1] == '-') {
            inLineComment = true
            current.append(ch)
            i++
            continue
        }
        when {
            ch == '\'' -> {
                inSingle = true
                current.append(ch)
                i++
            }
            ch == '"' -> {
                inDouble = true
                current.append(ch)
                i++
            }
            ch == '$' -> {
                val match = Regex("^\\$[A-Za-z0-9_]*\\$").find(sql.substring(i))
                if (match != null) {
                    dollarTag = match.value
                    current.append(match.value)
                    i += match.value.length
                } else {
                    current.append(ch)
                    i++
                }
            }
            ch == ';' -> {
                statements.add(current.toString())
                current.setLength(0)
                i++
            }
            else -> {
                current.append(ch)
                i++
            }
        }
    }
    if (current.toString().isNotBlank()) {
        statements.add(current.toString())
    }
    return statements.map { it.trim() }.filter { it.isNotEmpty() }
}

fun runMigration(conn: java.sql.Connection) {
    val schemaPath = Path.of(System.getProperty("user.dir"))
        .resolve("../sql/mysql/schema.sql")
        .normalize()
    val schema = schemaPath.readText()

    conn.createStatement().use { stmt ->
        stmt.execute("DROP TABLE IF EXISTS user_tags")
        stmt.execute("DROP TABLE IF EXISTS tags")
        stmt.execute("DROP TABLE IF EXISTS orders")
        stmt.execute("DROP TABLE IF EXISTS users")
    }

    // MySQL requires executing statements one at a time
    for (sql in splitSqlStatements(schema)) {
        conn.createStatement().use { stmt ->
            stmt.execute(sql)
        }
    }
}

fun testCreateUser(conn: java.sql.Connection) {
    val name = "CreateUser"
    try {
        createUser(conn, "Alice", "alice@example.com", UsersStatus.ACTIVE)
        val user = getLastInsertUser(conn)
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
        val users = listActiveUsers(conn, UsersStatus.ACTIVE)
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
        val order = getLastInsertOrder(conn)
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
