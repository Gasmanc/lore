# lore — root justfile
# `just check` is THE gate. Everything runs locally (no CI minutes required).

# Default: list available targets
default:
    @just --list

# Run all linters + tests — the gate. Mirrors what release CI would run.
check: check-rust
    @echo "✓ all checks passed"

# All tests
test: test-rust
    @echo "✓ all tests passed"

# Format all code
fmt:
    cargo fmt --all
    @echo "✓ formatted"

# Auto-fix clippy findings where possible, then format
fix:
    cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged
    cargo fmt --all
    @echo "✓ fixed"

# --- Rust workspace ---
check-rust:
    cargo fmt --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    just test-rust

test-rust:
    if command -v cargo-nextest >/dev/null 2>&1; then cargo nextest run --no-tests=pass --workspace --all-features; else cargo test --workspace --all-features; fi

# Supply-chain / licence / advisory audit (needs `cargo install cargo-deny`).
deny-rust:
    cargo deny check

# Retrieval-quality benchmark (needs the embedding model cached).
bench:
    cargo run -p lore-bench --release

# Build the release binary.
build:
    cargo build --release -p lore-cli

# --- Registry (local build; replaces the GitHub Actions build-registry job) ---

# Build one package spec (or all under packages/) into local .db files and a
# registry index.json. Usage: `just build-registry` or `just build-registry packages/npm/next.yaml`.
build-registry spec="": build
    bash scripts/build-registry.sh "{{spec}}"

# Install the freshly built binary to ~/.local/bin for daily use.
install: build
    install -Dm755 target/release/lore ~/.local/bin/lore
    @echo "✓ installed to ~/.local/bin/lore"
