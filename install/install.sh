#!/usr/bin/env sh
# Install the lore CLI.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/lore-dev/lore/main/install/install.sh | sh
#
# Options (environment variables):
#   LORE_VERSION  — release tag to install, e.g. "v0.1.0" (default: latest)
#   LORE_BIN_DIR  — directory to place the binary (default: ~/.local/bin)

set -eu

REPO="lore-dev/lore"
BIN_DIR="${LORE_BIN_DIR:-$HOME/.local/bin}"

# ── Detect platform ────────────────────────────────────────────────────────────

uname_os() {
    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    case "$os" in
        linux*)  echo "unknown-linux-gnu" ;;
        darwin*) echo "apple-darwin" ;;
        *)       echo "Unsupported OS: $os" >&2; exit 1 ;;
    esac
}

uname_arch() {
    arch=$(uname -m)
    case "$arch" in
        x86_64 | amd64) echo "x86_64" ;;
        arm64 | aarch64) echo "aarch64" ;;
        *)               echo "Unsupported arch: $arch" >&2; exit 1 ;;
    esac
}

OS=$(uname_os)
ARCH=$(uname_arch)
TARGET="${ARCH}-${OS}"

# ── Resolve version ────────────────────────────────────────────────────────────

if [ -z "${LORE_VERSION:-}" ]; then
    LORE_VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
fi

if [ -z "$LORE_VERSION" ]; then
    echo "Error: could not determine latest release. Set LORE_VERSION explicitly." >&2
    exit 1
fi

echo "Installing lore ${LORE_VERSION} for ${TARGET}…"

# ── Download and verify ────────────────────────────────────────────────────────

ARCHIVE="lore-${LORE_VERSION}-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${LORE_VERSION}"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "${BASE_URL}/${ARCHIVE}" -o "${TMP}/${ARCHIVE}"
curl -fsSL "${BASE_URL}/SHA256SUMS" -o "${TMP}/SHA256SUMS"

# Verify checksum (sha256sum on Linux, shasum on macOS).
cd "$TMP"
if command -v sha256sum >/dev/null 2>&1; then
    grep "${ARCHIVE}" SHA256SUMS | sha256sum -c -
elif command -v shasum >/dev/null 2>&1; then
    grep "${ARCHIVE}" SHA256SUMS | shasum -a 256 -c -
else
    echo "Warning: no sha256 tool found; skipping checksum verification." >&2
fi

# ── Install ────────────────────────────────────────────────────────────────────

tar -xzf "${ARCHIVE}"
mkdir -p "${BIN_DIR}"
install -m 755 "lore-${LORE_VERSION}-${TARGET}/lore" "${BIN_DIR}/lore"

echo "lore installed to ${BIN_DIR}/lore"

# Remind user to add BIN_DIR to PATH if necessary.
case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) echo "  Add ${BIN_DIR} to your PATH to use lore from any directory." ;;
esac
