# Concurrency

The library is built on Tokio and supports high-concurrency workloads via
async tasks, channels, and synchronisation primitives.

## Spawning Tasks

Run independent work concurrently with `tokio::spawn`:

```rust
let handle = tokio::spawn(async {
    expensive_computation().await
});
let result = handle.await?;
```

## JoinSet

Wait for a dynamic set of tasks:

```rust
use tokio::task::JoinSet;

let mut set = JoinSet::new();
for url in urls {
    set.spawn(client.get(url));
}

while let Some(result) = set.join_next().await {
    process(result??);
}
```

## Channels

Use `tokio::sync::mpsc` for producer-consumer patterns:

```rust
let (tx, mut rx) = tokio::sync::mpsc::channel::<Work>(32);

tokio::spawn(async move {
    while let Some(work) = rx.recv().await {
        process(work).await;
    }
});

tx.send(Work::new()).await?;
```

## Semaphore

Limit concurrency to avoid overwhelming a downstream service:

```rust
use tokio::sync::Semaphore;
use std::sync::Arc;

let sem = Arc::new(Semaphore::new(10));  // at most 10 concurrent requests

let permit = sem.acquire().await?;
client.get(url).await?;
drop(permit);
```

## Shared State

Use `Arc<Mutex<T>>` for shared mutable state across tasks:

```rust
let counter = Arc::new(tokio::sync::Mutex::new(0u64));

let c = counter.clone();
tokio::spawn(async move {
    *c.lock().await += 1;
});
```

## Cancellation

Use `tokio::select!` to race a future against a cancellation signal:

```rust
tokio::select! {
    result = client.get(url) => handle(result),
    _ = shutdown_signal()   => break,
}
```
