# Installation

This guide covers every supported installation method for the `lore` CLI.

---

## macOS and Linux (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/lore-dev/lore/main/install/install.sh | sh
```

The script:

1. Detects your OS and CPU architecture.
2. Downloads the correct pre-built binary from the latest GitHub release.
3. Verifies the SHA-256 checksum against the published `SHA256SUMS` file.
4. Installs the binary to `~/.local/bin/lore` (configurable via `LORE_BIN_DIR`).

**Options (environment variables):**

| Variable | Default | Description |
|---|---|---|
| `LORE_VERSION` | latest | Pin to a specific release tag, e.g. `v0.2.0` |
| `LORE_BIN_DIR` | `~/.local/bin` | Directory where `lore` is placed |

**Example: pin a version and install to `/usr/local/bin`:**

```sh
LORE_VERSION=v0.2.0 LORE_BIN_DIR=/usr/local/bin \
  curl -fsSL https://raw.githubusercontent.com/lore-dev/lore/main/install/install.sh | sh
```

---

## Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/lore-dev/lore/main/install/install.ps1 | iex
```

The script downloads the `x86_64-pc-windows-msvc` binary, verifies its checksum,
and places `lore.exe` in `$HOME\.local\bin`.  Add that directory to your `PATH`
if it is not there already.

---

## Homebrew (macOS and Linux)

```sh
brew tap lore-dev/lore
brew install lore
```

To upgrade:

```sh
brew upgrade lore
```

---

## Cargo (from crates.io)

Requires a Rust toolchain.  See <https://rustup.rs> for setup.

```sh
cargo install lore-cli
```

---

## From source

```sh
git clone https://github.com/lore-dev/lore
cd lore
cargo install --path crates/lore-cli
```

---

## Verifying the installation

```sh
lore --version
# lore 0.1.0
```

---

## First-time model download

When you first run a command that requires semantic search (`lore add`, `lore search`,
`lore build`), lore automatically downloads the **bge-small-en-v1.5** embedding model
(~130 MB) and caches it under:

| Platform | Cache directory |
|---|---|
| macOS | `~/Library/Caches/lore/models/` |
| Linux | `~/.cache/lore/models/` |
| Windows | `%LOCALAPPDATA%\lore\models\` |

This is a one-time download.  Subsequent runs use the local cache with no network
access required.

---

## Uninstalling

**curl/PowerShell install:**

```sh
rm "$(which lore)"
```

**Homebrew:**

```sh
brew uninstall lore
```

**Cargo:**

```sh
cargo uninstall lore-cli
```

Remove the model cache and package databases:

```sh
# macOS
rm -rf ~/Library/Caches/lore ~/.local/share/lore

# Linux
rm -rf ~/.cache/lore ~/.local/share/lore
```
