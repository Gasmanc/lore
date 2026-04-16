# Getting Started

Welcome to mylib!  This guide walks you through installation, basic setup, and
your first query.

## Installation

Add mylib to your project with Cargo:

```toml
[dependencies]
mylib = "1.0"
```

Or install the CLI globally:

```sh
cargo install mylib-cli
```

## Quick Start

Import the client and run your first query:

```rust
use mylib::Client;

#[tokio::main]
async fn main() {
    let client = Client::new("https://api.example.com");
    let result = client.query("hello world").await.unwrap();
    println!("{result}");
}
```

## Configuration

The client reads configuration from environment variables:

- `MYLIB_URL` — API endpoint (required)
- `MYLIB_TOKEN` — authentication token (optional)
- `MYLIB_TIMEOUT` — request timeout in seconds (default: 30)

You can also pass a `Config` struct directly:

```rust
use mylib::{Client, Config};

let config = Config {
    url: "https://api.example.com".into(),
    token: Some("secret".into()),
    timeout_secs: 60,
};
let client = Client::from_config(config);
```

## Next Steps

- See the [API Reference](api-reference.md) for the full method list.
- Check [Advanced Topics](advanced.md) for rate limiting and retries.
