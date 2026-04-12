# Caching

The library provides an in-process response cache with configurable TTL,
maximum size, and cache-invalidation helpers.

## Enabling the Cache

```rust
use mylib::{Client, CacheConfig};
use std::time::Duration;

let client = Client::builder()
    .cache(CacheConfig {
        max_entries: 1000,
        ttl:         Duration::from_secs(300),
    })
    .build();
```

## TTL (Time to Live)

Each cached entry expires automatically after `ttl`.  A background sweep
runs every 60 seconds to evict expired entries and free memory.  You can
trigger an immediate sweep:

```rust
client.cache().sweep_expired();
```

## Cache Keys

The default key is the full request URL including query string.  Override
with a custom key function:

```rust
client.cache_key(|req| req.path().to_owned());
```

## Invalidation

Remove a specific entry or clear the entire cache:

```rust
client.cache().invalidate("https://api.example.com/users/42");
client.cache().clear();
```

## Stale-While-Revalidate

Set `stale_while_revalidate` to serve a stale cached entry immediately while
a background refresh runs in parallel:

```rust
CacheConfig {
    ttl:                   Duration::from_secs(60),
    stale_while_revalidate: Duration::from_secs(30),
    ..Default::default()
}
```

## Cache Statistics

Inspect hit rate and eviction counts at runtime:

```rust
let stats = client.cache().stats();
println!("hit rate: {:.1}%", stats.hit_rate() * 100.0);
println!("evictions: {}", stats.evictions);
```
