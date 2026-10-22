# CLAUDE.md

figma2json converts Figma `.fig` files (a ZIP holding a binary `canvas.fig`) into compact JSON that an
LLM or a code generator can use to implement the design. It is a Rust crate with a CLI (`src/main.rs`)
and a library (`src/lib.rs`). Version 0.0.1, MIT, MSRV 1.88.

Read these before larger changes:

- `docs/ARCHITECTURE.md`: data flow from bytes to JSON, modules, public API, errors, memory, CLI modes
- `docs/PIPELINE.md`: every transformation pass in order, its `When` condition and ordering constraints,
  plus known issues
- `docs/OUTPUT_FORMAT.md`: the JSON the tool writes (default, `--legacy`, `--keep-internal`, `--raw`)
- `docs/DEVELOPMENT.md`: setup, tests, output-equivalence checks, release process, `tools/` scripts
- `docs/DECISIONS.md`: why things are the way they are

## Repo map

```text
src/main.rs                     CLI (clap): ZIP-to-directory mode, bare canvas.fig mode, output writing
src/lib.rs                      crate root: re-exports convert*, ConvertOptions, FigError, Result, FileType
src/pipeline.rs                 decode phase + declarative pass tables (PRUNE, NORMALIZE, CLEAN_DOCUMENT, CLEAN_OUTPUT)
src/error.rs                    FigError (typed variants) and Result
src/types.rs                    FileType, ParsedFile
src/parser/                     header.rs (magic, ZIP sniff), chunks.rs (ZIP, chunks), compression.rs
src/schema/decoder.rs           Kiwi schema + message decoded straight to serde_json
src/schema/tree.rs              flat nodeChanges -> tree via parentIndex
src/schema/transformations/     one file per pass; walk.rs holds the shared walker
src/blobs/                      base64 blobs, parse commands / vectorNetwork, substitute *Blob refs
tools/                          Python helpers (cache builder, cache verifier, audit, test .fig generator)
.github/workflows/ci.yml        fmt, clippy, test (stable + 1.88), doc
```

## Commands (exactly what CI runs)

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
RUSTDOCFLAGS=-D warnings cargo doc --no-deps --locked
```

CI also runs `cargo build --locked` and `cargo test --locked` on toolchain 1.88. The MSRV is 1.88 because
it is the declared `rust-version` of both crates and is tested in CI; the newest dependency floor is
`ruzstd 0.9` (1.87).
Always pass `--locked`; do not update `Cargo.lock` casually (it can raise the MSRV).
`Cargo.toml` sets `unsafe_code = "forbid"` and Clippy `all = deny`.

Run the CLI locally: `cargo run -- design.fig out` (ZIP) or `cargo run -- out/canvas.fig` (stdout).
Synthetic input: `python3 -I tools/make_test_fig.py /tmp/t.fig [--zip]`.

## Architecture in brief

`decode` (src/pipeline.rs): ZIP? -> `extract_from_zip` -> magic header (`fig-kiwi` / `fig-jam.`) ->
length-prefixed chunks -> each chunk raw DEFLATE or zstd -> Kiwi schema (chunk 0) decodes the message
(chunk 1) straight to JSON -> `nodeChanges` built into a tree rooted at `0:0` -> `blobs` taken out.

`transform`: PRUNE -> blob substitution -> image hashes -> NORMALIZE -> design context (default mode
only) -> CLEAN_DOCUMENT -> assemble root (`version`, `fileType`, `document`, `blobs`, plus `variables`
and `styles`) -> CLEAN_OUTPUT. `convert_raw` stops after blob substitution and assembly.
Details: `docs/ARCHITECTURE.md`, `docs/PIPELINE.md`.

## Conventions (MUST follow)

- **No `//` code comments** in Rust. Anything worth saying goes in `///` / `//!` rustdoc. Python: use
  docstrings; no `#` comments except the shebang and `# noqa` markers. Comments were removed on purpose.
