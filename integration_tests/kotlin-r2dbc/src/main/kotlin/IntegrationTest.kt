import generated.*
import io.r2dbc.postgresql.PostgresqlConnectionConfiguration
import io.r2dbc.postgresql.PostgresqlConnectionFactory
import io.r2dbc.spi.ConnectionFactory
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import java.math.BigDecimal
import java.nio.file.Path
import java.sql.DriverManager
import kotlin.io.path.readText
import kotlin.system.exitProcess

// This harness exercises the reactive kotlin-r2dbc backend (non-extension-
// function mode: every generated query is a `suspend fun name(cf: ConnectionFactory, ...)`
// -- see kotlin_r2dbc.rs). `runBlocking { ... }` is the subscription point for
// every test: a suspend function backed by `Mono.from(...).awaitFirst()` (or
// `.awaitFirstOrNull()`, or `.asFlow()`) does nothing until its coroutine is
// actually run, and `runBlocking` is what runs it, on the calling thread,
// synchronously, propagating any exception straight out. Every assertion
// below sits inside the `runBlocking` block that produced the value it
// checks, so a wrong expected value fails the assertion it is paired with --
// there is no suspend call left dangling without being run.
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
    val database = uri.path.removePrefix("/")

    DriverManager.getConnection(jdbcUrl, user, password).use { conn ->
        runMigration(conn)
    }

    val cf: ConnectionFactory = PostgresqlConnectionFactory(
        PostgresqlConnectionConfiguration.builder()
            .host(uri.host)
            .port(uri.port)
            .username(user)
            .password(password)
            .database(database)
            .build()
    )

    runBlocking {
        testCreateUser(cf)
        testGetUserById(cf)
        testUpdateUserEmail(cf)
        testCreateOrder(cf)
        testGetOrdersByUser(cf)
        testGetOrderTotal(cf)
        testListActiveUsers(cf)
        testGetUserOrders(cf)
        testCountUsersByStatus(cf)
        testSearchUsers(cf)
        testRoundTripUserAddress(cf)
        // ~keep Runs after every order-counting assertion and immediately before the delete that
        // cleans both orders up: it adds a second order, and `orders.user_id` has no ON DELETE
        // CASCADE, so it cannot run any later than this without stranding a row that DeleteUser
        // would then trip over.
        testCreateOrderWithNullNotes(cf)
        testDeleteOrdersByUser(cf)
        testDeleteUser(cf)
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

suspend fun testCreateUser(cf: ConnectionFactory) {
    val name = "CreateUser"
    try {
        // createUser(cf, ...) is a suspend fun; calling it here inside
        // runBlocking's coroutine is what actually runs the R2DBC statement
        // and awaits its result -- change ACTIVE, or any check below, and the
        // assertion sees the real row this call produced.
        val user = createUser(cf, "Alice", "alice@example.com", UserStatus.ACTIVE)
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

suspend fun testGetUserById(cf: ConnectionFactory) {
    val name = "GetUserById"
    try {
        val user = getUserById(cf, createdUserId)
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

suspend fun testUpdateUserEmail(cf: ConnectionFactory) {
    val name = "UpdateUserEmail"
    try {
        updateUserEmail(cf, "alice-updated@example.com", createdUserId)
        val user = getUserById(cf, createdUserId)
        if (user.email != "alice-updated@example.com") {
            fail(name, "expected updated email, got ${user.email}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testCreateOrder(cf: ConnectionFactory) {
    val name = "CreateOrder"
    try {
        val order = createOrder(cf, createdUserId, BigDecimal("99.99"), "Test order")
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

// ~keep board #229: `notes` is a nullable String param bound through R2DBC's
// `Statement.bind`/`bindNull` split. `Statement.bind(index, Any)` throws
// `IllegalArgumentException` for a null value -- only `bindNull(index, Class<*>)` may send SQL
// NULL -- so a generator that always emits `bind` regardless of nullability throws here, not at
// the database, the moment a caller passes null. Every other harness call above always passes a
// non-null literal for every nullable parameter, so none of them can catch that regression; this
// is the one call in the file that actually exercises the null path.
suspend fun testCreateOrderWithNullNotes(cf: ConnectionFactory) {
    val name = "CreateOrderWithNullNotes"
    try {
        val order = createOrder(cf, createdUserId, BigDecimal("50.00"), null)
        if (order.notes != null) {
            fail(name, "expected notes null, got ${order.notes}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testGetOrdersByUser(cf: ConnectionFactory) {
    val name = "GetOrdersByUser"
    try {
        // getOrdersByUser returns Flow<OrderRow>; toList() is a suspend
        // terminal operator that collects the flow (i.e. subscribes to the
        // underlying Flux) before the size check runs.
        val orders = getOrdersByUser(cf, createdUserId).toList()
        if (orders.size != 1) {
            fail(name, "expected 1 order, got ${orders.size}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testGetOrderTotal(cf: ConnectionFactory) {
    val name = "GetOrderTotal"
    try {
        val result = getOrderTotal(cf, createdUserId)
        if (result.total_sum == null) {
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

suspend fun testListActiveUsers(cf: ConnectionFactory) {
    val name = "ListActiveUsers"
    try {
        val users = listActiveUsers(cf, UserStatus.ACTIVE).toList()
        if (users.isEmpty()) {
            fail(name, "expected at least 1 active user")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testGetUserOrders(cf: ConnectionFactory) {
    val name = "GetUserOrders"
    try {
        val results = getUserOrders(cf, UserStatus.ACTIVE).toList()
        if (results.isEmpty()) {
            fail(name, "expected at least 1 result")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testCountUsersByStatus(cf: ConnectionFactory) {
    val name = "CountUsersByStatus"
    try {
        val result = countUsersByStatus(cf, UserStatus.ACTIVE)
        if (result.user_count < 1) {
            fail(name, "expected count >= 1, got ${result.user_count}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testSearchUsers(cf: ConnectionFactory) {
    val name = "SearchUsers"
    try {
        val users = searchUsers(cf, "%Alice%").toList()
        if (users.isEmpty()) {
            fail(name, "expected at least 1 user matching Alice")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testDeleteOrdersByUser(cf: ConnectionFactory) {
    val name = "DeleteOrdersByUser"
    try {
        val count = deleteOrdersByUser(cf, createdUserId)
        // ~keep Two, not one: testCreateOrderWithNullNotes adds a second order just above.
        // Asserting the exact count is what makes this the cleanup step for both.
        if (count != 2L) {
            fail(name, "expected 2 deleted orders, got $count")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}

suspend fun testDeleteUser(cf: ConnectionFactory) {
    val name = "DeleteUser"
    try {
        deleteUser(cf, createdUserId)
        // getUserById is `:one`: a missing row's Mono errors with
        // NoSuchElementException, and awaitFirst() rethrows that error into
        // this suspend function, which the catch below turns into the
        // expected pass.
        try {
            getUserById(cf, createdUserId)
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

suspend fun testRoundTripUserAddress(cf: ConnectionFactory) {
    val name = "RoundTripUserAddress"
    try {
        val address = UserAddress("12 \"Main\", Apt \\3", "", "10115")
        val present = roundTripUserAddress(cf, address)
        if (present.address != address) {
            fail(name, "escaped composite did not round-trip: ${present.address}")
            return
        }
        val absent = roundTripUserAddress(cf, null)
        if (absent.address != null) {
            fail(name, "whole-composite NULL did not round-trip: ${absent.address}")
            return
        }
        pass(name)
    } catch (e: Exception) {
        fail(name, e)
    }
}
