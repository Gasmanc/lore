# Authentication

Every request to a protected endpoint must carry a valid credential.
The library supports API keys, Bearer tokens, and OAuth 2.0.

## API Key

Set the `MYLIB_API_KEY` environment variable, or pass it at construction time:

```rust
let client = Client::builder()
    .api_key("sk-live-xxxxxxxxxxxxxxxx")
    .build();
```

The key is sent as the `X-API-Key` request header.

## Bearer Token

Wrap a pre-obtained token with `BearerAuth`:

```rust
use mylib::auth::BearerAuth;

let auth = BearerAuth::new("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...");
let client = Client::builder().auth(auth).build();
```

## OAuth 2.0 Client Credentials

Use the built-in `OAuthFlow` to automatically obtain and refresh access tokens:

```rust
use mylib::auth::OAuthFlow;

let flow = OAuthFlow::client_credentials(
    "https://auth.example.com/token",
    "my-client-id",
    "my-client-secret",
);
let client = Client::builder().oauth(flow).build();
```

The library caches the access token and refreshes it five minutes before
expiry so callers never need to manage token lifecycle manually.

## Rotating Credentials

Call `client.rotate_key("new-key")` to update credentials without rebuilding
the client.  In-flight requests complete with the old key; all subsequent
requests use the new one.

## Security Notes

- Never embed API keys or OAuth secrets in source code.
- Store secrets in environment variables or a secrets manager.
- Rotate keys every 90 days as a security best practice.
