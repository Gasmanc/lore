# Testing

The library ships test helpers that make it easy to write unit tests and
integration tests without hitting live services.

## Unit Tests

Use the `mock_client` helper to create a preconfigured client that returns
canned responses:

```rust
#[cfg(test)]
mod tests {
    use mylib::testing::mock_client;

    #[tokio::test]
    async fn test_fetch_user() {
        let client = mock_client()
            .response(200, r#"{"id":1,"name":"Alice"}"#)
            .build();

        let user: User = client.get("/users/1").await.unwrap().json().await.unwrap();
        assert_eq!(user.name, "Alice");
    }
}
```

## Asserting on Requests

Verify that the client sent the expected request:

```rust
let mock = mock_client()
    .expect_post("/items")
    .with_body(r#"{"name":"Widget"}"#)
    .response(201, r#"{"id":42}"#)
    .build();

client.post("/items").json(&item).send().await.unwrap();
mock.assert_all_called();
```

## Integration Tests

Place integration tests in `tests/` and enable them with a feature flag:

```toml
[features]
integration-tests = []
```

```bash
cargo test --features integration-tests
```

## Test Database

Use an in-memory SQLite database so tests run fast and in isolation:

```rust
let db = mylib::db::Db::open(":memory:").await.unwrap();
db.migrate(SCHEMA).await.unwrap();
```

## Snapshot Testing

`assert_snapshot!` captures and diffs structured output across test runs:

```rust
assert_snapshot!(response.body, @r#"{"status":"ok"}"#);
```
