# figma2json web app

A static page that converts Figma `.fig` files to JSON in the browser. The converter is the `figma2json`
library compiled to WebAssembly; the file is read and converted locally in a Web Worker and is never uploaded.

Published at <https://obscode-io.github.io/figma2json/>.

## Layout

```text
web/
├── Cargo.toml     figma2json-web crate (cdylib + rlib), a member of the root workspace
├── src/lib.rs     wasm-bindgen API
├── tests/         native tests of the conversion wrapper (cargo test -p figma2json-web)
├── build.sh       builds www/pkg/
└── www/
    ├── index.html
    ├── style.css
    ├── app.js     UI: file picking, options, preview, downloads
    ├── worker.js  loads the WebAssembly module and runs conversions off the main thread
    └── pkg/       wasm-bindgen output (generated, gitignored)
```

## Build

Requirements: a Rust toolchain (1.88 or newer) with the `wasm32-unknown-unknown` target, and
`wasm-bindgen-cli` at exactly the version of the `wasm-bindgen` crate (0.2.100). `wasm-opt` from
[Binaryen](https://github.com/WebAssembly/binaryen) is optional; when it is on `PATH` the build shrinks the
module with it (set `WASM_OPT=0` to skip).

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked
web/build.sh
```

The script runs `cargo build -p figma2json-web --release --target wasm32-unknown-unknown`, then
`wasm-bindgen --target web --out-dir web/www/pkg`. It can be run from any directory.

## Serve

The page uses ES modules and a module worker, so it must be served over HTTP (not opened as a `file://`
URL). Any static server works:

```bash
python3 -m http.server -d web/www 8000
# open http://localhost:8000
```

To publish elsewhere, copy `web/www/` (including `pkg/`) to any static host. Serve `.wasm` files as
`application/wasm`.

## JavaScript API

`pkg/figma2json_web.js` exports:

```js
import init, { convert, Options, version } from "./pkg/figma2json_web.js";

await init();
const options = new Options();
options.compact = false;      // --compact
options.legacy = false;       // --legacy
options.keepInternal = false; // --keep-internal
options.raw = false;          // --raw: also produce canvas.raw.json

const conversion = convert(new Uint8Array(await file.arrayBuffer()), options); // throws Error on failure
conversion.json;        // canvas.json text
conversion.rawJson;     // canvas.raw.json text, or undefined
conversion.fileNames;   // ["images/<hash>.png", "meta.json", ...] from the ZIP container
conversion.imageCount;
conversion.fileContents(0); // Uint8Array
conversion.toZip();     // Uint8Array: canvas.json (+ canvas.raw.json) and the files above
conversion.free();
```

The same API is available from Rust (`figma2json_web::Conversion::new`), which is how it is tested natively.
