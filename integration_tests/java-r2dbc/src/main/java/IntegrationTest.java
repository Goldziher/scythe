import generated.Queries;
import generated.Queries.UserStatus;
import io.r2dbc.postgresql.PostgresqlConnectionConfiguration;
import io.r2dbc.postgresql.PostgresqlConnectionFactory;
import io.r2dbc.spi.ConnectionFactory;

import java.math.BigDecimal;
import java.nio.file.Files;
import java.nio.file.Path;
import java.sql.Connection;
import java.sql.DriverManager;
import java.util.NoSuchElementException;

// This harness exercises the reactive java-r2dbc backend. Every query
// function returns a Mono<T>/Flux<T> that does nothing until subscribed --
// `Mono.usingWhen(...).flatMap(...)` builds a cold pipeline, it does not run
// one. `.block()` is the subscription point in every test below: it blocks
// the calling thread until the publisher completes (or errors) and returns
// the terminal value, which is exactly what a synchronous, sequential
// integration test needs. Every assertion in this file runs *after* a
// `.block()` call on the expression it is checking, inside the same method,
// so a wrong `.block()`ed value fails the assertion that follows it -- there
// is no floating assertion beside an unsubscribed publisher anywhere here.
//
// Schema setup and row-count verification (runMigration, and any raw-JDBC
// probe queries) still use blocking `java.sql.DriverManager` -- R2DBC has no
// schema-migration story, and mixing one blocking JDBC connection for setup
// with the reactive connection factory under test is the same pattern
// java-jdbc's own migration helper already establishes for this generator.
public class IntegrationTest {

    private static int passed = 0;
    private static int failed = 0;

    private static void pass(String name) {
        System.out.println("PASS: " + name);
        passed++;
    }

    private static void fail(String name, Exception e) {
        System.out.println("FAIL: " + name + " - " + e.getMessage());
        failed++;
    }

    private static void fail(String name, String message) {
        System.out.println("FAIL: " + name + " - " + message);
        failed++;
    }

    public static void main(String[] args) throws Exception {
        String databaseUrl = System.getenv("DATABASE_URL");
        if (databaseUrl == null || databaseUrl.isEmpty()) {
            System.err.println("DATABASE_URL environment variable is required");
            System.exit(1);
        }

        java.net.URI uri = new java.net.URI(databaseUrl);
        String userInfo = uri.getUserInfo();
        String user = userInfo != null ? userInfo.split(":")[0] : "";
        String password = userInfo != null && userInfo.contains(":") ? userInfo.split(":")[1] : "";
        String jdbcUrl = "jdbc:postgresql://" + uri.getHost() + ":" + uri.getPort() + uri.getPath();
        String database = uri.getPath().startsWith("/") ? uri.getPath().substring(1) : uri.getPath();

        try (Connection migrationConn = DriverManager.getConnection(jdbcUrl, user, password)) {
            runMigration(migrationConn);
        }

        ConnectionFactory cf = new PostgresqlConnectionFactory(
            PostgresqlConnectionConfiguration.builder()
                .host(uri.getHost())
                .port(uri.getPort())
                .username(user)
                .password(password)
                .database(database)
                .build()
        );

        testCreateUser(cf);
        testGetUserById(cf);
        testUpdateUserEmail(cf);
        testCreateOrder(cf);
        testGetOrdersByUser(cf);
        testGetOrderTotal(cf);
        testListActiveUsers(cf);
        testGetUserOrders(cf);
        testCountUsersByStatus(cf);
        testSearchUsers(cf);
        testRoundTripUserAddress(cf);
        // ~keep Runs after every order-counting assertion and immediately before the delete that
        // cleans both orders up: it adds a second order, and `orders.user_id` has no ON DELETE
        // CASCADE, so it cannot run any later than this without stranding a row that DeleteUser
        // would then trip over.
        testCreateOrderWithNullNotes(cf);
        testDeleteOrdersByUser(cf);
        testDeleteUser(cf);

        System.out.println();
        System.out.println("Results: " + passed + " passed, " + failed + " failed");
        if (failed > 0) {
            System.exit(1);
        }
        System.out.println("ALL TESTS PASSED");
    }

