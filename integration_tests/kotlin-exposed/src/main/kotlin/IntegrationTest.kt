import generated.*
import java.math.BigDecimal
import java.nio.file.Path
import kotlin.io.path.readText
import kotlin.system.exitProcess
import org.jetbrains.exposed.sql.Database
import org.jetbrains.exposed.sql.statements.StatementType
import org.jetbrains.exposed.sql.transactions.transaction

// Exposed is a DSL, not a driver, and that changes the harness shape: a
// generated query function takes no Connection. It opens its own
// `transaction { }` against the ambient Database that `Database.connect`
// installs, so this file connects once in main() and then calls the query
// functions with query arguments only -- unlike every kotlin-jdbc branch
// below, which threads `conn` through every test.

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
var bobId = 0

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

    Database.connect(jdbcUrl, driver = "org.postgresql.Driver", user = user, password = password)

    runMigration()

    testCreateUser()
    testGetUserById()
    testUpdateUserEmail()
    testCreateOrder()
    testGetOrdersByUser()
    testGetOrderTotal()
    testListActiveUsers()
    testGetUserOrders()
    testCountUsersByStatus()
    testGetUserWithTags()
    testSearchUsers()
    testGetUserProfileNullable()
    testDeleteOrdersByUser()
    testDeleteUser()

    println()
    println("Results: $passed passed, $failed failed")
    if (failed > 0) {
        exitProcess(1)
    }
    println("ALL TESTS PASSED")
}

// The schema file holds several statements; Exposed's `exec` sends one at a
// time, so it is split the same way every other harness splits it. The
// fixture is comment-free and single-quote-free by the guard in
// tools/integration-test-generator/tests/coverage_completeness.rs, so a plain
// split on ';' is sound here.
fun runMigration() {
    val schemaPath = Path.of(System.getProperty("user.dir"))
        .resolve("../sql/pg/schema.sql")
        .normalize()
    val schema = schemaPath.readText()

    transaction {
        for (stmt in "DROP TABLE IF EXISTS user_tags CASCADE;DROP TABLE IF EXISTS tags CASCADE;DROP TABLE IF EXISTS orders CASCADE;DROP TABLE IF EXISTS users CASCADE;DROP TYPE IF EXISTS user_status CASCADE;DROP TYPE IF EXISTS user_address CASCADE".split(";")) {
            exec(stmt)
        }
        for (stmt in schema.split(";")) {
            if (stmt.isNotBlank()) {
                exec(stmt)
            }
        }
    }
}

