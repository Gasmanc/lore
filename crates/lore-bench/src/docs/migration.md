# Migration Guide

This guide covers breaking changes introduced in each major version and
explains how to update your code.

## Migrating from v1 to v2

Version 2 is a major release with several breaking API changes.

### Client construction

**v1:**
```rust
let client = Client::new("https://api.example.com", "my-key");
```

**v2:**
```rust
let client = Client::builder()
    .base_url("https://api.example.com")
    .api_key("my-key")
    .build();
```

### Error type

The `Error` enum was renamed and several variants were merged.

**v1:**
```rust
match err {
    Error::HttpError(status) => { ... }
    Error::ParseError(msg)   => { ... }
}
```

**v2:**
```rust
match err {
    Error::ServerError { status, .. } => { ... }
    Error::Parse(msg)                 => { ... }
}
```

### Async runtime

v1 used `async-std`; v2 requires `tokio`.  Update your `Cargo.toml`:

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
```

And replace `#[async_std::main]` with `#[tokio::main]`.

### Config file format

The `mylib.yaml` config file has been replaced by `mylib.toml`.
Run the migration helper to convert automatically:

```bash
myapp migrate-config --input mylib.yaml --output mylib.toml
```

## Migrating from v2 to v3

v3 adds no breaking changes but deprecates several methods that will be
removed in v4.

### Deprecated methods

| v2 method              | v3 replacement               |
|------------------------|------------------------------|
| `client.send(req)`     | `client.execute(req)`        |
| `resp.body_string()`   | `resp.text().await`          |
| `Config::from_env()`   | `Config::from_environment()` |

Enable `#[deny(deprecated)]` to catch all usages at compile time.
