# File I/O

The library provides async helpers for reading and writing files, walking
directories, and atomically replacing existing files.

## Reading Files

Read the entire contents of a file into a `String`:

```rust
use mylib::fs;

let contents: String = fs::read_to_string("config.toml").await?;
```

Read raw bytes:

```rust
let bytes: Vec<u8> = fs::read("image.png").await?;
```

## Writing Files

Write a string or bytes to a file, creating it if it does not exist:

```rust
fs::write("output.txt", "Hello, world!\n").await?;
fs::write_bytes("data.bin", &bytes).await?;
```

## Atomic Writes

To avoid partial writes, write to a temporary file then rename:

```rust
fs::write_atomic("important.json", &serialised).await?;
```

This writes to `important.json.tmp` first, then renames on success.

## Appending

```rust
fs::append("log.txt", "new line\n").await?;
```

## Walking Directories

Iterate over all files under a directory tree:

```rust
let files = fs::walk_dir("./docs").await?;
for path in files {
    println!("{}", path.display());
}
```

Filter by extension:

```rust
let md_files = fs::walk_dir_ext("./docs", "md").await?;
```

## Creating and Removing Directories

```rust
fs::create_dir_all("output/reports/2024").await?;
fs::remove_dir_all("tmp/cache").await?;
```

## File Metadata

```rust
let meta = fs::metadata("myfile.txt").await?;
println!("size: {} bytes, modified: {:?}", meta.size, meta.modified);
```
