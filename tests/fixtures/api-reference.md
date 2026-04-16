# API Reference

Complete reference for the mylib public API.

## Client

The top-level entry point for all mylib operations.

### `Client::new`

```rust
pub fn new(url: impl Into<String>) -> Client
```

Creates a client connected to `url`.  Uses the default [`Config`] values for
timeout and authentication.

### `Client::query`

```rust
pub async fn query(&self, q: &str) -> Result<QueryResult, Error>
```

Executes a search query and returns up to 10 matching results sorted by
relevance score.

#### Parameters

- `q` — natural-language or keyword query string (max 512 chars)

#### Errors

Returns [`Error::Http`] on network failure or [`Error::Auth`] if the token is
invalid.

### `Client::from_config`

```rust
pub fn from_config(config: Config) -> Client
```

Creates a client with the given [`Config`].

## Config

```rust
pub struct Config {
    pub url: String,
    pub token: Option<String>,
    pub timeout_secs: u64,
}
```

### Fields

- `url` — base URL of the API server
- `token` — bearer token for authentication; if `None` anonymous access is used
- `timeout_secs` — per-request TCP timeout; defaults to `30`

## QueryResult

```rust
pub struct QueryResult {
    pub items: Vec<Item>,
    pub total: u64,
}
```

### Fields

- `items` — ordered list of matching documents
- `total` — total count of matching documents (may exceed `items.len()`)

## Error

```rust
pub enum Error {
    Http(reqwest::Error),
    Auth(String),
    Deserialise(serde_json::Error),
}
```
