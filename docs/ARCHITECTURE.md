# Architecture

This document describes how figma2json turns the bytes of a `.fig` file into JSON. Every claim
here refers to a source file. If the code and this document disagree, the code is right and this
document needs fixing.

## Data flow

```text
 .fig bytes (CLI: fs::read)
     |
     |  parser::is_zip_container  ("PK" prefix)                 src/parser/header.rs
     v
 +----------------------------+   yes   parser::extract_from_zip -> bytes of "canvas.fig"
 | ZIP container?             |-------> (CLI ZIP mode first extracts the whole archive to disk,
 +----------------------------+          then converts every *.fig it finds)      src/parser/chunks.rs
     | no
     v
 canvas.fig
 [8]  magic "fig-kiwi" | "fig-jam."   parser::detect_file_type -> FileType       src/parser/header.rs
 [4]  version (u32 LE)                parser::extract_chunks -> ParsedFile        src/parser/chunks.rs
 [4]  len0 [len0 bytes] chunk 0  (Kiwi binary schema)
 [4]  len1 [len1 bytes] chunk 1  (Kiwi message: the document)
 ...  further chunks (ignored by the pipeline)
     |
     |  parser::decompress_chunk: PNG/JPEG as-is, else raw DEFLATE, else zstd   src/parser/compression.rs
     v
 schema bytes + data bytes
     |
     |  schema::decode_fig_to_json: walk the Kiwi schema, write serde_json     src/schema/decoder.rs
     |  directly (enums -> {"__enum__", "value"}, `bytes` fields -> base64)
     v
 message { nodeChanges: [...], blobs: [...], ... }
     |
     |  take_array("nodeChanges") -> schema::build_tree (parentIndex)         src/pipeline.rs, src/schema/tree.rs
     |  take_array("blobs")
     v
 Decoded { version, file_type, document (rooted at 0:0), blobs }             src/pipeline.rs
     |
     +--> convert_raw: substitute_blobs -> assemble -> done (no passes)
     |
     v  transform()                                                          src/pipeline.rs
 1. PRUNE           (document)  text glyphs, path geometry
 2. blobs           blobs::process_blobs (byte arrays -> base64)              src/blobs/mod.rs
                    blobs::substitute_blobs (`xBlob: n` -> parsed `x`)        src/blobs/substitution.rs
 3. image hashes    schema::transform_image_hashes (hash -> images/<hex>.ext, renames files)
 4. NORMALIZE       (document)  matrix -> CSS transform, colors -> hex, enums -> strings, blendMode
 5. design context  schema::add_design_context (default mode only) -> DesignDefinitions
 6. CLEAN_DOCUMENT  (document)  guids, edit info, text layout, defaults, internal-only nodes ...
 7. assemble        { version, fileType, document, blobs } (+ variables, styles in default mode)
 8. CLEAN_OUTPUT    (whole root) root metadata, blobs, style ids, plugin data, empty objects ...
     |
     v
 serde_json::Value  -> CLI streams it to a file or stdout (pretty by default, --compact)
```

The stage tables and their `When` conditions are listed pass by pass in [PIPELINE.md](PIPELINE.md).

## Modules

