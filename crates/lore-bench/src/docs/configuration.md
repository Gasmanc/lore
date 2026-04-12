# Configuration

The library reads configuration from a file, environment variables, or code.
Environment variables always take precedence over the config file.

## Configuration File

Place a `mylib.toml` in the project root:

```toml
[client]
timeout_secs = 30
retry_max    = 3
retry_delay_ms = 500

[logging]
level = "info"
```

## Environment Variables

| Variable                 | Default | Description                              |
|--------------------------|---------|------------------------------------------|
| `MYLIB_TIMEOUT`          | `30`    | Request timeout in seconds               |
| `MYLIB_RETRY_MAX`        | `3`     | Maximum number of retry attempts         |
| `MYLIB_RETRY_DELAY_MS`   | `500`   | Delay between retries in milliseconds    |
| `MYLIB_LOG_LEVEL`        | `info`  | Log verbosity (trace/debug/info/warn)    |
| `MYLIB_BASE_URL`         | —       | Override the default base URL            |

## Code-Level Configuration

Pass a `Config` struct to the client builder:

```rust
use mylib::{Client, Config};
use std::time::Duration;

let config = Config::builder()
    .timeout(Duration::from_secs(60))
    .retry_max(5)
    .retry_delay(Duration::from_millis(1000))
    .build();

let client = Client::with_config(config);
```

## Timeout Settings

Timeout controls how long the client waits for a server response before
cancelling the request and optionally retrying.  Setting `timeout_secs = 0`
disables the timeout entirely (not recommended in production).

## Retry Policy

The default exponential backoff doubles the delay between each retry attempt.
You can supply a custom `RetryPolicy` to implement jitter or fixed delays.
