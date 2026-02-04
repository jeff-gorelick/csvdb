CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, product TEXT, amount REAL);

CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT);
CREATE INDEX idx_users_email ON users(email);

CREATE VIEW active_orders AS SELECT * FROM orders WHERE amount > 20;
