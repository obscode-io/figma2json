# Development

## Setup

- Rust 1.88 or newer (`rustup`). 1.88 is the MSRV (check locally with `cargo +1.88`). It is the declared
  `rust-version`; the newest dependency floor is `ruzstd 0.9` (1.87). For the web app also install the
  `wasm32-unknown-unknown` target and `wasm-bindgen-cli` 0.2.100 (see [WEB.md](WEB.md)).
- Components: `rustfmt`, `clippy`.
- Python 3 for `tools/` (standard library only).

```bash
cargo build --locked              # debug binary: target/debug/figma2json
cargo build --release --locked    # release: LTO, 1 codegen unit, stripped (Cargo.toml [profile.release])
cargo install --path . --locked   # install f2j and figma2json into ~/.cargo/bin
```

Always pass `--locked`: CI does, and an unintended `Cargo.lock` update can raise the MSRV.

## Running the CLI locally

```bash
cargo run -- design.fig out                     # ZIP from Figma: extracts to out/, writes out/canvas.json
cargo run -- out/canvas.fig                     # bare canvas.fig: JSON to stdout
cargo run -- out/canvas.fig -o design.json      # ... or to a file
cargo run -- design.fig out --raw -v            # also out/canvas.raw.json, progress on stderr
cargo run -- design.fig out --legacy --compact
cargo run -- design.fig out --keep-internal
```

