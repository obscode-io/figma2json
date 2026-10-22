#!/usr/bin/env bash
# Builds the WebAssembly module and its JavaScript bindings into web/www/pkg/.
# Requires the wasm32-unknown-unknown target and wasm-bindgen-cli 0.2.100;
# runs wasm-opt too when it is installed (set WASM_OPT=0 to skip it).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out_dir="$root/web/www/pkg"
wasm_bindgen_version="0.2.100"

if ! command -v wasm-bindgen > /dev/null; then
    echo "wasm-bindgen not found. Install it with:" >&2
    echo "  cargo install wasm-bindgen-cli --version $wasm_bindgen_version --locked" >&2
    exit 1
fi
installed="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$installed" != "$wasm_bindgen_version" ]; then
    echo "wasm-bindgen $installed found, but the web crate needs $wasm_bindgen_version:" >&2
    echo "  cargo install wasm-bindgen-cli --version $wasm_bindgen_version --locked" >&2
    exit 1
fi

cargo build \
    --manifest-path "$root/Cargo.toml" \
    -p figma2json-web \
    --release \
    --locked \
    --target wasm32-unknown-unknown

rm -rf "$out_dir"
wasm-bindgen \
    --target web \
    --no-typescript \
    --out-dir "$out_dir" \
    "$target_dir/wasm32-unknown-unknown/release/figma2json_web.wasm"

wasm="$out_dir/figma2json_web_bg.wasm"
if [ "${WASM_OPT:-1}" != "0" ] && command -v wasm-opt > /dev/null; then
    wasm-opt -Oz \
        --enable-bulk-memory \
        --enable-mutable-globals \
        --enable-nontrapping-float-to-int \
        --enable-sign-ext \
        --enable-reference-types \
        --enable-multivalue \
        -o "$wasm.opt" "$wasm"
    mv "$wasm.opt" "$wasm"
fi

echo "Built $out_dir ($(wc -c < "$wasm" | tr -d ' ') bytes of WebAssembly)"
echo "Serve it with: python3 -m http.server -d web/www 8000"
