# Getting Started

This guide walks you through installing the library and running your first program.

## Installation

Add the library to your project using your package manager:

```bash
cargo add mylib
```

Or add it manually to `Cargo.toml`:

```toml
[dependencies]
mylib = "2.0"
```

For npm projects:

```bash
npm install mylib
```

## Quickstart

After installation, import the library and initialise the client:

```rust
use mylib::Client;

#[tokio::main]
async fn main() {
    let client = Client::new();
    println!("Hello from mylib {}", mylib::VERSION);
}
```

## First Request

Send your first request in three lines:

```rust
let client = Client::new();
let response = client.get("https://api.example.com/hello").await?;
println!("{}", response.body);
```

## Prerequisites

- Rust 1.75 or later (stable)
- An internet connection for the initial setup
- A valid licence key if using the commercial edition

## Next Steps

Once installed, read the [Configuration](configuration.md) guide to customise timeouts, retries, and logging.
