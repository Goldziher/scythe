import generated.Queries;

import java.nio.file.Files;
import java.nio.file.Path;
import java.sql.Connection;
import java.sql.DriverManager;

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
        try (Connection conn = DriverManager.getConnection("jdbc:sqlite::memory:")) {
            try (var stmt = conn.createStatement()) {
                stmt.execute("PRAGMA foreign_keys = ON");
            }

            runMigration(conn);

            testCreateUser(conn);
            testGetUserById(conn);
            testListActiveUsers(conn);
            testCreateOrder(conn);
            testGetOrdersByUser(conn);
            testDeleteOrdersByUser(conn);
            testDeleteUser(conn);
        }

        System.out.println();
        System.out.println("Results: " + passed + " passed, " + failed + " failed");
        if (failed > 0) {
            System.exit(1);
        }
        System.out.println("ALL TESTS PASSED");
    }

    private static void runMigration(Connection conn) throws Exception {
        Path schemaPath = Path.of(System.getProperty("user.dir"))
            .resolve("../sql/sqlite/schema.sql")
            .normalize();
        String schema = Files.readString(schemaPath);

        // SQLite supports executing multiple statements at once
        try (var stmt = conn.createStatement()) {
            stmt.execute("DROP TABLE IF EXISTS user_tags");
            stmt.execute("DROP TABLE IF EXISTS tags");
            stmt.execute("DROP TABLE IF EXISTS orders");
            stmt.execute("DROP TABLE IF EXISTS users");
        }

        for (String sql : schema.split(";")) {
            String trimmed = sql.trim();
            if (!trimmed.isEmpty()) {
                try (var stmt = conn.createStatement()) {
                    stmt.execute(trimmed);
                }
            }
        }
    }

    private static int createdUserId;

    private static void testCreateUser(Connection conn) {
        String name = "CreateUser";
        try {
            Queries.createUser(conn, "Alice", "alice@example.com", "active");
            // SQLite: use last_insert_rowid() to get the new user's id
            int lastId;
            try (var stmt = conn.createStatement();
                 var rs = stmt.executeQuery("SELECT last_insert_rowid()")) {
                rs.next();
                lastId = rs.getInt(1);
            }
            var user = Queries.getUserById(conn, lastId);
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

    private static void testGetUserById(Connection conn) {
        String name = "GetUserById";
        try {
            var user = Queries.getUserById(conn, createdUserId);
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

    private static void testListActiveUsers(Connection conn) {
        String name = "ListActiveUsers";
        try {
            var users = Queries.listActiveUsers(conn, "active");
            if (users.isEmpty()) {
                fail(name, "expected at least 1 active user");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testCreateOrder(Connection conn) {
        String name = "CreateOrder";
        try {
            Queries.createOrder(conn, createdUserId, 99.99f, "Test order");
            // SQLite: use last_insert_rowid() to get the new order's id
            int lastId;
            try (var stmt = conn.createStatement();
                 var rs = stmt.executeQuery("SELECT last_insert_rowid()")) {
                rs.next();
                lastId = rs.getInt(1);
            }
            // Verify via GetOrdersByUser
            var orders = Queries.getOrdersByUser(conn, createdUserId);
            if (orders.isEmpty()) {
                fail(name, "no orders found after creation");
                return;
            }
            var order = orders.get(0);
            if (Math.abs(order.total() - 99.99) > 0.01) {
                fail(name, "expected total 99.99, got " + order.total());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testGetOrdersByUser(Connection conn) {
        String name = "GetOrdersByUser";
        try {
            var orders = Queries.getOrdersByUser(conn, createdUserId);
            if (orders.size() != 1) {
                fail(name, "expected 1 order, got " + orders.size());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testDeleteOrdersByUser(Connection conn) {
        String name = "DeleteOrdersByUser";
        try {
            int count = Queries.deleteOrdersByUser(conn, createdUserId);
            if (count != 1) {
                fail(name, "expected 1 deleted order, got " + count);
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testDeleteUser(Connection conn) {
        String name = "DeleteUser";
        try {
            Queries.deleteUser(conn, createdUserId);
            var user = Queries.getUserById(conn, createdUserId);
            if (user != null) {
                fail(name, "expected null after deletion, but user still exists");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }
}
