# Logging

The library integrates with the `tracing` ecosystem for structured,
levelled logging.  Logs are emitted as `tracing` events and can be routed
to any subscriber.

## Log Levels

Five severity levels are available, from noisiest to quietest:

| Level   | Use case                                           |
|---------|----------------------------------------------------|
| `TRACE` | Very fine-grained diagnostic information           |
| `DEBUG` | Developer-oriented diagnostics                     |
| `INFO`  | Normal operational events (startup, shutdown)      |
| `WARN`  | Recoverable issues that need attention             |
| `ERROR` | Non-recoverable errors that affect functionality   |

## Configuring the Log Level

Set the level with the `MYLIB_LOG_LEVEL` environment variable:

```bash
MYLIB_LOG_LEVEL=debug cargo run
```

Or in code:

```rust
mylib::logging::init("debug");
```

## Structured Fields

Every log event carries structured key-value fields.  In code:

```rust
tracing::info!(user_id = %user.id, action = "login", "User logged in");
```

## Log Sinks

By default, logs are written to `stderr` in a human-readable format.
For production, configure a JSON sink:

```rust
use tracing_subscriber::fmt;

fmt()
    .json()
    .with_env_filter("mylib=info,warn")
    .init();
```

## Request Tracing

Each outgoing HTTP request is automatically instrumented with a span that
records the URL, method, status code, and latency.  Connect a tracing
collector (Jaeger, Zipkin) to visualise distributed traces.

## Disabling Logs

Set `MYLIB_LOG_LEVEL=off` to suppress all library output.
