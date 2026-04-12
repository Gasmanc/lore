# CLI Reference

The `myapp` binary exposes several subcommands.  Run `myapp --help` for a
summary or `myapp <subcommand> --help` for subcommand-specific flags.

## Global Flags

| Flag              | Short | Description                          |
|-------------------|-------|--------------------------------------|
| `--verbose`       | `-v`  | Increase log verbosity               |
| `--quiet`         | `-q`  | Suppress all output except errors    |
| `--config <path>` | `-c`  | Path to the configuration file       |
| `--version`       | `-V`  | Print version and exit               |

## Subcommands

### `serve`

Start the HTTP server.

```bash
myapp serve --port 8080 --workers 4
```

Flags:
- `--port <n>` — listening port (default: 8080)
- `--workers <n>` — number of async worker threads (default: num\_cpus)
- `--host <addr>` — bind address (default: 0.0.0.0)

### `migrate`

Apply pending database migrations.

```bash
myapp migrate --db myapp.db
```

### `export`

Export data to a file.

```bash
myapp export --format json --output data.json
myapp export --format csv  --output data.csv
```

### `import`

Import data from a file.

```bash
myapp import data.json
```

### `check`

Validate configuration and exit without starting the server.

```bash
myapp check --config mylib.toml
```

### `version`

Print the build version, commit SHA, and build date.

```bash
myapp version
```

## Exit Codes

| Code | Meaning                        |
|------|--------------------------------|
| `0`  | Success                        |
| `1`  | General error                  |
| `2`  | Invalid arguments or flags     |
| `3`  | Configuration error            |
