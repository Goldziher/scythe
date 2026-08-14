import { Kysely, MssqlDialect, sql } from "kysely";
import * as Tarn from "tarn";
import * as Tedious from "tedious";
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
	process.env["MSSQL_URL"] ??
	"sqlserver://sa:Scythe_Test1@localhost:1433?database=scythe_test";

const mssqlUrl = new URL(DATABASE_URL);
const db = new Kysely<any>({
	dialect: new MssqlDialect({
		tarn: { ...Tarn, options: { min: 0, max: 10 } },
		tedious: {
			...Tedious,
			connectionFactory: () =>
				new Tedious.Connection({
					server: mssqlUrl.hostname,
					authentication: {
						type: "default",
						options: {
							userName: mssqlUrl.username,
							password: decodeURIComponent(mssqlUrl.password),
						},
					},
					options: {
						port: parseInt(mssqlUrl.port) || 1433,
						database: mssqlUrl.searchParams.get("database") || "master",
						trustServerCertificate: true,
					},
				}),
		},
	}),
});

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


async function main(): Promise<void> {
	try {
		// Clean slate
		for (const table of ["user_tags", "tags", "orders", "users"]) {
			await sql.raw(`IF OBJECT_ID('${table}', 'U') IS NOT NULL DROP TABLE ${table}`).execute(db);
		}

		const { readFile } = await import("node:fs/promises");
		const schemaPath = new URL("../sql/mssql/schema.sql", import.meta.url).pathname;
		const schemaSql = await readFile(schemaPath, "utf8");
		for (const stmt of splitSqlStatements(schemaSql)) {
			await sql.raw(stmt).execute(db);
		}

		// Test: CreateUser
		const user = await createUser(db, 1, "Alice", "alice@example.com", true);
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
		pass("CreateUser");

		// Test: GetUserById
		const fetched = await getUserById(db, userId);
		assert(fetched !== null, "GetUserById", "user should not be null");
		assert(fetched!.id === userId, "GetUserById", `expected id ${userId}`);
		assert(fetched!.name === "Alice", "GetUserById", `expected name Alice`);
		pass("GetUserById");

		// Test: ListActiveUsers
		const activeUsers = await listActiveUsers(db);
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
		const order = await createOrder(db, 1, userId, "99.95", "first order");
		assert(order !== null, "CreateOrder", "order should not be null");
		assert(
			order!.user_id === userId,
			"CreateOrder",
			`expected user_id ${userId}`,
		);
		assert(
			String(order!.total) === "99.95",
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
		const orders = await getOrdersByUser(db, userId);
		assert(
			orders.length === 1,
			"GetOrdersByUser",
			`expected 1 order, got ${orders.length}`,
		);
		assert(
			String(orders[0]!.total) === "99.95",
			"GetOrdersByUser",
			`expected total 99.95`,
		);
		pass("GetOrdersByUser");

		// Test: DeleteUser
		const deletedOrders = await deleteOrdersByUser(db, userId);
		assert(
			deletedOrders === 1,
			"DeleteUser",
			`expected 1 deleted order, got ${deletedOrders}`,
		);
		await deleteUser(db, userId);
		// GetUserById is `:one`, which errors on a missing row rather than
		// returning null. Absence is therefore observed as a throw, and
		// `gone === null` would never be reached. The flag is what makes the
		// assertion positive: a bare try/catch would pass whether or not
		// anything was thrown.
		let goneThrew = false;
		try {
			await getUserById(db, userId);
		} catch {
			goneThrew = true;
		}
		assert(goneThrew, "DeleteUser", "user should not be found after deletion");
		pass("DeleteUser");

		if (exitCode === 0) {
			console.log("ALL TESTS PASSED");
		}
	} finally {
		await db.destroy();
	}

	process.exit(exitCode);
}

main().catch((error) => {
	console.error("Fatal error:", error);
	process.exit(1);
});