fun testCreateUser() {
    val name = "CreateUser"
    try {
        val user = createUser("Alice", "alice@example.com", UserStatus.ACTIVE)
        if (user.name != "Alice") {
            fail(name, "expected name Alice, got ${user.name}")
            return
        }
        if (user.email != "alice@example.com") {
            fail(name, "expected email alice@example.com, got ${user.email}")
            return
        }
        if (user.status != UserStatus.ACTIVE) {
            fail(name, "expected status ACTIVE, got ${user.status}")
            return
        }
        createdUserId = user.id
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetUserById() {
    val name = "GetUserById"
    try {
        val user = getUserById(createdUserId)
        if (user.id != createdUserId) {
            fail(name, "expected id $createdUserId, got ${user.id}")
            return
        }
        if (user.name != "Alice") {
            fail(name, "expected name Alice, got ${user.name}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testUpdateUserEmail() {
    val name = "UpdateUserEmail"
    try {
        updateUserEmail("alice-updated@example.com", createdUserId)
        val user = getUserById(createdUserId)
        if (user.email != "alice-updated@example.com") {
            fail(name, "expected updated email, got ${user.email}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testCreateOrder() {
    val name = "CreateOrder"
    try {
        val order = createOrder(createdUserId, BigDecimal("99.99"), "Test order")
        if (order.user_id != createdUserId) {
            fail(name, "expected user_id $createdUserId, got ${order.user_id}")
            return
        }
        if (order.total.compareTo(BigDecimal("99.99")) != 0) {
            fail(name, "expected total 99.99, got ${order.total}")
            return
        }
        if (order.notes != "Test order") {
            fail(name, "expected notes 'Test order', got ${order.notes}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetOrdersByUser() {
    val name = "GetOrdersByUser"
    try {
        val orders = getOrdersByUser(createdUserId)
        if (orders.size != 1) {
            fail(name, "expected 1 order, got ${orders.size}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetOrderTotal() {
    val name = "GetOrderTotal"
    try {
        val result = getOrderTotal(createdUserId)
        val sum = result.total_sum
        if (sum == null) {
            fail(name, "returned null total_sum")
            return
        }
        if (sum.compareTo(BigDecimal("99.99")) != 0) {
            fail(name, "expected total_sum 99.99, got $sum")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

// The enum round-trip is the point of this one: the parameter goes out as the
// enum's SQL spelling and the rows come back decoded through fromValue, so a
// binding that sent the Kotlin enum object instead would fail here rather
// than somewhere downstream.
fun testListActiveUsers() {
    val name = "ListActiveUsers"
    try {
        val users = listActiveUsers(UserStatus.ACTIVE)
        if (users.isEmpty()) {
            fail(name, "expected at least 1 active user")
            return
        }
        if (users.none { it.name == "Alice" }) {
            fail(name, "expected Alice among active users")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetUserOrders() {
    val name = "GetUserOrders"
    try {
        val bob = createUser("Bob", "bob@example.com", UserStatus.ACTIVE)
        bobId = bob.id
        val rows = getUserOrders(UserStatus.ACTIVE)
        // Bob has no orders, so his LEFT JOIN row must carry a null total --
        // the nullable-side column the join widens.
        val bobRow = rows.firstOrNull { it.id == bobId }
        if (bobRow == null) {
            fail(name, "expected a row for Bob from the LEFT JOIN")
            return
        }
        if (bobRow.total != null) {
            fail(name, "expected null total for a user with no orders, got ${bobRow.total}")
            return
        }
        if (rows.none { it.id == createdUserId && it.total != null }) {
            fail(name, "expected Alice's row to carry a non-null total")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testCountUsersByStatus() {
    val name = "CountUsersByStatus"
    try {
        val result = countUsersByStatus(UserStatus.ACTIVE)
        if (result.status != UserStatus.ACTIVE) {
            fail(name, "expected status ACTIVE, got ${result.status}")
            return
        }
        if (result.user_count < 2) {
            fail(name, "expected count >= 2, got ${result.user_count}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testGetUserWithTags() {
    val name = "GetUserWithTags"
    try {
        transaction {
            exec("INSERT INTO tags (name) VALUES ('vip')")
            exec("INSERT INTO user_tags (user_id, tag_id) SELECT $createdUserId, id FROM tags WHERE name = 'vip'")
        }
        val rows = getUserWithTags(createdUserId)
        if (rows.size != 1) {
            fail(name, "expected 1 tagged row, got ${rows.size}")
            return
        }
        if (rows[0].tag_name != "vip") {
            fail(name, "expected tag_name vip, got ${rows[0].tag_name}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testSearchUsers() {
    val name = "SearchUsers"
    try {
        val users = searchUsers("Ali%")
        if (users.none { it.name == "Alice" }) {
            fail(name, "expected Alice among search results")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

// Board #197/#204: a nullable enum and a nullable composite column, each read
// both present and as SQL NULL, plus a composite field holding a double quote
// and a comma so the composite text parser's escaping is exercised rather
// than assumed. Seeded through raw exec because a composite literal is
// outside this backend's parameter-binding surface; the read path is what is
// under test and it runs entirely through generated code.
fun testGetUserProfileNullable() {
    val name = "GetUserProfile"
    try {
        var presentId = 0
        var absentId = 0
        var quotedId = 0
        transaction {
            exec(
                "INSERT INTO users (name, email, status, secondary_status, address) VALUES " +
                    "('Carol', 'carol@example.com', 'active', 'inactive', " +
                    "ROW('1 Main St', 'Springfield', '12345')) RETURNING id",
                explicitStatementType = StatementType.SELECT
            ) { rs -> if (rs.next()) presentId = rs.getInt(1); presentId }
            exec(
                "INSERT INTO users (name, email, status, secondary_status, address) VALUES " +
                    "('Dave', 'dave@example.com', 'active', NULL, NULL) RETURNING id",
                explicitStatementType = StatementType.SELECT
            ) { rs -> if (rs.next()) absentId = rs.getInt(1); absentId }
            exec(
                "INSERT INTO users (name, email, status, secondary_status, address) VALUES " +
                    "('Eve', 'eve@example.com', 'active', 'banned', " +
                    "ROW('12 \"Main\", Apt 3', 'Springfield', '12345')) RETURNING id",
                explicitStatementType = StatementType.SELECT
            ) { rs -> if (rs.next()) quotedId = rs.getInt(1); quotedId }
        }

        val present = getUserProfile(presentId)
        if (present.secondary_status != UserStatus.INACTIVE) {
            fail(name, "expected secondary_status INACTIVE, got ${present.secondary_status}")
            return
        }
        val address = present.address
        if (address == null) {
            fail(name, "expected address to be present")
            return
        }
        if (address.street != "1 Main St" || address.city != "Springfield" || address.zip != "12345") {
            fail(name, "unexpected address: $address")
            return
        }

        val absent = getUserProfile(absentId)
        if (absent.secondary_status != null) {
            fail(name, "expected null secondary_status, got ${absent.secondary_status}")
            return
        }
        if (absent.address != null) {
            fail(name, "expected null address, got ${absent.address}")
            return
        }

        val quoted = getUserProfile(quotedId)
        val quotedAddress = quoted.address
        if (quotedAddress == null) {
            fail(name, "expected an address for the quoted row")
            return
        }
        if (quotedAddress.street != "12 \"Main\", Apt 3") {
            fail(name, "composite escaping lost the quotes/comma: got ${quotedAddress.street}")
            return
        }

        transaction {
            exec("DELETE FROM users WHERE id IN ($presentId, $absentId, $quotedId)")
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testDeleteOrdersByUser() {
    val name = "DeleteOrdersByUser"
    try {
        val count = deleteOrdersByUser(createdUserId)
        if (count != 1) {
            fail(name, "expected 1 deleted order, got $count")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

fun testDeleteUser() {
    val name = "DeleteUser"
    try {
        transaction {
            exec("DELETE FROM user_tags WHERE user_id = $createdUserId")
        }
        deleteUser(createdUserId)
        deleteUser(bobId)
        try {
            getUserById(createdUserId)
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
