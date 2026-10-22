# Web app

The browser app converts `.fig` files with the same library the CLI uses, compiled to WebAssembly. The file
is read and converted in the browser; nothing is uploaded. Build and serve instructions for users are in
[`web/README.md`](../web/README.md); this page is the design reference.

## Pieces

```text
Cargo.toml (root)          [workspace] members = ["web"]; root package = library + CLI
src/pipeline.rs            convert_in_memory: filesystem-free conversion (used by the web crate)
web/Cargo.toml             figma2json-web: cdylib + rlib, depends on figma2json with default-features = false
web/src/lib.rs             wasm-bindgen API: Options, Conversion, convert(), version()
web/tests/                 native tests of the wrapper (cargo test -p figma2json-web)
web/build.sh               cargo build --target wasm32-unknown-unknown + wasm-bindgen --target web (+ wasm-opt if present)
web/www/index.html         page markup
web/www/style.css          light/dark via prefers-color-scheme, responsive down to 375px
web/www/app.js             UI: drop zone / file picker, options, status, preview, copy, downloads
web/www/worker.js          module Web Worker: loads pkg/, runs convert() and toZip() off the main thread
web/www/pkg/               wasm-bindgen output (generated, gitignored)
.github/workflows/ci.yml   job "web": wasm32 clippy + web/build.sh
.github/workflows/pages.yml deploys web/www to GitHub Pages on push to main
```

## Core library support

What the library provides so it can run without a filesystem:

- **Cargo features.** `cli` (default) enables `clap` and `anyhow`; the `figma2json` binary has
  `required-features = ["cli"]`. The web crate depends on the library with `default-features = false`.
- **Pure-Rust decompression.** Zstandard chunks are decoded with `ruzstd`, so the same code builds for
  native and wasm32 without a C toolchain. The C `zstd` crate is a dev-dependency only, used in tests to
  check `ruzstd` against reference output. `zip` is built with only its `deflate` feature.
- **In-memory ZIP.** `parser::extract_zip_to_memory` returns `(path, bytes)` entries instead of writing
  to disk.
- **Image extensions without renaming files.** `schema::image_extension(&[u8])` detects PNG, JPEG, WebP,
  GIF or SVG from bytes. `schema::transform_image_hashes_with(tree, |filename| -> Option<&str>)` asks a
  closure for each image's extension. The disk version `transform_image_hashes` is a wrapper that detects
  from the file and renames it.
- **`convert_in_memory(bytes, &ConvertOptions) -> InMemoryConversion { json, files }`** runs the full
  pipeline. For a ZIP container it returns every other archive entry as an `OutputFile { path, contents }`,
  with images renamed to `images/<hash>.<ext>` as the JSON references them. For a bare `canvas.fig`,
  `files` is empty.

## Web crate API

`web/src/lib.rs` is a thin wrapper. The plain-Rust core (`Conversion::new(&[u8], &Options)`,
`Conversion::zip()`) returns `String` errors so it is testable natively; the `#[wasm_bindgen]` methods wrap
it and turn errors into JS `Error`s.

| JS                                  | Meaning                                                              |
| ----------------------------------- | -------------------------------------------------------------------- |
| `new Options()`                     | `compact`, `legacy`, `keepInternal`, `raw` booleans (CLI flags)      |
| `convert(bytes, options)`           | Returns a `Conversion`; throws on invalid input                       |
| `conversion.json`                   | `canvas.json` text (pretty or compact)                               |
| `conversion.rawJson`                | `canvas.raw.json` text when `raw` is set, else `undefined`           |
| `conversion.fileNames`              | Other ZIP entries: `images/<hash>.<ext>`, `meta.json`, `thumbnail.png` |
| `conversion.fileContents(i)`        | Bytes of entry `i`                                                   |
| `conversion.imageCount`             | Number of `images/` entries                                          |
| `conversion.toZip()`                | ZIP of `canvas.json` (+ `canvas.raw.json`) and the entries above     |
| `version()`                         | Crate version                                                        |

## Front end

- Plain HTML, CSS and ES modules: no npm, no bundler. The page must be served over HTTP (module worker).
- `app.js` posts `{type: "convert", bytes, options}` to the worker and receives `status`, `converted`,
  `zip`, `version` and `error` messages; the `.fig` buffer and the ZIP are transferred, not copied.
- Changing an option re-runs the conversion on the last file. The ZIP is built only when requested.
- The preview shows the first 100 KB of JSON; copy and downloads use the full text.

## Differences from the CLI

- Output is the same as the CLI for the same file and flags, with one exception: when the same image
  hash is referenced more than once, the CLI gives only the first reference an extension (it renames the
  file on disk at the first reference; see Known issues in [PIPELINE.md](PIPELINE.md)). The web app gives
  every reference the extension.
- Very large files are limited by browser memory (a 4 GB wasm32 address space at most).

## Building and testing

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked   # must equal the wasm-bindgen crate version
web/build.sh
python3 -m http.server -d web/www 8000
cargo test -p figma2json-web
cargo clippy -p figma2json-web --target wasm32-unknown-unknown --locked -- -D warnings
```

`wasm-bindgen` is pinned to `=0.2.100` in `web/Cargo.toml`; when upgrading it, upgrade the CLI in CI
(`ci.yml`, `pages.yml`) and locally to the same version. The release `.wasm` is about 512 KB before
`wasm-opt`.

To check the browser output against the CLI, convert the same file with both (for example a file from
`tools/make_test_fig.py --zip`) and compare `canvas.json` and the ZIP contents byte for byte.

## Deployment

`pages.yml` builds with `web/build.sh` and publishes `web/www` with `actions/upload-pages-artifact` and
`actions/deploy-pages` on every push to `main`. Enable it once in the repository settings
(Settings → Pages → Source: GitHub Actions). The site is served at
`https://obsidian-code-io.github.io/figma2json/`.
