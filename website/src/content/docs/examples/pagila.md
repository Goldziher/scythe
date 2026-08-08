---
title: Pagila
description: Real-world example based on the Pagila sample DVD rental database.
---

Real-world example based on the [Pagila sample database](https://github.com/devrimgunduz/pagila) (PostgreSQL License). This mirrors the `tests/schemas/medium/pagila` fixture in the scythe repository.

## Schema overview

15 tables modeling a DVD rental store, plus 3 views. Includes an enum, a domain, and multi-table joins.

```sql
CREATE TYPE mpaa_rating AS ENUM ('G', 'PG', 'PG-13', 'R', 'NC-17');

CREATE DOMAIN year AS integer CHECK (VALUE >= 1901 AND VALUE <= 2155);

CREATE TABLE language (
    language_id SERIAL PRIMARY KEY,
    name VARCHAR(20) NOT NULL,
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE actor (
    actor_id SERIAL PRIMARY KEY,
    first_name VARCHAR(45) NOT NULL,
    last_name VARCHAR(45) NOT NULL,
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE category (
    category_id SERIAL PRIMARY KEY,
    name VARCHAR(25) NOT NULL,
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE film (
    film_id SERIAL PRIMARY KEY,
    title VARCHAR(255) NOT NULL,
    description TEXT,
    release_year year,
    language_id INTEGER NOT NULL REFERENCES language(language_id),
    original_language_id INTEGER REFERENCES language(language_id),
    rental_duration SMALLINT NOT NULL DEFAULT 3,
    rental_rate NUMERIC(4,2) NOT NULL DEFAULT 4.99,
    length SMALLINT,
    replacement_cost NUMERIC(5,2) NOT NULL DEFAULT 19.99,
    rating mpaa_rating DEFAULT 'G',
    special_features TEXT[],
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE film_actor (
    actor_id INTEGER NOT NULL REFERENCES actor(actor_id),
    film_id INTEGER NOT NULL REFERENCES film(film_id),
    last_update TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (actor_id, film_id)
);

CREATE TABLE film_category (
    film_id INTEGER NOT NULL REFERENCES film(film_id),
    category_id INTEGER NOT NULL REFERENCES category(category_id),
    last_update TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (film_id, category_id)
);

CREATE TABLE country (
    country_id SERIAL PRIMARY KEY,
    country VARCHAR(50) NOT NULL,
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE city (
    city_id SERIAL PRIMARY KEY,
    city VARCHAR(50) NOT NULL,
    country_id INTEGER NOT NULL REFERENCES country(country_id),
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE address (
    address_id SERIAL PRIMARY KEY,
    address VARCHAR(50) NOT NULL,
    address2 VARCHAR(50),
    district VARCHAR(20) NOT NULL,
    city_id INTEGER NOT NULL REFERENCES city(city_id),
    postal_code VARCHAR(10),
    phone VARCHAR(20) NOT NULL,
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE store (
    store_id SERIAL PRIMARY KEY,
    manager_staff_id INTEGER NOT NULL,
    address_id INTEGER NOT NULL REFERENCES address(address_id),
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE staff (
    staff_id SERIAL PRIMARY KEY,
    first_name VARCHAR(45) NOT NULL,
    last_name VARCHAR(45) NOT NULL,
    address_id INTEGER NOT NULL REFERENCES address(address_id),
    email VARCHAR(50),
    store_id INTEGER NOT NULL REFERENCES store(store_id),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    username VARCHAR(16) NOT NULL,
    password VARCHAR(40),
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE customer (
    customer_id SERIAL PRIMARY KEY,
    store_id INTEGER NOT NULL REFERENCES store(store_id),
    first_name VARCHAR(45) NOT NULL,
    last_name VARCHAR(45) NOT NULL,
    email VARCHAR(50),
    address_id INTEGER NOT NULL REFERENCES address(address_id),
    activebool BOOLEAN NOT NULL DEFAULT TRUE,
    create_date DATE NOT NULL DEFAULT CURRENT_DATE,
    last_update TIMESTAMP DEFAULT NOW(),
    active INTEGER
);

CREATE TABLE inventory (
    inventory_id SERIAL PRIMARY KEY,
    film_id INTEGER NOT NULL REFERENCES film(film_id),
    store_id INTEGER NOT NULL REFERENCES store(store_id),
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE rental (
    rental_id SERIAL PRIMARY KEY,
    rental_date TIMESTAMP NOT NULL,
    inventory_id INTEGER NOT NULL REFERENCES inventory(inventory_id),
    customer_id INTEGER NOT NULL REFERENCES customer(customer_id),
    return_date TIMESTAMP,
    staff_id INTEGER NOT NULL REFERENCES staff(staff_id),
    last_update TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE payment (
    payment_id SERIAL PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customer(customer_id),
    staff_id INTEGER NOT NULL REFERENCES staff(staff_id),
    rental_id INTEGER NOT NULL REFERENCES rental(rental_id),
    amount NUMERIC(5,2) NOT NULL,
    payment_date TIMESTAMP NOT NULL
);
```

## Representative queries

### 1. Window function -- top rented films

```sql
-- @name GetTopRentedFilms
-- @returns :many
SELECT f.film_id, f.title, COUNT(r.rental_id) as rental_count,
       RANK() OVER (ORDER BY COUNT(r.rental_id) DESC) as rank
FROM film f
JOIN inventory i ON f.film_id = i.film_id
JOIN rental r ON i.inventory_id = r.inventory_id
GROUP BY f.film_id, f.title
ORDER BY rental_count DESC
LIMIT $1;
```

### 2. CTE + window function -- actor filmography

```sql
-- @name GetActorFilmography
-- @returns :many
WITH actor_films AS (
    SELECT a.actor_id, a.first_name, a.last_name,
           f.film_id, f.title, f.rating,
           ROW_NUMBER() OVER (PARTITION BY a.actor_id ORDER BY f.title) as film_number
    FROM actor a
    JOIN film_actor fa ON a.actor_id = fa.actor_id
    JOIN film f ON fa.film_id = f.film_id
)
SELECT actor_id, first_name, last_name, film_id, title, rating, film_number
FROM actor_films
WHERE actor_id = $1;
```

### 3. Complex JOIN -- customer profile

```sql
-- @name GetCustomer
-- @returns :one
SELECT c.customer_id, c.first_name, c.last_name, c.email,
       a.address, a.postal_code, a.phone,
       ci.city, co.country, c.activebool
FROM customer c
JOIN address a ON c.address_id = a.address_id
JOIN city ci ON a.city_id = ci.city_id
JOIN country co ON ci.country_id = co.country_id
WHERE c.customer_id = $1;
```

### 4. Conditional aggregation -- category revenue comparison

```sql
-- @name GetCategoryRevenueComparison
-- @returns :many
SELECT c.name as category,
       SUM(CASE WHEN r.rental_date >= $1 THEN p.amount ELSE 0 END) as current_period,
       SUM(CASE WHEN r.rental_date < $1 THEN p.amount ELSE 0 END) as previous_period
FROM category c
JOIN film_category fc ON c.category_id = fc.category_id
JOIN film f ON fc.film_id = f.film_id
JOIN inventory i ON f.film_id = i.film_id
JOIN rental r ON i.inventory_id = r.inventory_id
JOIN payment p ON r.rental_id = p.rental_id
GROUP BY c.name
ORDER BY current_period DESC;
```

### 5. Enum column -- films by category

```sql
-- @name ListFilmsByCategory
-- @returns :many
SELECT f.film_id, f.title, f.rating, f.rental_rate, c.name as category
FROM film f
JOIN film_category fc ON f.film_id = fc.film_id
JOIN category c ON fc.category_id = c.category_id
WHERE c.name = $1
ORDER BY f.title;
```

## Highlights

- **Window functions**: `RANK() OVER (ORDER BY ...)` and `ROW_NUMBER() OVER (PARTITION BY ...)` for ranking
- **CTEs**: `WITH ... AS` for readable multi-step queries
- **Complex JOINs**: 4-table join through `customer -> address -> city -> country`
- **Conditional aggregation**: `SUM(CASE WHEN ...)` for period comparisons
- **Domain types**: `year` domain resolves to `int32` via base type
- **Array columns**: `special_features TEXT[]` maps to `array<string>`
- **Enum columns**: `f.rating` (`mpaa_rating`) maps to `enum::mpaa_rating`

---

*Based on the Pagila sample database, PostgreSQL License.*
