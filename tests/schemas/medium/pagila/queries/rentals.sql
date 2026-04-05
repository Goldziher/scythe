-- @name CreateRental
-- @returns :one
INSERT INTO rental (rental_date, inventory_id, customer_id, staff_id)
VALUES ($1, $2, $3, $4)
RETURNING rental_id, rental_date, inventory_id, customer_id, staff_id;

-- @name ReturnRental
-- @returns :exec
UPDATE rental SET return_date = NOW() WHERE rental_id = $1 AND return_date IS NULL;

-- @name GetOverdueRentals
-- @returns :many
SELECT r.rental_id, c.first_name, c.last_name, c.email,
       f.title, r.rental_date,
       CURRENT_DATE - r.rental_date::date as days_overdue
FROM rental r
JOIN customer c ON r.customer_id = c.customer_id
JOIN inventory i ON r.inventory_id = i.inventory_id
JOIN film f ON i.film_id = f.film_id
WHERE r.return_date IS NULL
  AND r.rental_date < CURRENT_TIMESTAMP - INTERVAL '7 days'
ORDER BY r.rental_date;

-- @name GetDailyRevenue
-- @returns :many
SELECT payment_date::date as day, SUM(amount) as revenue, COUNT(*) as transactions
FROM payment
WHERE payment_date >= $1 AND payment_date < $2
GROUP BY payment_date::date
ORDER BY day;

-- @name GetStoreInventoryCount
-- @returns :many
SELECT s.store_id, COUNT(i.inventory_id) as total_inventory,
       COUNT(DISTINCT i.film_id) as unique_films
FROM store s
LEFT JOIN inventory i ON s.store_id = i.store_id
GROUP BY s.store_id;
