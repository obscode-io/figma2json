# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- License changed from MIT to Apache-2.0. Added a `NOTICE` file with the copyright and attribution notice;
  redistributions must keep it (Section 4(d) of the License). Version 0.0.1 on crates.io remains MIT.

## [0.0.1] - 2026-10-22

First release, published on [crates.io](https://crates.io/crates/figma2json).

### Added
- `f2j`: a short alias for the `figma2json` command, installed alongside it. Help, `--version` and error
  messages use the name it was run as. The CLI moved into the library as `figma2json::cli` (`cli` feature).
- Web app: System / Light / Dark theme switch in the header; the choice is remembered in the browser.
- Web app (`web/`): the converter compiled to WebAssembly, in a static page that converts a `.fig` entirely in
  the browser (in a Web Worker; the file is never uploaded). Options match the CLI flags; the JSON can be
  previewed, copied, or downloaded as `canvas.json` or as a ZIP with the images. Deployed to GitHub Pages
  (<https://obscode-io.github.io/figma2json/>) by the CI workflow; `web/build.sh` builds it
  locally.
- `figma2json-web` crate (workspace member) with the wasm-bindgen API: `convert`, `Conversion`, `Options`.
- Library: `convert_in_memory` (with `InMemoryConversion` and `OutputFile`) converts a `.fig` or its ZIP
  container without the filesystem; `parser::extract_zip_to_memory`, `schema::image_extension` and
  `schema::transform_image_hashes_with` are the building blocks.
- CI is a single workflow (`ci.yml`): fmt, Clippy, tests and the web build, then a Pages deploy from `main`.

- `figma2json` CLI: converts a `.fig` saved from Figma (a ZIP archive) into an output directory, or a bare
  `canvas.fig` to stdout / a file. Flags: `-o`, `--compact`, `--raw`, `--keep-internal`, `--legacy`, `-v`.
- Library API: `convert`, `convert_with_options` (`ConvertOptions { legacy, keep_internal }`), `convert_raw`,
  plus the parser, Kiwi decoder, tree builder, blob parser and every transformation pass as public modules.
- Design context in the output: node `id` and public `type` on every node; `componentId`, `componentName`,
  `componentSetName` on instances; `variable`, `variableCollection`, `variableId` next to every `ALIAS`;
  `fillStyle`, `strokeStyle`, `textStyle`, `effectStyle`, `gridStyle`; root `variables` and `styles` with
  per-mode values and usage counts.
- Auto-layout alignment and sizing, constraints, `scrollBehavior`, `layoutGrids` and override `guidPath`s are
  kept; every padding value Figma stores is kept (a missing side is 0). Internal `{sessionID, localID}`
  references become `"s:l"` strings.
- `--keep-internal`: keeps the hidden page with local copies of the library components the file uses.
- `--legacy`: a smaller output without design context (no ids, types, token or style names, auto-layout
  alignment/sizing, constraints, layout grids or override paths).
- Memory-conscious decoding: Kiwi data is decoded straight to JSON, glyphs and geometry are dropped before blob
  expansion, and output is streamed to disk.
- Python helpers in `tools/` that split `canvas.json` into an agent-friendly per-screen cache.
- Errors are typed (`FigError`: `InvalidMagicHeader`, `FileTooSmall`, `IncompleteChunk`, `NotEnoughChunks`,
  `CanvasNotFoundInZip`, `Decompression`, `Decode`, `InvalidDocument`, `InvalidBlob`, `Io`, `Zip`).

### Changed
- Web app: output is always compact JSON; the "Compact JSON" option is removed.
- Zstandard chunks are decoded with the pure-Rust `ruzstd` crate instead of the `zstd` C bindings, so the library
  has no C dependencies. Output is unchanged.
- The CLI's dependencies (`clap`, `anyhow`) are behind a default `cli` feature; library users can set
  `default-features = false`.
- The `zip` dependency is built with DEFLATE support only (no bzip2, zstd, AES or `time`).

[Unreleased]: https://github.com/obscode-io/figma2json/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/obscode-io/figma2json/releases/tag/v0.0.1
