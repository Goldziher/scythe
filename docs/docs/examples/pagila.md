# Pagila

Real-world example based on the [Pagila sample database](https://github.com/devrimgunduz/pagila) (PostgreSQL License).

## Schema overview

15+ tables modeling a DVD rental store. Includes enums, domains, views, and complex relationships.

```sql
CREATE TYPE mpaa_rating AS ENUM ('G', 'PG', 'PG-13', 'R', 'NC-17');

CREATE DOMAIN year AS INTEGER CHECK (VALUE >= 1901 AND VALUE <= 2155);

CREATE TABLE film (
    film_id SERIAL PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT,
    release_year year,
    language_id INTEGER NOT NULL REFERENCES language(language_id),
    rental_duration SMALLINT NOT NULL DEFAULT 3,
    rental_rate NUMERIC(4,2) NOT NULL DEFAULT 4.99,
    length SMALLINT,
    replacement_cost NUMERIC(5,2) NOT NULL DEFAULT 19.99,
    rating mpaa_rating DEFAULT 'G',
    special_features TEXT[],
    last_update TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE actor (
    actor_id SERIAL PRIMARY KEY,
    first_name TEXT NOT NULL,
    last_name TEXT NOT NULL,
    last_update TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE film_actor (
    actor_id INTEGER NOT NULL REFERENCES actor(actor_id),
    film_id INTEGER NOT NULL REFERENCES film(film_id),
    last_update TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (actor_id, film_id)
);

CREATE TABLE customer (
    customer_id SERIAL PRIMARY KEY,
    store_id INTEGER NOT NULL,
    first_name TEXT NOT NULL,
    last_name TEXT NOT NULL,
    email TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    create_date DATE NOT NULL DEFAULT CURRENT_DATE,
    last_update TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE rental (
    rental_id SERIAL PRIMARY KEY,
    rental_date TIMESTAMPTZ NOT NULL,
    inventory_id INTEGER NOT NULL,
    customer_id INTEGER NOT NULL REFERENCES customer(customer_id),
    return_date TIMESTAMPTZ,
    staff_id INTEGER NOT NULL,
    last_update TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE payment (
    payment_id SERIAL PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customer(customer_id),
    staff_id INTEGER NOT NULL,
    rental_id INTEGER REFERENCES rental(rental_id),
    amount NUMERIC(5,2) NOT NULL,
    payment_date TIMESTAMPTZ NOT NULL
);
```

## Representative queries

### 1. Window function -- top renters

```sql
-- @name ListTopRenters
-- @returns :many
SELECT
    c.customer_id,
    c.first_name,
    c.last_name,
    COUNT(*) AS rental_count,
    RANK() OVER (ORDER BY COUNT(*) DESC) AS rank
FROM customer c
JOIN rental r ON r.customer_id = c.customer_id
GROUP BY c.customer_id, c.first_name, c.last_name
ORDER BY rental_count DESC
LIMIT $1;
```

### 2. CTE -- monthly revenue

```sql
-- @name GetMonthlyRevenue
-- @returns :many
WITH monthly AS (
    SELECT
        DATE_TRUNC('month', payment_date) AS month,
        SUM(amount) AS revenue
    FROM payment
    GROUP BY DATE_TRUNC('month', payment_date)
)
SELECT month, revenue
FROM monthly
ORDER BY month DESC
LIMIT $1;
```

### 3. Complex JOIN -- film details with cast

```sql
-- @name GetFilmWithCast
-- @returns :many
SELECT
    f.film_id,
    f.title,
    f.rating,
    f.release_year,
    f.special_features,
    a.first_name || ' ' || a.last_name AS actor_name
FROM film f
JOIN film_actor fa ON fa.film_id = f.film_id
JOIN actor a ON a.actor_id = fa.actor_id
WHERE f.film_id = $1
ORDER BY a.last_name, a.first_name;
```

### 4. Subquery -- films never rented

```sql
-- @name ListUnrentedFilms
-- @returns :many
SELECT f.film_id, f.title, f.rating
FROM film f
WHERE f.film_id NOT IN (
    SELECT DISTINCT i.film_id
    FROM inventory i
    JOIN rental r ON r.inventory_id = i.inventory_id
)
ORDER BY f.title
LIMIT $1;
```

### 5. Aggregation with enum filter

```sql
-- @name CountFilmsByRating
-- @returns :many
SELECT rating, COUNT(*) AS total, AVG(rental_rate) AS avg_rate
FROM film
GROUP BY rating
ORDER BY total DESC;
```

## Highlights

- **Window functions**: `RANK() OVER (ORDER BY ...)` for ranking
- **CTEs**: `WITH ... AS` for readable multi-step queries
- **Complex JOINs**: 3-table joins through junction tables
- **Domain types**: `year` domain resolves to `int32` via base type
- **Array columns**: `special_features TEXT[]` maps to `array<string>`
- **Enum columns**: `mpaa_rating` maps to `enum::mpaa_rating`
- **Nullable TIMESTAMPTZ**: `return_date` is `Option<DateTime>` / `datetime.datetime | None`

---

*Based on the Pagila sample database, PostgreSQL License.*
