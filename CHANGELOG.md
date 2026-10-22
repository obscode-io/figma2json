# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.0.1]

First release.

### Added
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
- CI: `cargo fmt --check`, Clippy with `-D warnings`, tests on stable and on the 1.88 MSRV, `cargo doc` with
  warnings denied.

[Unreleased]: https://github.com/obsidian-code-io/figma2json/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/obsidian-code-io/figma2json/releases/tag/v0.0.1
