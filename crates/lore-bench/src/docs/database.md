# Database

The library ships a lightweight async database layer built on top of SQLite.
It manages connection pools, schema migrations, and typed query helpers.

## Opening a Connection

```rust
use mylib::db::Db;

let db = Db::open("myapp.db").await?;
```

For in-memory databases (useful in tests):

```rust
let db = Db::open(":memory:").await?;
```

## Running Migrations

Migrations are plain SQL strings applied in order.  The current schema version
is tracked in the `_meta` table so only pending migrations run on startup.

```rust
db.migrate(&[
    "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    "ALTER TABLE users ADD COLUMN email TEXT",
]).await?;
```

## Executing Queries

Use `query_map` to deserialise rows into a typed struct:

```rust
#[derive(Debug)]
struct User { id: i64, name: String }

let users: Vec<User> = db.query_map(
    "SELECT id, name FROM users WHERE name LIKE ?1",
    ("Alice%",),
    |row| Ok(User { id: row.get(0)?, name: row.get(1)? }),
).await?;
```

## Transactions

Wrap multiple writes in a transaction to guarantee atomicity:

```rust
db.transaction(|tx| async move {
    tx.execute("INSERT INTO users (name) VALUES (?1)", ("Bob",)).await?;
    tx.execute("INSERT INTO roles (user_id, role) VALUES (?1, ?2)", (1, "admin")).await?;
    Ok(())
}).await?;
```

## Connection Pool

The default pool size is the number of CPU cores.  Adjust via:

```rust
Db::builder("myapp.db").pool_size(8).open().await?;
```
