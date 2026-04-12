# Plugins

The plugin system lets you extend the library with custom behaviour without
forking the codebase.  Plugins register hooks that the library calls at
well-defined lifecycle points.

## Creating a Plugin

Implement the `Plugin` trait:

```rust
use mylib::plugin::{Plugin, Context, HookResult};

pub struct LoggingPlugin;

impl Plugin for LoggingPlugin {
    fn name(&self) -> &str { "logging" }

    fn on_request(&self, ctx: &mut Context) -> HookResult {
        println!("→ {} {}", ctx.method(), ctx.url());
        HookResult::Continue
    }

    fn on_response(&self, ctx: &mut Context) -> HookResult {
        println!("← {}", ctx.status());
        HookResult::Continue
    }
}
```

## Registering a Plugin

Register the plugin at client construction time:

```rust
let client = Client::builder()
    .plugin(LoggingPlugin)
    .plugin(MetricsPlugin::new(prometheus_registry))
    .build();
```

Plugins run in registration order.  A plugin can return `HookResult::Abort`
to short-circuit the remaining plugins and the default handler.

## Hook Points

| Hook              | When it fires                                      |
|-------------------|----------------------------------------------------|
| `on_request`      | Before the request is sent                         |
| `on_response`     | After a response is received                       |
| `on_error`        | When an error occurs                               |
| `on_retry`        | Before each retry attempt                          |
| `on_startup`      | Once, when the client is first created             |
| `on_shutdown`     | Once, when the client is dropped                   |

## Accessing Plugin State

Plugins can store per-request state in the `Context` extension map:

```rust
ctx.extensions_mut().insert(RequestId::new());
let id: &RequestId = ctx.extensions().get().unwrap();
```

## Async Hooks

If the hook needs to perform I/O, implement `AsyncPlugin` instead:

```rust
#[async_trait]
impl AsyncPlugin for DatabaseAuditPlugin {
    async fn on_request(&self, ctx: &mut Context) -> HookResult {
        self.db.insert_audit_log(ctx).await;
        HookResult::Continue
    }
}
```
