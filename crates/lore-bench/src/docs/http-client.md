# HTTP Client

The built-in HTTP client supports GET, POST, PUT, PATCH, DELETE, and custom
methods.  All requests are async and support streaming response bodies.

## Basic GET

```rust
let response = client.get("https://api.example.com/items").await?;
let body: Vec<Item> = response.json().await?;
```

## POST with JSON Body

Serialise any `serde::Serialize` value as the request body:

```rust
#[derive(serde::Serialize)]
struct NewItem { name: String, price: f64 }

let item = NewItem { name: "Widget".into(), price: 9.99 };
let response = client
    .post("https://api.example.com/items")
    .json(&item)
    .send()
    .await?;
```

## Custom Headers

```rust
let response = client
    .get("https://api.example.com/protected")
    .header("X-Request-ID", "abc-123")
    .header("Accept", "application/json")
    .send()
    .await?;
```

## Query Parameters

```rust
let response = client
    .get("https://api.example.com/search")
    .query("q", "rust async")
    .query("limit", "10")
    .send()
    .await?;
```

## Streaming Responses

For large responses, stream the body in chunks to avoid buffering everything
in memory:

```rust
let mut stream = client
    .get("https://files.example.com/large.csv")
    .stream()
    .await?;

while let Some(chunk) = stream.next().await {
    process_chunk(chunk?);
}
```

## Status Codes

`response.error_for_status()` returns `Err(Error::ServerError(...))` for any
4xx or 5xx response code, making it easy to treat non-success as a failure.
