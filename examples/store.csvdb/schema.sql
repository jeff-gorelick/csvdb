CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT
);
CREATE INDEX idx_customers_email ON customers (email);
CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL,
    product TEXT NOT NULL,
    amount REAL,
    notes TEXT
);

CREATE VIEW order_totals AS SELECT customer_id, COUNT(*) as order_count, SUM(amount) as total_spent FROM orders GROUP BY customer_id;