| Module | File(s) | Responsibility |
| --- | --- | --- |
| crate root | `src/lib.rs` | Module declarations, re-exports, crate docs. `#![forbid(unsafe_code)]`. |
| `pipeline` (private) | `src/pipeline.rs` | `convert`, `convert_with_options`, `convert_raw`, `ConvertOptions`; the `decode` phase; the `When` enum; the four `const` stage tables; `transform`; pipeline tests. |
| `error` | `src/error.rs` | `FigError` (`thiserror`, `#[non_exhaustive]`) and `Result<T>`. |
| `types` | `src/types.rs` | `FileType` (`Figma` / `FigJam`, `as_str()` gives `"figma"` / `"figjam"`) and `ParsedFile { version, chunks }` with `schema_chunk()`, `data_chunk()`, `image_chunks()`. |
| `parser::header` | `src/parser/header.rs` | `detect_file_type` (8-byte magic), `is_zip_container` (`PK` prefix). |
| `parser::chunks` | `src/parser/chunks.rs` | `extract_from_zip` (returns the `canvas.fig` entry), `extract_zip_to_directory` (whole archive; entries escaping the target via absolute paths or `..` are skipped through `enclosed_name`), `extract_chunks` (version + length-prefixed chunks; a trailing partial length header is ignored; at least 2 chunks required). |
| `parser::compression` | `src/parser/compression.rs` | `is_already_compressed` (PNG/JPEG magic), `decompress_chunk` (raw DEFLATE first, then zstd). |
| `schema::decoder` | `src/schema/decoder.rs` | `decode_fig_to_json`: decodes the Kiwi schema, finds the root definition named `Message` that has both `nodeChanges` and `blobs` fields, and decodes the data chunk straight to `serde_json::Value`. Floats that are NaN/infinite become `null`. Byte arrays in a field named `bytes` become base64 strings; other byte arrays become number arrays. A private `kiwi_value_to_json` is kept under `#[allow(dead_code)]`. |
| `schema::tree` | `src/schema/tree.rs` | `build_tree`: indexes nodes by `"sessionID:localID"`, groups children by `parentIndex.guid`, sorts siblings by `parentIndex.position` (string order), removes `parentIndex`, and builds the tree from `0:0`. Nodes not reachable from `0:0` are dropped. |
| `schema::transformations` | `src/schema/transformations/*.rs` | One file per pass (55 files) plus `walk.rs` (`for_each_object_mut`, crate-private). |
| `blobs` | `src/blobs/mod.rs` | `process_blobs`: `bytes` number arrays -> base64 (decoded files already have base64, so this is a no-op for them). |
| `blobs::parser` | `src/blobs/parser.rs` | `parse_blob` (dispatch on the field name without `Blob`), `parse_commands` (0=Z, 1=M, 2=L, 3=Q, 4=C with little-endian f32 coordinates), `parse_vector_network` (vertices, segments, regions; bit 0 of a region's first u32 is the winding rule, `NONZERO` if set, else `ODD`). Truncated or invalid data yields `None` and the reference is left as is. |
| `blobs::substitution` | `src/blobs/substitution.rs` | `substitute_blobs`: for every key ending in `Blob` whose value is a valid index, replace `xBlob: n` with `x: <parsed>`. Unknown blob types and out-of-range indexes are left alone. |

## Public API

Crate root (`src/lib.rs`):

- `convert(bytes, base_dir) -> Result<Value>`: `convert_with_options` with `ConvertOptions::default()`.
- `convert_with_options(bytes, base_dir, &ConvertOptions) -> Result<Value>`.
- `convert_raw(bytes) -> Result<Value>`: decode + blob substitution, no passes.
- `ConvertOptions { legacy: bool, keep_internal: bool }` (both `false` by default).
- `FigError`, `Result`, `FileType`, `ParsedFile`.
- Public modules: `blobs`, `error`, `parser`, `schema`, `types`. `pipeline` is private.

`bytes` can be a ZIP container or a bare `canvas.fig`. `base_dir` is where `images/<hash>` files are looked
up and renamed (`.` when `None`).

`figma2json::schema` (`src/schema/mod.rs`) re-exports `decode_fig_to_json`, `build_tree`, every pass
(`remove_*`, `simplify_enums`, `simplify_text_properties`, `transform_colors_to_css`,
`transform_matrix_to_css`, `transform_image_hashes`, `add_design_context`) and `DesignDefinitions`,
so callers can assemble their own pipelines. `figma2json::parser` re-exports `extract_chunks`,
`extract_from_zip`, `extract_zip_to_directory`, `decompress_chunk`, `detect_file_type`,
`is_zip_container`. `figma2json::blobs` re-exports `parse_blob`, `parse_commands`,
`parse_vector_network`, `substitute_blobs` and defines `process_blobs`.

## Error model

The library returns `figma2json::Result<T>` = `Result<T, FigError>` (`src/error.rs`):

| Variant | Raised by |
| --- | --- |
| `InvalidMagicHeader(Vec<u8>)` | `detect_file_type`: first 8 bytes are neither `fig-kiwi` nor `fig-jam.` |
| `FileTooSmall { expected, actual }` | `detect_file_type` (< 8 bytes), `extract_chunks` (< 12 bytes) |
| `IncompleteChunk { offset, expected, actual }` | `extract_chunks`: a chunk length runs past the data |
| `NotEnoughChunks { expected, actual }` | `extract_chunks` (< 2 chunks); `decode` if a chunk accessor is empty |
| `CanvasNotFoundInZip` | `extract_from_zip` |
| `Decompression(String)` | `decompress_chunk`: both DEFLATE and zstd failed |
| `Decode(String)` | `decode_fig_to_json`: bad schema, no root `Message`, bad message data |
| `InvalidDocument(String)` | `decode` (`nodeChanges` / `blobs` missing), `build_tree` (missing or malformed guid / parentIndex guid, `0:0` or a child not found) |
| `InvalidBlob(String)` | `process_blobs`, `parse_blob` (no `bytes`, bad base64, non-byte values), `substitute_blobs` |
| `Io(io::Error)`, `Zip(ZipError)` | `#[from]` conversions in the parser |

Every pass returns `Result<()>`; most cannot fail. `add_design_context` propagates the result of the
nested `simplify_text_properties` call. The CLI (`src/main.rs`) wraps library errors with `anyhow`
context such as `Failed to convert .fig file to JSON` or `Failed to convert: <path>`.

## Memory design

Large `.fig` files carry tens of megabytes of blob bytes and hundreds of thousands of nodes. The code
avoids materialising data it will drop:

- **Decode straight to JSON** (`src/schema/decoder.rs`): no intermediate `kiwi_schema::Value` tree, and
  `bytes` arrays become one base64 string instead of one JSON number per byte.
- **Move, do not clone**: `take_array` moves `nodeChanges` and `blobs` out of the decoded message
  (`std::mem::take`); `build_tree` takes nodes out of its map as it builds the tree.
- **Prune before substituting blobs**: `PRUNE` removes `derivedTextData.glyphs` and
  `fillGeometry` / `strokeGeometry` (except on icon/image nodes) before `substitute_blobs`, so the
  blobs they reference are never parsed into JSON. Neither pass reads blob content, so the output is
  the same as pruning afterwards.
- **Stream output**: the CLI serialises with `serde_json::to_writer[_pretty]` through a `BufWriter`
  instead of building one string (`write_json_file`, `write_json_stdout`).
- **One conversion in memory at a time**: with `--raw`, the transformed JSON is written and dropped
  before `convert_raw` decodes the file again (`convert_one` in `src/main.rs`).

The processed `blobs` array is still carried into the assembled root and only removed by
`remove_root_blobs` in CLEAN_OUTPUT.

## CLI

`figma2json <INPUT> [OUTPUT_DIR] [OPTIONS]` (`src/main.rs`, clap derive). The input is read fully into
memory and sniffed with `is_zip_container`.

**ZIP mode** (input starts with `PK`): `OUTPUT_DIR` is required (error otherwise) and `-o` is rejected.
The archive is extracted to `OUTPUT_DIR` (`extract_zip_to_directory`), then every `*.fig` under it is
found recursively, sorted, and converted. Each `X.fig` produces `X.json` next to it (and `X.raw.json`
with `--raw`), with `base_dir` = the `.fig`'s directory, so `images/` from the archive is found and
renamed in place. No `.fig` found is an error.

**Bare mode** (anything else, normally an extracted `canvas.fig`): JSON goes to `-o FILE` or stdout.
`base_dir` is the parent of `-o` when given, otherwise the current directory (not the input's
directory). With `--raw`, the raw JSON goes to `<-o or INPUT>` with its extension replaced by
`raw.json`, even when the main output goes to stdout. An `OUTPUT_DIR` argument is accepted but
silently ignored in this mode.

| Flag | Effect |
| --- | --- |
| `-o, --output FILE` | Bare mode only: write to a file instead of stdout. |
| `--compact` | `serde_json::to_writer` instead of `to_writer_pretty`. |
| `--raw` | Also write `convert_raw` output (`*.raw.json`). |
| `--legacy` | `ConvertOptions::legacy`: no design context, more fields removed. |
| `--keep-internal` | `ConvertOptions::keep_internal`: skip `remove_internal_only_nodes`. |
| `-v, --verbose` | Progress on stderr. |

Stdout output ends with a newline; file output does not. When the reader of stdout goes away
(`| head`, `| jq` exiting), `write_json_stdout` treats `BrokenPipe` (found in the `anyhow` chain as an
`io::Error` or a `serde_json::Error` with an I/O kind) as success and exits 0. Writes to files report
errors with the path as context.

## Output key order

`serde_json` is used without the `preserve_order` feature, so objects are `BTreeMap`s and every
object in the output has its keys in sorted order, independent of the order in the `.fig` file.
