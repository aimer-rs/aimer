#!/bin/sh
# Build the language services as WebAssembly and copy them next to the VS Code extension.
# Usage: scripts/build-wasm.sh [destination.wasm]   (default: ../../../AimerRustVsCode/aimer_shader.wasm)
set -eu
cd "$(dirname "$0")/../../.."
# The VS Code extension still loads the module under its old file name.
dest="${1:-../AimerRustVsCode/aimer_shader.wasm}"
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo rustc -p aimer_transpiler --lib --features lsp --crate-type cdylib --target wasm32-unknown-unknown --profile wasm
cp "${CARGO_TARGET_DIR:-target}"/wasm32-unknown-unknown/wasm/aimer_transpiler.wasm "$dest"
echo "wrote $dest ($(wc -c < "$dest") bytes)"
