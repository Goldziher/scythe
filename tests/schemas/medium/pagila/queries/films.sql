-- @name GetFilm
-- @returns :one
SELECT f.film_id, f.title, f.description, f.release_year, f.rating,
       f.rental_rate, f.length, f.replacement_cost, f.special_features,
       l.name as language
FROM film f
JOIN language l ON f.language_id = l.language_id
WHERE f.film_id = $1;

-- @name ListFilmsByCategory
-- @returns :many
SELECT f.film_id, f.title, f.rating, f.rental_rate, c.name as category
FROM film f
JOIN film_category fc ON f.film_id = fc.film_id
JOIN category c ON fc.category_id = c.category_id
WHERE c.name = $1
ORDER BY f.title;

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

-- @name GetFilmRevenueByCategory
-- @returns :many
WITH film_revenue AS (
    SELECT f.film_id, SUM(p.amount) as revenue
    FROM film f
    JOIN inventory i ON f.film_id = i.film_id
    JOIN rental r ON i.inventory_id = r.inventory_id
    JOIN payment p ON r.rental_id = p.rental_id
    GROUP BY f.film_id
)
SELECT c.name as category,
       COUNT(DISTINCT f.film_id) as film_count,
       COALESCE(SUM(fr.revenue), 0) as total_revenue,
       COALESCE(AVG(fr.revenue), 0) as avg_revenue
FROM category c
JOIN film_category fc ON c.category_id = fc.category_id
JOIN film f ON fc.film_id = f.film_id
LEFT JOIN film_revenue fr ON f.film_id = fr.film_id
GROUP BY c.name
ORDER BY total_revenue DESC;

-- @name SearchFilms
-- @returns :many
SELECT film_id, title, description, rating, rental_rate
FROM film
WHERE title LIKE $1 OR description LIKE $2
ORDER BY title
LIMIT $3 OFFSET $4;
