import mysql from "mysql2/promise";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
	getLastInsertUser,
	getLastInsertOrder,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["DATABASE_URL"] ??
	"mysql://scythe:scythe@localhost:3306/scythe_test";

const pool = mysql.createPool(DATABASE_URL);

let exitCode = 0;

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
	}
}

async function main(): Promise<void> {
	try {
		// Clean slate
		await pool.execute("DROP TABLE IF EXISTS user_tags");
		await pool.execute("DROP TABLE IF EXISTS tags");
		await pool.execute("DROP TABLE IF EXISTS orders");
		await pool.execute("DROP TABLE IF EXISTS users");

		await pool.execute(`CREATE TABLE users (
      id INT AUTO_INCREMENT PRIMARY KEY,
      name VARCHAR(255) NOT NULL,
      email VARCHAR(255),
      status ENUM('active', 'inactive', 'banned') NOT NULL DEFAULT 'active',
      created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
    )`);
		await pool.execute(`CREATE TABLE orders (
      id INT AUTO_INCREMENT PRIMARY KEY,
      user_id INT NOT NULL,
      total DECIMAL(10, 2) NOT NULL,
      notes TEXT,
      created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
      FOREIGN KEY (user_id) REFERENCES users (id)
    )`);
		await pool.execute(`CREATE TABLE tags (
      id INT AUTO_INCREMENT PRIMARY KEY,
      name VARCHAR(255) NOT NULL UNIQUE
    )`);
		await pool.execute(`CREATE TABLE user_tags (
      user_id INT NOT NULL,
      tag_id INT NOT NULL,
      PRIMARY KEY (user_id, tag_id),
      FOREIGN KEY (user_id) REFERENCES users (id),
      FOREIGN KEY (tag_id) REFERENCES tags (id)
    )`);

		// Test: CreateUser + GetLastInsertUser
		await createUser(pool, "Alice", "alice@example.com", "active");
		const user = await getLastInsertUser(pool);
		assert(user !== null, "CreateUser", "user should not be null");
		assert(
			user!.name === "Alice",
			"CreateUser",
			`expected name Alice, got ${user!.name}`,
		);
		assert(
			user!.email === "alice@example.com",
			"CreateUser",
			`expected email alice@example.com`,
		);
		const userId = user!.id;
		console.log("PASS: CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(pool, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		console.log("PASS: GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(pool, "active");
		assert(
			activeUsers.length > 0,
			"ListActiveUsers",
			"should have at least one user",
		);
		assert(
			activeUsers[0]!.name === "Alice",
			"ListActiveUsers",
			"first user should be Alice",
		);
		console.log("PASS: ListActiveUsers");

		// Test: CreateOrder + GetLastInsertOrder
		await createOrder(pool, userId, "99.95", "first order");
		const order = await getLastInsertOrder(pool);
		assert(order !== null, "CreateOrder", "order should not be null");
		assert(
			order!.user_id === userId,
			"CreateOrder",
			`expected user_id ${userId}`,
		);
		assert(
			order!.total === "99.95",
			"CreateOrder",
			`expected total 99.95, got ${order!.total}`,
		);
		assert(
			order!.notes === "first order",
			"CreateOrder",
			`expected notes 'first order'`,
		);
		console.log("PASS: CreateOrder");

		// Test: GetOrdersByUser
		const orders = await getOrdersByUser(pool, userId);
		assert(
			orders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${orders.length}`,
		);
		console.log("PASS: GetOrdersByUser");

		// Test: DeleteOrdersByUser
		const deletedOrders = await deleteOrdersByUser(pool, userId);
		assert(
			deletedOrders === 1,
			"DeleteOrdersByUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(pool, userId);
		const gone = await getUserById(pool, userId);
		assert(gone === null, "DeleteUser", "user should be null after deletion");
		console.log("PASS: DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		await pool.end();
	}

	process.exit(exitCode);
}

// Force exit after 10 seconds if pool.end() hangs
const forceExitTimer = setTimeout(() => process.exit(exitCode), 10_000);
forceExitTimer.unref();

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
