# Performance

This guide covers profiling the application, identifying bottlenecks,
and tuning the library for high-throughput workloads.

## Profiling

Enable the `profile` feature to collect per-operation timing data:

```toml
[features]
profile = ["mylib/profile"]
```

Access the report at runtime:

```rust
let report = client.profile_report();
println!("{report}");
```

## Flamegraph

Use `cargo-flamegraph` to visualise CPU hot paths:

```bash
cargo flamegraph --bin myapp
```

The output SVG highlights which functions consume the most CPU time.

## Benchmarks

Run the built-in microbenchmarks with Criterion:

```bash
cargo bench
```

Benchmark results are written to `target/criterion/`.

## Connection Pool Sizing

The optimal pool size depends on your workload.  A good starting point is
`2 * num_cpus`.  Monitor the `pool.wait_time` metric and increase the pool
if threads frequently wait for a free connection.

## Reducing Allocations

Enable the `jemalloc` allocator feature for ~15% lower allocation overhead
in allocation-heavy workloads:

```toml
mylib = { version = "2", features = ["jemalloc"] }
```

## Async Task Overhead

Each request spawns a lightweight async task.  For extremely high request
rates (>100 k/s), use the `batch_send` API to amortise task spawn cost:

```rust
let responses = client.batch_send(requests).await?;
```

## Metrics

Prometheus-compatible metrics are available at `/metrics` when the
`metrics` feature is enabled.  Key metrics:

- `mylib_requests_total` — counter by status code
- `mylib_request_duration_seconds` — histogram of latencies
- `mylib_pool_active_connections` — gauge
