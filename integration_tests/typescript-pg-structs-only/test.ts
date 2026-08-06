/**
 * structs_only emits only type declarations (interfaces/Zod schemas, enums,
 * composites) — no query functions and no driver import. There is no
 * runtime behavior to exercise here, so this file only proves (via
 * `tsc --noEmit`) that the emitted declarations type-check and wire
 * together correctly. The structural claim (no driver import, no query
 * function emitted) is asserted separately by grepping the generated
 * output — see the `structs-only-check` task.
 */
import type {
	UserStatus,
	CreateUserRow,
	GetUserByIdRow,
	ListActiveUsersRow,
	CreateOrderRow,
	GetOrdersByUserRow,
} from "./generated/queries.js";

function describeUserStatus(status: UserStatus): string {
	return `status: ${status}`;
}

function describeUser(row: GetUserByIdRow): string {
	return `${row.name} <${row.email ?? "no-email"}> (${describeUserStatus(row.status)})`;
}

function describeActiveUser(row: ListActiveUsersRow): string {
	return row.name;
}

function describeCreatedUser(row: CreateUserRow): string {
	return describeUser(row);
}

function describeOrder(row: CreateOrderRow): string {
	return `order for user ${row.user_id}: ${row.total}`;
}

function describeUserOrder(row: GetOrdersByUserRow): string {
	return `${row.total} (${row.notes ?? "no notes"})`;
}

function main(): void {
	const sample: GetUserByIdRow = {
		id: 1,
		name: "Alice",
		email: "alice@example.com",
		status: "active",
		created_at: new Date(),
	};
	console.log(describeUser(sample));
	console.log(describeActiveUser(sample));
	console.log(describeCreatedUser(sample));
	console.log(
		describeOrder({
			id: 1,
			user_id: 1,
			total: "99.95",
			notes: "first order",
			created_at: new Date(),
		}),
	);
	console.log(
		describeUserOrder({
			id: 1,
			total: "99.95",
			notes: "first order",
			created_at: new Date(),
		}),
	);
	console.log("PASS: structs_only declarations type-check and compose");
}

main();
