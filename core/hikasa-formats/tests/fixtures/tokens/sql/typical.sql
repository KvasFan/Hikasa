-- Monthly revenue per customer
SELECT c.name,
       SUM(o.amount) AS total
FROM customers c
JOIN orders o ON o.customer_id = c.id
WHERE o.created_at >= '2024-01-01'
GROUP BY c.name
HAVING SUM(o.amount) > 1000.50
ORDER BY total DESC;
