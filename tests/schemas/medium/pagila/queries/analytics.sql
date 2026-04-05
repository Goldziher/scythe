-- @name GetRentalsByDayOfWeek
-- @returns :many
SELECT EXTRACT(DOW FROM rental_date) as day_of_week,
       COUNT(*) as rental_count
FROM rental
GROUP BY EXTRACT(DOW FROM rental_date)
ORDER BY day_of_week;

-- @name GetFilmAvailability
-- @returns :many
SELECT f.film_id, f.title,
       COUNT(i.inventory_id) as total_copies,
       COUNT(i.inventory_id) - COUNT(r.rental_id) as available_copies
FROM film f
LEFT JOIN inventory i ON f.film_id = i.film_id
LEFT JOIN rental r ON i.inventory_id = r.inventory_id AND r.return_date IS NULL
WHERE f.film_id = ANY($1)
GROUP BY f.film_id, f.title;

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
