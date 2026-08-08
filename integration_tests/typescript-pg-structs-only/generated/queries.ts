// scythe:provenance v=0.14.0 backend=typescript-pg engine=postgresql schema=sch1:2e813606acee8b51
export const UserStatusValues = {
	Active: "active",
	Inactive: "inactive",
	Banned: "banned",
} as const;

export type UserStatus = typeof UserStatusValues[keyof typeof UserStatusValues];

/** Row type for CreateOrder queries. */
export interface CreateOrderRow {
	id: number;
	user_id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}



/** Row type for GetOrdersByUser queries. */
export interface GetOrdersByUserRow {
	id: number;
	total: string;
	notes: string | null;
	created_at: Date;
}



/** Row type for GetOrderTotal queries. */
export interface GetOrderTotalRow {
	total_sum: string | null;
}



/** Row type for GetOrderWeightTotal queries. */
export interface GetOrderWeightTotalRow {
	weight_total: number | null;
}





/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	created_at: Date;
}



/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow {
	id: number;
	name: string;
	email: string | null;
}



/** Row type for CreateUser queries. */
export interface CreateUserRow {
	id: number;
	name: string;
	email: string | null;
	status: UserStatus;
	created_at: Date;
}







/** Row type for GetUserOrders queries. */
export interface GetUserOrdersRow {
	id: number;
	name: string;
	total: string | null;
	notes: string | null;
}



/** Row type for CountUsersByStatus queries. */
export interface CountUsersByStatusRow {
	status: UserStatus;
	user_count: number;
}



/** Row type for GetUserWithTags queries. */
export interface GetUserWithTagsRow {
	id: number;
	name: string;
	tag_name: string;
}



/** Row type for SearchUsers queries. */
export interface SearchUsersRow {
	id: number;
	name: string;
	email: string | null;
}


