# Error Handling

The library uses a structured `Error` type so callers can match on the cause
and decide whether to retry, surface the failure, or recover silently.

## The `Error` Type

```rust
use mylib::Error;

match client.get(url).await {
    Ok(resp)                  => process(resp),
    Err(Error::Timeout)       => println!("request timed out"),
    Err(Error::NotFound(url)) => println!("resource not found: {url}"),
    Err(Error::Auth(msg))     => println!("authentication failed: {msg}"),
    Err(e)                    => return Err(e.into()),
}
```

## Error Variants

| Variant          | When it occurs                                       |
|------------------|------------------------------------------------------|
| `Timeout`        | Server did not respond within the configured timeout |
| `NotFound`       | HTTP 404 or resource deleted                         |
| `Auth`           | Invalid credentials or expired token                 |
| `RateLimit`      | HTTP 429; contains `retry_after` seconds             |
| `ServerError`    | HTTP 5xx from the remote server                      |
| `Network`        | TCP-level failure, DNS error, or TLS handshake fail  |
| `Parse`          | Response body could not be deserialised              |

## Recoverable vs Fatal Errors

`Error::is_retryable()` returns `true` for transient failures (`Timeout`,
`ServerError`, `Network`).  The built-in retry policy calls this method to
decide whether to retry automatically.

## Using `?` with `anyhow`

The library's `Error` implements `std::error::Error`, so it composes cleanly
with `anyhow`:

```rust
use anyhow::Result;

async fn fetch(url: &str) -> Result<String> {
    let resp = client.get(url).await?;
    Ok(resp.text().await?)
}
```

## Tracing Integration

Every error is emitted as a `tracing::error!` event so structured log
aggregation tools capture the full context automatically.
