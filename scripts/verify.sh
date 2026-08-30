#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

for required_command in cargo node npm; do
  if ! command -v "$required_command" >/dev/null 2>&1; then
    echo "Required command '$required_command' was not found in PATH." >&2
    exit 1
  fi
done

node_major="$(node -p "process.versions.node.split('.')[0]")"
if ((node_major < 24)); then
  echo "Node.js 24 or newer is required; found $(node --version)." >&2
  exit 1
fi

cd "$REPOSITORY_ROOT"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix apps/desktop ci
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