- **No `unwrap()` / `expect()` outside tests.** Propagate with `?` and a typed error.
- **Typed `FigError` variants** (src/error.rs), never a catch-all:
  - `InvalidMagicHeader`: first 8 bytes are not `fig-kiwi` / `fig-jam.`
  - `FileTooSmall`: shorter than the header (8) or header + version (12)
  - `IncompleteChunk`: a chunk length runs past the end of the data
  - `NotEnoughChunks`: no schema + data chunk
  - `CanvasNotFoundInZip`: ZIP without a `canvas.fig` entry
  - `Decompression`: neither raw DEFLATE nor zstd worked
  - `Decode`: Kiwi schema/message decoding failed, or no root `Message` definition
  - `InvalidDocument`: structurally wrong decoded data (missing `nodeChanges`/`blobs`, bad guid, no `0:0`)
  - `InvalidBlob`: blob bytes missing, out of range or not base64
  - `Io` / `Zip`: wrapped via `#[from]`
  The enum is `#[non_exhaustive]`; adding a variant is fine, reusing an unrelated one is not.
  The CLI uses `anyhow` with `.context(...)` on top.
- **Transformation passes** have the signature `fn(&mut JsonValue) -> Result<()>` (alias `Pass` in
  src/pipeline.rs) and use `schema::transformations::walk::for_each_object_mut` unless they must filter
  array elements (then a small recursive function, as in `internal_only_nodes_removal.rs`).
- **Passes are registered only in the `const` tables in src/pipeline.rs** with a `When` condition
  (`Always`, `LegacyOnly`, `DefaultOnly`, `UnlessKeepInternal`). Order is significant; see
  `docs/PIPELINE.md` before moving anything.
- **Output must stay byte-identical** unless a change is intended. Compare CLI output before/after on a
  real `.fig` and on `tools/make_test_fig.py` output, in all four `--legacy` / `--keep-internal`
  combinations. Intended output changes go in `CHANGELOG.md`.
- **Memory-conscious ordering**: glyph and geometry pruning run before blob substitution so those blobs
  are never expanded into JSON. Keep large values moved (`std::mem::take`), not cloned.
- **Versioning**: the version is 0.0.1. `CHANGELOG.md` follows Keep a Changelog with an `[Unreleased]`
  section on top; add entries there.
- **Git**: commits are authored as `mtalhazulf <talhazulf4163@gmail.com>` only. No `Co-Authored-By` or
  any other attribution trailers in commit messages.
- **No credits to other projects** in docs or code. The copyright lines in `LICENSE` stay as they are
  (legally required); do not edit `LICENSE`.

## Adding a transformation pass

1. Create `src/schema/transformations/<name>.rs` with `pub fn <verb>_<what>(tree: &mut JsonValue) -> Result<()>`.
   Use `for_each_object_mut` from `super::walk` unless the pass must remove array elements.
2. Document it with `///` (what it removes or rewrites, and a `no_run` example in the existing style).
   No `//` comments.
3. Add `#[cfg(test)] mod tests` in the same file covering: the field is changed, unrelated fields are
   kept, nested objects and arrays are reached, and values that must not match are left alone.
4. Add `pub mod <name>;` and `pub use <name>::<fn>;` in `src/schema/transformations/mod.rs` and the
   re-export in `src/schema/mod.rs` (keep the lists alphabetical).
5. Register `(schema::<fn>, When::...)` in the right table in `src/pipeline.rs`. Check what it depends
   on: enums are plain strings only after `simplify_enums`; guids and internal-only nodes are gone after
   CLEAN_DOCUMENT; `variables` / `styles` exist only in CLEAN_OUTPUT.
6. If behaviour differs by mode, extend the pipeline tests at the bottom of `src/pipeline.rs`.
7. Run the four CI commands, compare CLI output before/after, update `docs/PIPELINE.md`
   (and `docs/OUTPUT_FORMAT.md` if the output changes), and add a `CHANGELOG.md` entry under `[Unreleased]`.

## Web app

The `web/` crate is the WebAssembly browser app (workspace member; the root package stays the library + CLI).
It uses `convert_in_memory` (no filesystem) and is built with `web/build.sh` (wasm-bindgen pinned to
0.2.100). The library's `cli` feature (default) gates `clap`/`anyhow`; keep the core free of filesystem
and C dependencies so it still builds for `wasm32-unknown-unknown`. CI also runs
`cargo clippy -p figma2json-web --target wasm32-unknown-unknown --locked -- -D warnings` and `web/build.sh`;
local workspace checks use `--workspace`. Details: docs/WEB.md.