In bare mode images are looked up in the directory of `-o`, or the current directory without `-o`
(see [ARCHITECTURE.md](ARCHITECTURE.md#cli)). Run from the extraction directory, or use ZIP mode, when
image filenames matter.

### A synthetic `.fig`

`tools/make_test_fig.py` writes a small valid file without needing Figma (standard library only):

```bash
python3 -I tools/make_test_fig.py /tmp/t.fig          # bare canvas.fig
python3 -I tools/make_test_fig.py /tmp/tz.fig --zip   # ZIP container with canvas.fig + images/<hash>
cargo run -- /tmp/t.fig
cargo run -- /tmp/tz.fig /tmp/tz-out
```

It encodes a Kiwi schema and message by hand (raw DEFLATE chunks, `fig-kiwi` header, version 48):
DOCUMENT `0:0` > CANVAS `0:1` "Page 1" > FRAME `1:2` "Frame" (solid fill `#ff8000`) > VECTOR `1:3`
"Vector" with a `commandsBlob` and a `vectorNetworkBlob`. With `--zip` the frame also has an IMAGE paint
whose hash points at a 1x1 PNG in the archive. The output is deterministic. It covers ZIP extraction,
chunk parsing, DEFLATE, Kiwi decoding, tree building, both blob parsers, image renaming, colors, enums
and default removal, but not variables, styles, components, text or auto-layout. Use real files for those.

## Tests

```bash
cargo test --locked          # unit tests + doctests (about 630 unit tests and 67 doctests)
cargo test --locked design_context   # filter by name
```

- **Unit tests per pass**: every file in `src/schema/transformations/` has a `#[cfg(test)] mod tests`
  that runs the pass on small `json!` trees. Same for `src/parser/*`, `src/blobs/*`,
  `src/schema/tree.rs` and `walk.rs`.
- **Pipeline tests** at the bottom of `src/pipeline.rs`: `When::applies` for all four mode
  combinations, default vs legacy output of a small decoded document, `--keep-internal`, raw assembly,
  and `take_array` errors. They call the private `transform` directly with a `Decoded` value, so they do
  not need a `.fig` file.
- **Doctests**: the `///` examples. Examples that need files or show only the idea are `no_run`;
  runnable ones (for example in `src/parser/header.rs`, `src/parser/compression.rs`,
  `src/blobs/mod.rs`, `src/blobs/substitution.rs`) are checked by `cargo test`.

Pass unit tests check a pass in isolation. They do not catch ordering problems (PIPELINE.md known
issue 1 passes its own tests). When a pass depends on what an earlier stage produced, test it with input
in the form it has at that point in the pipeline, or add a pipeline test.

## Checking that a refactor does not change the output

Output must stay byte-identical unless the change is meant to alter it. Compare the CLI before and
after on the same inputs, in all four mode combinations:

```bash
git switch --detach <base-commit> && cargo build --release --locked && cp target/release/f2j /tmp/fig-old
git switch - && cargo build --release --locked && cp target/release/f2j /tmp/fig-new

python3 -I tools/make_test_fig.py /tmp/t.fig --zip
for input in /tmp/t.fig /path/to/real-design.fig; do
  for flags in "" "--legacy" "--keep-internal" "--legacy --keep-internal"; do
    for v in old new; do
      rm -rf /tmp/cmp-$v && mkdir -p /tmp/cmp-$v
      /tmp/fig-$v "$input" /tmp/cmp-$v/out $flags --raw
    done
    diff -r /tmp/cmp-old /tmp/cmp-new && echo "same: $input $flags"
  done
done
```

The loop above is for ZIP inputs (output directory); in bare mode the directory argument is ignored and
the JSON goes to stdout. For a bare `canvas.fig`, compare stdout instead:
`/tmp/fig-old canvas.fig $flags | cmp - <(/tmp/fig-new canvas.fig $flags)`. Also compare stderr and
exit codes when touching `src/cli.rs` (error messages are part of the CLI's behaviour).

Use at least one real Figma file: the synthetic file has no variables, styles, components, text or
auto-layout. If the output changes on purpose, describe the change in `CHANGELOG.md` under
`[Unreleased]` and update [OUTPUT_FORMAT.md](OUTPUT_FORMAT.md) / [PIPELINE.md](PIPELINE.md).

## CI

`.github/workflows/ci.yml` (the only workflow) runs on pushes to `main`, on pull requests and on manual
dispatch. One `check` job runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked` and `web/build.sh`. On `main` it then deploys `web/www` to GitHub Pages
(`deploy` job).

Run the same commands locally before pushing (see `CLAUDE.md`), plus
`RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` and, for MSRV changes,
`cargo +1.88 test --workspace --locked`. `Cargo.toml` also sets
`unsafe_code = "forbid"` and Clippy `all = "deny"`.

## Release process

1. Make sure `CHANGELOG.md` `[Unreleased]` lists everything since the last release.
2. Bump `version` in `Cargo.toml` (Semantic Versioning) and run `cargo build --locked` so `Cargo.lock`
   picks up the new version of the package itself.
3. In `CHANGELOG.md`, rename `[Unreleased]` to `[X.Y.Z]`, add a new empty `[Unreleased]` above it, and
   update the compare/tag links at the bottom (`[Unreleased]: .../compare/vX.Y.Z...HEAD`,
   `[X.Y.Z]: .../releases/tag/vX.Y.Z`).
4. Run the CI commands and `cargo publish --dry-run --locked -p figma2json`; commit (author
   `mtalhazulf <talhazulf4163@gmail.com>`, no attribution trailers) and push to `main`.
5. Publish: `CARGO_REGISTRY_TOKEN="$CRATE_TOKEN" cargo publish --locked -p figma2json` (the token is a
   crates.io API token; the cloud environment exposes it as `CRATE_TOKEN`). Only `figma2json` is published;
   `figma2json-web` has `publish = false`. The package contents are set by `include` in `Cargo.toml`.
6. Tag `vX.Y.Z` on that commit and push the tag (or create a GitHub release with that tag and the
   changelog section as notes).

A published version is permanent: it can be yanked (`cargo yank --version X.Y.Z`) but never changed or
deleted, so every change after a release needs a new version.

Current release: 0.0.1 (<https://crates.io/crates/figma2json>, docs at <https://docs.rs/figma2json>).

## Python tools

All scripts are in `tools/`, use only the standard library, and read the JSON that figma2json writes.
Check them with `python3 -I -m py_compile tools/*.py`.

### `make_test_fig.py`

`python3 -I tools/make_test_fig.py OUT.fig [--zip]`. Writes the synthetic `.fig` described above.

### `build_figma_cache.py`

Splits a `canvas.json` into an agent-friendly folder of small files.

```bash
python3 tools/build_figma_cache.py canvas.json OUT_DIR --kind design-system [--library-name KEY=NAME ...]
python3 tools/build_figma_cache.py canvas.json OUT_DIR --kind screens [--page UI] [--library-name KEY=NAME ...]
```

- Input must be default-mode output (it exits if `variables` is missing, that is if it was converted with
  `--legacy`). Convert with `--keep-internal` so library component copies can be exported.
- `OUT_DIR` must be new or empty (it exits otherwise).
- `--library-name KEY=NAME` (repeatable) gives a library a readable name; `KEY` is the first 11
  characters of a `sourceLibraryKey`, or `local` for the file itself.
- Before building it resolves instance overrides (`resolve_overrides`: adds `targetId` to each
  `symbolOverrides[]` entry, or `otherVariant: true` for overrides that belong to other variants) and
  collects component property names.
- Node files are pruned (`prune`): cached and derived data such as `derivedSymbolData`,
  `derivedTextData`, versions and `editInfo` are dropped (see `DROP`), and variable bindings are
  simplified (`"variable": "Text/primary"` on paints, `"boundVariables": {"STACK_SPACING": "Space/8"}`).
- `--kind design-system` writes `TOKENS.json` / `TOKENS.md` (collections, styles, libraries),
  `sections/<page>__<section>.json`, `components/<section>__<name>.json` (component sets over 150 KB are
  also split per variant into a directory), `components.json`, `COMPONENTS.md`, `index.json` (sections),
  `icons/*.svg` + `icons/index.json` (vector icons; `icons/graphics/` for graphics on a background), and
  `library-components/` + `LIBRARY-COMPONENTS.md` for library components the design system uses.
- `--kind screens` takes the page `--page` (default `UI`; it exits listing the pages if not found),
  treats each Section on it as a flow, and writes `flows/<flow>/<screen>__<id>.json`, `index.json`
  (screens, connector/prototype links, designer notes), `FLOWS.md`, `components/<id>.json` +
  `components/index.json` + `COMPONENTS.md` for every component the screens use, `icons/`, and
  `TOKENS.json` / `TOKENS.md` limited to what the page uses.
- Prints a JSON summary of counts on stdout.

### `verify_cache.py`

Checks a cache folder. Run it from the cache root:

```bash
cd CACHE_ROOT && python3 /path/to/tools/verify_cache.py
```

It expects the builder's output in `design-system/` and/or `admin/` subfolders (the `screens` kind built
into `admin/`). It checks that every `*.json` parses; that files referenced from `*.md` files and indexes
exist; that every `componentId` / `overriddenSymbolID` / `symbolIdValue` points at an exported component
(ids listed as `notFound` are warnings); that every `variableId` / `variable` name and every style name
exists in `TOKENS.json`; that node files contain no internal leftovers (`__enum__`, `colorVar`,
`parameterConsumptionMap`, `variableConsumptionMap`, `derivedSymbolData`, `guid`, `styleIdFor`, empty
`assetRef`); and that exported SVG icons parse as XML. It prints counts, warnings and up to 50 problems,
and exits 1 if there is any problem.

### `audit_admin.py`

```bash
python3 tools/audit_admin.py canvas.json CACHE_ROOT [TOOLS_DIR]
```

Audits the page named `UI` of a `canvas.json` (and every component its screens use) for things an
implementer would trip over: node types, paint types, colors not bound to a variable or style,
gradients, images, effects, fonts, text without a text style, mixed-style text, auto-layout vs absolute
frames, CSS-grid frames, rotation, fixed/sticky elements, layout grids, masks, blend modes, instances
whose component was not exported, override properties, prototype interactions, explicit variable
modes, and components not found in the design system. It needs `CACHE_ROOT/admin/` (index,
`components/index.json`, `icons/index.json`, `TOKENS.json`) and `CACHE_ROOT/design-system/components.json`.
`TOOLS_DIR` (default `.`) is where `build_figma_cache.py` is imported from. It prints a report and writes
the full result to `/tmp/audit.json`. The page name and folder layout are fixed in the script.
