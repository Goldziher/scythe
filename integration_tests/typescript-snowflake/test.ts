import snowflake, { type Connection } from "snowflake-sdk";
import {
	createUser,
	getUserById,
	listActiveUsers,
	createOrder,
	getOrdersByUser,
	deleteOrdersByUser,
	deleteUser,
} from "./generated/queries.js";

const DATABASE_URL =
	process.env["SNOWFLAKE_URL"] ??
	"snowflake://account:password@host/database/schema";


let exitCode = 0;
const failedTests = new Set<string>();

function assert(condition: boolean, testName: string, detail: string): void {
	if (!condition) {
		console.error(`FAIL: ${testName}: ${detail}`);
		exitCode = 1;
		failedTests.add(testName);
	}
}

function pass(testName: string, label: string = testName): void {
	if (!failedTests.has(testName)) {
		console.log(`PASS: ${label}`);
	}
}


// Splits a SQL script into individual statements on top-level `;` only --
// unlike a naive `.split(";")`, this tracks single-quoted and
// double-quoted spans, PostgreSQL dollar-quoted bodies, and `--` line
// comments (an apostrophe in a comment must not open a phantom string --
// board #224 follow-up) so a `;` inside a string literal, a `$$ ... $$`
// function body, or a comment does not split the statement in half.
// `/* ... */` block comments are not handled -- no schema under
// integration_tests/sql/ uses them today.
function splitSqlStatements(sql: string): string[] {
	const statements: string[] = [];
	let current = "";
	let inSingle = false;
	let inDouble = false;
	let inLineComment = false;
	let dollarTag: string | null = null;
	for (let i = 0; i < sql.length; i++) {
		const ch = sql[i];
		if (inLineComment) {
			current += ch;
			if (ch === "\n") inLineComment = false;
			continue;
		}
		if (dollarTag !== null) {
			current += ch;
			if (ch === "$" && sql.startsWith(dollarTag, i)) {
				current += dollarTag.slice(1);
				i += dollarTag.length - 1;
				dollarTag = null;
			}
			continue;
		}
		if (inSingle) {
			current += ch;
			if (ch === "'") inSingle = false;
			continue;
		}
		if (inDouble) {
			current += ch;
			if (ch === '"') inDouble = false;
			continue;
		}
		if (ch === "-" && sql[i + 1] === "-") {
			inLineComment = true;
			current += ch;
			continue;
		}
		if (ch === "'") {
			inSingle = true;
			current += ch;
			continue;
		}
		if (ch === '"') {
			inDouble = true;
			current += ch;
			continue;
		}
		if (ch === "$") {
			const match = /^\$[A-Za-z0-9_]*\$/.exec(sql.slice(i));
			if (match) {
				dollarTag = match[0];
				current += dollarTag;
				i += dollarTag.length - 1;
				continue;
			}
		}
		if (ch === ";") {
			statements.push(current);
			current = "";
			continue;
		}
		current += ch;
	}
	if (current.trim() !== "") statements.push(current);
	return statements.map((s) => s.trim()).filter(Boolean);
}

function connect(conn: Connection): Promise<void> {
	return new Promise((resolve, reject) => {
		conn.connect((err) => (err ? reject(err) : resolve()));
	});
}

function execute(conn: Connection, sqlText: string): Promise<void> {
	return new Promise((resolve, reject) => {
		conn.execute({
			sqlText,
			complete: (err) => (err ? reject(err) : resolve()),
		});
	});
}

function destroy(conn: Connection): Promise<void> {
	return new Promise((resolve, reject) => {
		conn.destroy((err) => (err ? reject(err) : resolve()));
	});
}

async function main(): Promise<void> {
	const { fileURLToPath, URL } = await import("node:url");
	const parsed = new URL(DATABASE_URL);
	const protocol = parsed.searchParams.get("protocol");
	const accessUrl = protocol
		? `${protocol}://${parsed.host}`
		: undefined;
	const [, database = "testdb", schema = "public"] = parsed.pathname.split("/");
	const conn = snowflake.createConnection({
		account: parsed.searchParams.get("account") ?? parsed.hostname,
		username: parsed.username,
		password: parsed.password,
		database,
		schema,
		...(accessUrl ? { accessUrl } : {}),
	});
	try {
		await connect(conn);

		// Clean slate: drop tables
		for (const table of ["user_tags", "tags", "orders", "users"]) {
			await execute(conn, `DROP TABLE IF EXISTS ${table}`);
		}

		// Load and execute schema
		const { readFile } = await import("node:fs/promises");
		const schemaPath = fileURLToPath(
			new URL("../sql/snowflake/schema.sql", import.meta.url),
		);
		const schemaSql = await readFile(schemaPath, "utf8");
		for (const stmt of splitSqlStatements(schemaSql)) {
			await execute(conn, stmt);
		}

		// Test: CreateUser
		await createUser(conn, "Alice", "alice@example.com", true);
		const users = await listActiveUsers(conn);
		const user = users.find((u) => u.name === "Alice");
		assert(user !== undefined, "CreateUser", "user should not be undefined");
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
		pass("CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(conn, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		pass("GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(conn);
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
		pass("ListActiveUsers");

		// Test: CreateOrder
		await createOrder(conn, userId, 99.95, "first order");
		const orders = await getOrdersByUser(conn, userId);
		const order = orders[0];
		assert(order !== undefined, "CreateOrder", "order should not be undefined");
		assert(
			Number(order!.total) === 99.95,
			"CreateOrder",
			`expected total 99.95, got ${order!.total}`,
		);
		assert(
			order!.notes === "first order",
			"CreateOrder",
			`expected notes 'first order'`,
		);
		pass("CreateOrder");

		// Test: GetOrdersByUser
		const allOrders = await getOrdersByUser(conn, userId);
		assert(
			allOrders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${allOrders.length}`,
		);
		assert(
			Number(allOrders[0]!.total) === 99.95,
			"GetOrdersByUser",
			`expected total 99.95`,
		);
		pass("GetOrdersByUser");

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(conn, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(conn, userId);
		// GetUserById is `:one`, which errors on a missing row rather than
		// returning null. Absence is therefore observed as a throw, and
		// `gone === null` would never be reached. The flag is what makes the
		// assertion positive: a bare try/catch would pass whether or not
		// anything was thrown.
		let goneThrew = false;
		try {
			await getUserById(conn, userId);
		} catch {
			goneThrew = true;
		}
		assert(goneThrew, "DeleteUser", "user should not be found after deletion");
		pass("DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		await destroy(conn);
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
