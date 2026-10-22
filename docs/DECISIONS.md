# Decisions

A short log of the decisions made so far, in the style of architecture decision records. Each entry
gives the context, the decision and its consequences. Add new entries at the end. Do not rewrite old
ones; supersede them with a new entry.

## 1. Typed error variants instead of a catch-all

**Context.** One `ZipError` variant was used for decoding, decompression, tree and blob failures that had
nothing to do with ZIP files, so callers could not tell them apart.

**Decision.** `FigError` (`src/error.rs`) has one variant per failure kind: `InvalidMagicHeader`,
`FileTooSmall`, `IncompleteChunk`, `NotEnoughChunks`, `CanvasNotFoundInZip`, `Decompression`, `Decode`,
`InvalidDocument`, `InvalidBlob`, plus `Io` and `Zip` via `#[from]`. The enum is `#[non_exhaustive]`.
No `unwrap` / `expect` outside tests.

**Consequences.** New failure kinds get a new variant, not a reused one. Adding a variant is not a
breaking change for library users.

## 2. Declarative pipeline tables

**Context.** Conversion used to be about 60 pass calls written out by hand, with numbered comments that
had drifted from the code, and duplicated decode logic between the normal and raw paths.

**Decision.** `src/pipeline.rs` has one `decode` phase shared by `convert_with_options` and
`convert_raw`, and four `const` tables (`PRUNE`, `NORMALIZE`, `CLEAN_DOCUMENT`, `CLEAN_OUTPUT`) of
`(Pass, When)` pairs, run by `run()`. Passes share the signature `fn(&mut Value) -> Result<()>`. Steps
with other signatures (blob substitution, image hashes, design context) are called directly from
`transform`.

**Consequences.** The order and the mode of every pass can be read in one place. The restructuring was
checked to produce identical output to the previous code on generated documents in all four
`legacy` / `keep_internal` combinations. Adding a pass means adding a table entry.

## 3. A shared walker for passes

**Context.** Each pass had its own recursive traversal, with small differences.

**Decision.** `schema::transformations::walk::for_each_object_mut` visits every object depth-first,
parent before children. Passes use it unless they must filter array elements (`internal_only_nodes_removal`,
`overridden_symbol_id_removal`, `visible_only_objects_removal`, `empty_objects_removal`), need a
different order (`image_hash`, which renames files in document order), or replace values in place
(`enum_simplification`, `design_context`).

**Consequences.** Less code, one place to reason about traversal. Anything a pass removes from an object
is not visited; anything it inserts is.

## 4. Decode Kiwi straight to JSON

**Context.** Decoding into `kiwi_schema::Value` first and converting that to JSON used several GB of
memory on large files: blob bytes became `Value::Byte` items and then one JSON number per byte.

**Decision.** `schema::decode_fig_to_json` walks the schema and writes `serde_json::Value` directly.
`bytes` fields become base64 strings. Large arrays are moved out of the message, not cloned.

**Consequences.** Lower peak memory. The decoder mirrors `kiwi_schema::Value::decode_bb`, so changes to
the Kiwi format would need to be mirrored by hand.

## 5. Prune before blob substitution

**Context.** Glyph outlines and path geometry are dropped from the output anyway, but substituting their
blobs first expanded them into large JSON arrays.

**Decision.** The `PRUNE` stage removes `derivedTextData.glyphs` and `fillGeometry` /
`strokeGeometry` before `substitute_blobs`. Neither pass reads blob content, so the output is unchanged.

**Consequences.** Blobs referenced only from pruned data are never parsed. Any new pass that discards
blob-backed data should go into `PRUNE` for the same reason.

## 6. Design context by default, `--legacy` for smaller output

**Context.** The original output stripped ids, types, variable and style names, and auto-layout
alignment and sizing. That made it compact but not enough to implement a design faithfully.

**Decision.** The default mode runs `add_design_context` and keeps node `id` / `type`, component links,
variable and style names, root `variables` / `styles`, auto-layout alignment and sizing, constraints,
scroll behaviour, layout grids and override `guidPath`s. `--legacy` (`ConvertOptions::legacy`) restores
the smaller output without them.

**Consequences.** Passes that only apply to one mode use `When::LegacyOnly` / `When::DefaultOnly`.
Design context must run before guid and internal-only node removal.

## 7. Keep every padding value

**Context.** Figma omits padding that is 0. Removing right/bottom padding when equal to left/top makes a
missing value ambiguous: it could mean 0 or "same as the other side".

**Decision.** `remove_redundant_padding` runs only in legacy mode. The default output keeps every padding
value Figma stores; a missing side means 0.

## 8. MSRV 1.88

**Context.** Originally the locked tree (`zip 0.6` -> `time 0.3.55`) required 1.88. Since `zip` is built
without default features and zstd uses `ruzstd`, the newest dependency floor is `ruzstd 0.9` (1.87).

**Decision.** `rust-version = "1.88"` in `Cargo.toml`; CI tests on stable and 1.88 with `--locked`.

**Consequences.** Dependency updates that raise the MSRV need a deliberate decision and a new entry here.

## 9. No inline comments

**Context.** Inline `//` comments had drifted from the code they described.

**Decision.** No `//` comments in Rust code. Explanations go in `///` / `//!` rustdoc, which is built in
CI with warnings denied, or in `docs/`. In Python, docstrings only; no `#` comments except the shebang
and `# noqa` markers.

**Consequences.** Documentation that matters is visible in `cargo doc` and in these files. Reviews should
reject new `//` comments.

## 10. Version reset to 0.0.1

**Context.** The crate was at 0.4.0 from earlier development, a number that did not reflect this
project's own release history.

**Decision.** The version was set to 0.0.1, with a single `[0.0.1]` entry in `CHANGELOG.md`
(Keep a Changelog, `[Unreleased]` on top). Version references were removed from the tools.

**Consequences.** Releases follow Semantic Versioning from 0.0.1, as described in
[DEVELOPMENT.md](DEVELOPMENT.md#release-process).

## 11. Browser app via WebAssembly, CLI kept

**Context.** Not every user can or wants to install a Rust toolchain. Design files are often
confidential, so uploading them to a server for conversion is a privacy problem, and a server would need
hosting and maintenance.

**Decision.** Provide a browser app that runs the converter compiled to WebAssembly, entirely client-side,
so the `.fig` file never leaves the user's machine. It can be served as static files. The CLI stays the
main tool for scripting and large files. The app lives in a `web/` crate and is documented in
`docs/WEB.md`.

**Implementation.** The core gained `convert_in_memory`, `extract_zip_to_memory`, `image_extension` and
`transform_image_hashes_with`; `clap`/`anyhow` moved behind a default `cli` feature; zstd decoding moved
to pure-Rust `ruzstd` so one decoder serves native and wasm32 without a C toolchain.

**Consequences.** The library must stay usable from WASM: the core conversion must not depend on
threads or the file system beyond what the browser provides. `transform_image_hashes` (disk, renames files) is
now a wrapper over the closure-based `transform_image_hashes_with`, which the browser path uses.
