# Serialization

The library uses `serde` for all serialisation and deserialisation.
JSON is the default wire format; YAML, TOML, and MessagePack are optional.

## Deriving Serialize and Deserialize

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub id:    i64,
    pub name:  String,
    pub email: Option<String>,
}
```

## JSON

Encode to a JSON string:

```rust
let json: String = serde_json::to_string(&user)?;
let pretty: String = serde_json::to_string_pretty(&user)?;
```

Decode from a JSON string:

```rust
let user: User = serde_json::from_str(&json)?;
```

Decode from bytes:

```rust
let user: User = serde_json::from_slice(&bytes)?;
```

## YAML

```rust
let yaml: String = serde_yaml::to_string(&config)?;
let config: Config = serde_yaml::from_str(&yaml)?;
```

## Serde Attributes

Control field names, skip fields, and provide defaults:

```rust
#[derive(Deserialize)]
pub struct Config {
    #[serde(rename = "base_url")]
    pub url: String,

    #[serde(default)]
    pub debug: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}
```

## Custom Serializers

Implement `Serialize` manually for types with special requirements:

```rust
impl Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_rfc3339())
    }
}
```

## Flattening

Merge nested struct fields into the parent JSON object:

```rust
#[derive(Serialize)]
pub struct Event {
    pub kind: String,
    #[serde(flatten)]
    pub meta: Metadata,
}
```
