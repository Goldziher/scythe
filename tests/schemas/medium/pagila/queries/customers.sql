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

-- @name GetCustomerRentalHistory
-- @returns :many
SELECT r.rental_id, f.title, r.rental_date, r.return_date,
       p.amount, f.rating
FROM rental r
JOIN inventory i ON r.inventory_id = i.inventory_id
JOIN film f ON i.film_id = f.film_id
LEFT JOIN payment p ON r.rental_id = p.rental_id
WHERE r.customer_id = $1
ORDER BY r.rental_date DESC
LIMIT $2 OFFSET $3;

-- @name GetTopSpendingCustomers
-- @returns :many
SELECT c.customer_id, c.first_name, c.last_name,
       SUM(p.amount) as total_spent,
       COUNT(p.payment_id) as payment_count,
       AVG(p.amount) as avg_payment
FROM customer c
JOIN payment p ON c.customer_id = p.customer_id
GROUP BY c.customer_id, c.first_name, c.last_name
HAVING SUM(p.amount) > $1
ORDER BY total_spent DESC;

-- @name GetCustomersByCountry
-- @returns :many
SELECT co.country, COUNT(c.customer_id) as customer_count
FROM customer c
JOIN address a ON c.address_id = a.address_id
JOIN city ci ON a.city_id = ci.city_id
JOIN country co ON ci.country_id = co.country_id
GROUP BY co.country
ORDER BY customer_count DESC;
