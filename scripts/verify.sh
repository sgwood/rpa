#!/bin/zsh
set -euo pipefail

REPOSITORY_ROOT="${0:A:h:h}"
CARGO_BIN="${CARGO_BIN:-$HOME/.cargo/bin/cargo}"
NODE_24="${NODE_24:-$HOME/.nvm/versions/node/v24.12.0/bin/node}"
NPM_CLI="${NPM_CLI:-$HOME/.nvm/versions/node/v24.12.0/lib/node_modules/npm/bin/npm-cli.js}"
NODE_BIN_DIR="${NODE_24:h}"

cd "$REPOSITORY_ROOT"
"$CARGO_BIN" fmt --all -- --check
"$CARGO_BIN" clippy --workspace --all-targets -- -D warnings
"$CARGO_BIN" test --workspace --no-fail-fast
cd "$REPOSITORY_ROOT/apps/desktop"
env PATH="$NODE_BIN_DIR:/usr/local/bin:/usr/bin:/bin" "$NODE_24" "$NPM_CLI" ci
env PATH="$NODE_BIN_DIR:/usr/local/bin:/usr/bin:/bin" "$NODE_24" "$NPM_CLI" test
env PATH="$NODE_BIN_DIR:/usr/local/bin:/usr/bin:/bin" "$NODE_24" "$NPM_CLI" run build