    private static void runMigration(Connection conn) throws Exception {
        Path schemaPath = Path.of(System.getProperty("user.dir"))
            .resolve("../sql/pg/schema.sql")
            .normalize();
        String schema = Files.readString(schemaPath);

        String dropSql = """
            DROP TABLE IF EXISTS user_tags CASCADE;
            DROP TABLE IF EXISTS tags CASCADE;
            DROP TABLE IF EXISTS orders CASCADE;
            DROP TABLE IF EXISTS users CASCADE;
            DROP TYPE IF EXISTS user_status CASCADE;
            DROP TYPE IF EXISTS user_address CASCADE;
            """;

        try (var stmt = conn.createStatement()) {
            stmt.execute(dropSql);
            stmt.execute(schema);
        }
    }

    private static int createdUserId;

    private static void testCreateUser(ConnectionFactory cf) {
        String name = "CreateUser";
        try {
            // .block() is the subscription: without it, createUser(...) returns an
            // unsubscribed Mono<UserRow> and nothing below would ever run -- the
            // driver would never even open a connection. Change ACTIVE below (or
            // the assertions after) and the assertion sees the real materialized
            // row, so a wrong expected value fails here, not silently passes.
            var user = Queries.createUser(cf, "Alice", "alice@example.com", UserStatus.ACTIVE).block();
            if (user == null) {
                fail(name, "returned null");
                return;
            }
            if (!"Alice".equals(user.name())) {
                fail(name, "expected name Alice, got " + user.name());
                return;
            }
            if (!"alice@example.com".equals(user.email())) {
                fail(name, "expected email alice@example.com, got " + user.email());
                return;
            }
            createdUserId = user.id();
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testGetUserById(ConnectionFactory cf) {
        String name = "GetUserById";
        try {
            var user = Queries.getUserById(cf, createdUserId).block();
            if (user == null) {
                fail(name, "returned null");
                return;
            }
            if (!"Alice".equals(user.name())) {
                fail(name, "expected name Alice, got " + user.name());
                return;
            }
            if (user.id() != createdUserId) {
                fail(name, "expected id " + createdUserId + ", got " + user.id());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testUpdateUserEmail(ConnectionFactory cf) {
        String name = "UpdateUserEmail";
        try {
            // updateUserEmail returns Mono<Void>; .block() waits for completion
            // (or propagates its error) before the follow-up read runs, so the
            // read below cannot race the write.
            Queries.updateUserEmail(cf, "alice-updated@example.com", createdUserId).block();
            var user = Queries.getUserById(cf, createdUserId).block();
            if (user == null) {
                fail(name, "user not found after update");
                return;
            }
            if (!"alice-updated@example.com".equals(user.email())) {
                fail(name, "expected updated email, got " + user.email());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testCreateOrder(ConnectionFactory cf) {
        String name = "CreateOrder";
        try {
            var order = Queries.createOrder(cf, createdUserId, new BigDecimal("99.99"), "Test order").block();
            if (order == null) {
                fail(name, "returned null");
                return;
            }
            if (order.user_id() != createdUserId) {
                fail(name, "expected user_id " + createdUserId + ", got " + order.user_id());
                return;
            }
            if (order.total().compareTo(new BigDecimal("99.99")) != 0) {
                fail(name, "expected total 99.99, got " + order.total());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    // ~keep board #229: `notes` is a nullable String param bound through R2DBC's
    // `Statement.bind`/`bindNull` split. `Statement.bind(index, Object)` throws
    // `IllegalArgumentException` for a null value -- only `bindNull(index, Class<?>)` may send SQL
    // NULL -- so a generator that always emits `bind` regardless of nullability throws here, not
    // at the database, the moment a caller passes null. Every other harness call above always
    // passes a non-null literal for every nullable parameter, so none of them can catch that
    // regression; this is the one call in the file that actually exercises the null path.
    private static void testCreateOrderWithNullNotes(ConnectionFactory cf) {
        String name = "CreateOrderWithNullNotes";
        try {
            var order = Queries.createOrder(cf, createdUserId, new BigDecimal("50.00"), null).block();
            if (order == null) {
                fail(name, "returned null");
                return;
            }
            if (order.notes() != null) {
                fail(name, "expected notes null, got " + order.notes());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testGetOrdersByUser(ConnectionFactory cf) {
        String name = "GetOrdersByUser";
        try {
            // getOrdersByUser returns Flux<OrderRow>; .collectList().block()
            // subscribes and drains the whole stream into a List before the
            // size assertion runs.
            var orders = Queries.getOrdersByUser(cf, createdUserId).collectList().block();
            if (orders == null || orders.size() != 1) {
                fail(name, "expected 1 order, got " + (orders == null ? "null" : orders.size()));
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testGetOrderTotal(ConnectionFactory cf) {
        String name = "GetOrderTotal";
        try {
            var result = Queries.getOrderTotal(cf, createdUserId).block();
            if (result == null || result.total_sum() == null) {
                fail(name, "returned null");
                return;
            }
            if (result.total_sum().compareTo(new BigDecimal("99.99")) != 0) {
                fail(name, "expected total_sum 99.99, got " + result.total_sum());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testListActiveUsers(ConnectionFactory cf) {
        String name = "ListActiveUsers";
        try {
            var users = Queries.listActiveUsers(cf, UserStatus.ACTIVE).collectList().block();
            if (users == null || users.isEmpty()) {
                fail(name, "expected at least 1 active user");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testGetUserOrders(ConnectionFactory cf) {
        String name = "GetUserOrders";
        try {
            // GetUserOrders is `:many` (Flux<T>), not `:grouped` -- Flux has no
            // `.block()` overload (only Mono does), so this must collect first.
            var results = Queries.getUserOrders(cf, UserStatus.ACTIVE).collectList().block();
            if (results == null || results.isEmpty()) {
                fail(name, "expected at least 1 result");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testCountUsersByStatus(ConnectionFactory cf) {
        String name = "CountUsersByStatus";
        try {
            var result = Queries.countUsersByStatus(cf, UserStatus.ACTIVE).block();
            if (result == null) {
                fail(name, "returned null");
                return;
            }
            if (result.user_count() < 1) {
                fail(name, "expected count >= 1, got " + result.user_count());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testSearchUsers(ConnectionFactory cf) {
        String name = "SearchUsers";
        try {
            var users = Queries.searchUsers(cf, "%Alice%").collectList().block();
            if (users == null || users.isEmpty()) {
                fail(name, "expected at least 1 user matching Alice");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testDeleteOrdersByUser(ConnectionFactory cf) {
        String name = "DeleteOrdersByUser";
        try {
            long count = Queries.deleteOrdersByUser(cf, createdUserId).block();
            // ~keep Two, not one: testCreateOrderWithNullNotes adds a second order just above.
            // Asserting the exact count is what makes this the cleanup step for both.
            if (count != 2) {
                fail(name, "expected 2 deleted orders, got " + count);
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testDeleteUser(ConnectionFactory cf) {
        String name = "DeleteUser";
        try {
            Queries.deleteUser(cf, createdUserId).block();
            // GetUserById is `:one`, so a missing row's Mono errors with
            // NoSuchElementException; .block() rethrows that error on this
            // thread, which the catch below turns into the expected pass.
            try {
                Queries.getUserById(cf, createdUserId).block();
                fail(name, "expected getUserById to throw after deletion, but it returned a row");
                return;
            } catch (RuntimeException expected) {
                if (!(expected.getCause() instanceof NoSuchElementException)
                    && !(expected instanceof NoSuchElementException)) {
                    throw expected;
                }
                // expected: the user was deleted
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testRoundTripUserAddress(ConnectionFactory cf) {
        String name = "RoundTripUserAddress";
        try {
            var address = new Queries.UserAddress("12 \"Main\", Apt \\3", "", "10115");
            var present = Queries.roundTripUserAddress(cf, address).block();
            if (present == null || !address.equals(present.address())) {
                fail(name, "escaped composite did not round-trip: " + present);
                return;
            }
            var absent = Queries.roundTripUserAddress(cf, null).block();
            if (absent == null || absent.address() != null) {
                fail(name, "whole-composite NULL did not round-trip: " + absent);
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }
}
