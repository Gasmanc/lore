# Types

The library makes extensive use of Rust's type system to encode invariants
at compile time and eliminate entire classes of runtime errors.

## Generic Types

Many APIs are generic over the response body type.  The type parameter must
implement `serde::DeserializeOwned`:

```rust
async fn fetch<T: DeserializeOwned>(url: &str) -> Result<T> {
    client.get(url).await?.json::<T>().await
}

let user: User = fetch("/users/1").await?;
let items: Vec<Item> = fetch("/items").await?;
```

## Trait Bounds

The `Processor` trait bound ensures only valid processors can be registered:

```rust
pub trait Processor: Send + Sync + 'static {
    type Input;
    type Output;
    fn process(&self, input: Self::Input) -> Self::Output;
}
```

## Newtype Wrappers

Use newtypes to distinguish IDs of different entity types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderId(i64);
```

This prevents accidentally passing a `UserId` where an `OrderId` is required.

## Builder Pattern

Complex types use the builder pattern for optional fields:

```rust
let config = Config::builder()
    .timeout(Duration::from_secs(30))
    .retry_max(3)
    .build();   // returns Result<Config, ConfigError>
```

## Phantom Types

`TypedUrl<T>` uses a phantom type parameter to encode the expected response
type in the URL itself, giving the compiler enough information to infer the
return type automatically:

```rust
let url: TypedUrl<User> = TypedUrl::new("/users/1");
let user = client.fetch(url).await?;  // inferred as User
```

## Type Aliases

Common composite types are aliased for readability:

```rust
pub type Result<T, E = Error> = std::result::Result<T, E>;
pub type Headers = HashMap<String, String>;
```
