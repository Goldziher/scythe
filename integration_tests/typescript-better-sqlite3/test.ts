import Database from "better-sqlite3";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
} from "./generated/queries.js";

const db = new Database(":memory:");

let exitCode = 0;

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
	}
}

function main(): void {
	// Enable WAL mode and foreign keys
	db.pragma("journal_mode = WAL");
	db.pragma("foreign_keys = ON");

	// Create schema
	db.exec(`CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    email TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'inactive', 'banned')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
  )`);
	db.exec(`CREATE TABLE orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users (id),
    total REAL NOT NULL,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
  )`);
	db.exec(`CREATE TABLE tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
  )`);
	db.exec(`CREATE TABLE user_tags (
    user_id INTEGER NOT NULL REFERENCES users (id),
    tag_id INTEGER NOT NULL REFERENCES tags (id),
    PRIMARY KEY (user_id, tag_id)
  )`);

	// Test: CreateUser
	createUser(db, "Alice", "alice@example.com", "active");
	const userId = Number(
		db.prepare("SELECT last_insert_rowid() AS id").get()!["id" as keyof object],
	);
	const user = getUserById(db, userId);
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
	console.log("PASS: CreateUser");

	// Test: GetUserById
	const fetched = getUserById(db, userId);
	assert(fetched !== null, "GetUserById", "user should not be null");
	assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
	assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
	console.log("PASS: GetUserById");

	// Test: ListActiveUsers
	const activeUsers = listActiveUsers(db, "active");
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

	// Test: CreateOrder
	createOrder(db, userId, 99.95, "first order");
	const orderId = Number(
		db.prepare("SELECT last_insert_rowid() AS id").get()!["id" as keyof object],
	);
	const orders = getOrdersByUser(db, userId);
	assert(
		orders.length === 1,
		"GetOrdersByUser",
		`expected 1 order, got ${orders.length}`,
	);
	assert(orders[0]!.total === 99.95, "GetOrdersByUser", `expected total 99.95`);
	console.log("PASS: CreateOrder");

	// Test: GetOrdersByUser
	console.log("PASS: GetOrdersByUser");

	// Test: DeleteOrdersByUser
	const deletedOrders = deleteOrdersByUser(db, userId);
	assert(
		deletedOrders === 1,
		"DeleteOrdersByUser",
		`expected 1 deleted order, got ${deletedOrders}`,
	);
	deleteUser(db, userId);
	const gone = getUserById(db, userId);
	assert(gone === null, "DeleteUser", "user should be null after deletion");
	console.log("PASS: DeleteUser");

	if (exitCode === 0) {
		console.log("ALL TESTS PASSED");
	}

	db.close();
	process.exit(exitCode);
}

main();
