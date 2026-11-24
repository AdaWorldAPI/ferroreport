#!/usr/bin/env bash
# Builds the npm package into bindings/wasm/pkg: a Node.js build (`node/`, loads synchronously)
# and a web build (`web/`, for browsers, Deno and bundlers; call `await init()` first).
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli matching Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=../..
cargo build --release --target wasm32-unknown-unknown -p ferroreport-wasm --manifest-path "$ROOT/Cargo.toml"
WASM="${CARGO_TARGET_DIR:-$ROOT/target}/wasm32-unknown-unknown/release/ferroreport_wasm.wasm"
rm -rf pkg
wasm-bindgen --target nodejs --out-dir pkg/node --out-name ferroreport "$WASM"
wasm-bindgen --target web --out-dir pkg/web --out-name ferroreport "$WASM"
if command -v wasm-opt >/dev/null; then
  for f in pkg/node/ferroreport_bg.wasm pkg/web/ferroreport_bg.wasm; do
    wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int "$f" -o "$f"
  done
fi
cp package.json README.md "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" pkg/
ls -la pkg pkg/node
