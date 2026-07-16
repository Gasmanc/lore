#!/usr/bin/env bash
# Build package databases from packages/**.yaml specs locally, then assemble a
# registry index.json. Replaces the GitHub Actions "Build Registry" job so no CI
# minutes are consumed. Unlike CI this CAN build `website` sources (live crawl).
#
# Usage:
#   scripts/build-registry.sh                     # build every spec
#   scripts/build-registry.sh packages/npm/next.yaml   # build one spec
#
# Requires: a `lore` binary (built by the `build` just-target, or on PATH) and
# `yq` (mikefarah) + `jq`.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

LORE="${LORE_BIN:-./target/release/lore}"
if [ ! -x "$LORE" ]; then
    LORE="$(command -v lore || true)"
fi
if [ -z "$LORE" ] || [ ! -x "$LORE" ]; then
    echo "error: no lore binary found (build with \`just build\` or set LORE_BIN)" >&2
    exit 1
fi
for tool in yq jq; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' is required" >&2; exit 1; }
done

OUTDIR="dist"
mkdir -p "$OUTDIR"

# Collect specs.
specs=()
if [ -n "${1:-}" ]; then
    specs=("$1")
else
    while IFS= read -r -d '' f; do specs+=("$f"); done \
        < <(find packages -maxdepth 2 -name '*.yaml' -print0 | sort -z)
fi
if [ ${#specs[@]} -eq 0 ]; then
    echo "no specs found under packages/" >&2
    exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

build_one() {
    local spec="$1"
    local name version registry description source_url source_type
    name=$(yq -r '.name // ""' "$spec")
    version=$(yq -r '.version // ""' "$spec")
    registry=$(yq -r '.registry // ""' "$spec")
    description=$(yq -r '.description // ""' "$spec")
    source_url=$(yq -r '.source_url // ""' "$spec")
    source_type=$(yq -r '.source.type // ""' "$spec")

    local db="$OUTDIR/${registry}-${name}@${version}.db"
    local build_args=(--name "$name" --version "$version" --registry "$registry" --output "$db")
    [ -n "$description" ] && build_args+=(--description "$description")
    [ -n "$source_url" ] && build_args+=(--source-url "$source_url")

    case "$source_type" in
        git)
            local url branch subdir src
            url=$(yq -r '.source.url // ""' "$spec")
            branch=$(yq -r '.source.branch // ""' "$spec")
            subdir=$(yq -r '.source.subdir // ""' "$spec")
            case "$subdir" in
                /*|*..*) echo "::error:: unsafe source.subdir '$subdir' in $spec" >&2; return 1 ;;
            esac
            local clone_dir="$workdir/src"
            rm -rf "$clone_dir"
            local clone_args=(--depth 1)
            [ -n "$branch" ] && clone_args+=(--branch "$branch")
            git clone "${clone_args[@]}" -- "$url" "$clone_dir"
            src="$clone_dir${subdir:+/$subdir}"
            "$LORE" build "$src" "${build_args[@]}"
            ;;
        local)
            local dir
            dir=$(yq -r '.source.dir // ""' "$spec")
            "$LORE" build "$dir" "${build_args[@]}"
            ;;
        website)
            # Local builds CAN crawl; `lore update` drives the same path.
            "$LORE" build-website "$(yq -r '.source.url' "$spec")" "${build_args[@]}"
            ;;
        *)
            echo "error: unknown source type '$source_type' in $spec" >&2
            return 1
            ;;
    esac
}

echo "Building ${#specs[@]} package(s)…"
for spec in "${specs[@]}"; do
    echo "── $spec"
    build_one "$spec"
done

# Assemble index.json from the JSON manifest sidecars `lore build` wrote.
REPO="${GITHUB_REPOSITORY:-Gasmanc/lore}"
mapfile -d '' jsons < <(find "$OUTDIR" -name '*.json' ! -name 'index.json' -print0 | sort -z)
if [ ${#jsons[@]} -eq 0 ]; then
    echo '[]' > "$OUTDIR/index.json"
else
    jq -s --arg repo "$REPO" \
        'map(. + {download_url: ("https://github.com/\($repo)/releases/download/registry/" + .package.registry + "-" + .package.name + "@" + .package.version + ".db")})' \
        "${jsons[@]}" > "$OUTDIR/index.json"
fi

echo "✓ registry built in $OUTDIR/ (index.json + $(ls "$OUTDIR"/*.db 2>/dev/null | wc -l) databases)"
