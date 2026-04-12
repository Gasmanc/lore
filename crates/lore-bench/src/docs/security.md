# Security

## TLS

All outgoing connections use TLS 1.2 or higher by default.  The system
certificate store is used to validate server certificates.

### Custom CA Bundle

Point to a custom certificate bundle (useful behind a corporate proxy):

```rust
let client = Client::builder()
    .tls_ca_bundle("/etc/ssl/corporate-ca.pem")
    .build();
```

### Client Certificates (mTLS)

For mutual TLS, supply a PEM-encoded certificate and private key:

```rust
let client = Client::builder()
    .tls_identity("client.pem", "client.key")
    .build();
```

### Disabling Certificate Verification

Never disable certificate verification in production.  For testing only:

```rust
// DANGER: only for local development
let client = Client::builder()
    .danger_accept_invalid_certs(true)
    .build();
```

## Encryption at Rest

Encrypt the local SQLite database:

```rust
Db::builder("myapp.db")
    .encryption_key(key_bytes)
    .open()
    .await?;
```

## Secrets Management

Retrieve secrets from a vault rather than environment variables:

```rust
let key = mylib::secrets::vault_read("myapp/api-key").await?;
```

## Permissions

Run the application with the minimum permissions required.  The library
never requests elevated OS privileges.

## Security Headers

The built-in HTTP server sets the following headers automatically:

- `Strict-Transport-Security: max-age=63072000; includeSubDomains`
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`

## Audit Logging

Enable the `audit` feature to log all authentication events:

```toml
mylib = { version = "2", features = ["audit"] }
```
