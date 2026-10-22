# figma2json

Turn Figma `.fig` files into clean, compact JSON that an LLM (or your own code) can use to implement the design.

`figma2json` reads the binary file Figma saves locally, decodes it and strips everything that doesn't help
build the UI: edit history, glyph outlines, plugin data, default values. What's left is the layer tree with
the information needed to write HTML/CSS: node ids, layout, auto-layout settings, colors, text styles, and
the names of the variables (design tokens), styles and components each layer uses.

## Quick start

1. In Figma, open the file and choose **File → Save local copy…**
2. Convert it:

   ```bash
   figma2json design.fig out
   ```

3. Point your LLM at the result:

   ```text
   Implement the design in out/canvas.json. Images are in out/images/.
   ```

## Installation

You need a Rust toolchain, version 1.88 or newer ([rustup.rs](https://rustup.rs)).

```bash
git clone https://github.com/obsidian-code-io/figma2json
cd figma2json
cargo install --path .
```

To build without installing, run `cargo build --release`; the binary is `target/release/figma2json`.

## Usage

```text
figma2json <INPUT> [OUTPUT_DIR] [OPTIONS]
```

A `.fig` saved from Figma is a ZIP archive. Give it an output directory: the archive is extracted there and
every `.fig` inside is converted next to itself.

```bash
figma2json design.fig out
# out/canvas.json, out/images/, ...
```

A bare `canvas.fig` (the file inside the archive) can be converted to stdout or to a file:

```bash
figma2json out/canvas.fig                 # JSON to stdout
figma2json out/canvas.fig -o design.json  # JSON to a file
```

### Options

| Option                | What it does                                                                                       |
| --------------------- | -------------------------------------------------------------------------------------------------- |
| `-o, --output <FILE>` | Write JSON to a file instead of stdout. Bare `canvas.fig` only.                                    |
| `--compact`           | Compact JSON instead of indented.                                                                  |
| `--raw`               | Also write `*.raw.json`: the decoded file with no transformations applied.                         |
| `--keep-internal`     | Keep internal-only nodes, including the hidden page with local copies of library components.       |
| `--legacy`            | Smaller output without design context: no ids, types, token or style names, and no auto-layout alignment/sizing.  |
| `-v, --verbose`       | Log each step to stderr.                                                                           |
| `-h, --help`          | Show help.                                                                                         |
| `-V, --version`       | Show version.                                                                                      |

### Examples

```bash
# Everything, including library component definitions, as compact JSON
figma2json design.fig out --keep-internal --compact

# List the design tokens
figma2json out/canvas.fig | jq '.variables[].variables[].name'

# Find every instance of a component
figma2json out/canvas.fig | jq '.. | objects | select(.componentName? == "Button")'
```

## Output

```json
{
  "document": {
    "id": "0:0",
    "type": "DOCUMENT",
    "children": [ { "id": "0:1", "type": "CANVAS", "name": "Page 1", "children": [ ... ] } ]
  },
  "variables": [
    {
      "name": "Colors",
      "modes": [ { "id": "1:0", "name": "Light" }, { "id": "1:1", "name": "Dark" } ],
      "variables": [
        {
          "name": "text/primary",
          "type": "COLOR",
          "valuesByMode": { "Light": { "alias": "gray/900" }, "Dark": "#ffffff" },
          "uses": 866
        }
      ]
    }
  ],
  "styles": [ { "name": "Body/M", "styleType": "TEXT", "fontSize": 14, "lineHeight": "20px", "uses": 120 } ]
}
```

### What each node keeps

| Field                                                       | Meaning                                                                                  |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `id`                                                        | Figma node id, the same value as `?node-id=` in Figma URLs                               |
| `type`                                                      | `FRAME`, `TEXT`, `INSTANCE`, `COMPONENT`, `COMPONENT_SET`, `RECTANGLE`, `VECTOR`, ...    |
| `transform`                                                 | Position, rotation and scale (`x`, `y`, `rotation`, `scaleX`, `scaleY`, `skewX`)         |
| `fillPaints`, `strokePaints`                                | Visible paints, colors as CSS hex (`#rrggbb` / `#rrggbbaa`)                              |
| `componentId`, `componentName`, `componentSetName`          | The component (and variant) an instance uses                                             |
| `variable`, `variableCollection`, `variableId`              | The design token a value is bound to, next to every `"dataType": "ALIAS"`                |
| `fillStyle`, `strokeStyle`, `textStyle`, `effectStyle`, `gridStyle` | Name of the shared style a node uses                                             |
| `stackPrimaryAlignItems`, `stackCounterAlignItems`          | Auto-layout justify / align (missing = `MIN`)                                            |
| `stackPrimarySizing`, `stackCounterSizing`                  | Hug or fixed (missing primary = hug, missing counter = fixed)                            |
| `stackChildPrimaryGrow`, `stackChildAlignSelf`              | Fill container / stretch on auto-layout children                                         |
| `stackHorizontalPadding`, `stackPaddingRight`, `stackVerticalPadding`, `stackPaddingBottom` | Left, right, top and bottom padding (missing = 0)        |
| `horizontalConstraint`, `verticalConstraint`, `scrollBehavior`, `layoutGrids` | Constraints, fixed/sticky scrolling, layout grids                      |
| `image.filename`                                            | Path of the image file, e.g. `images/6049a17a.png`                                       |

Instances are not expanded: an `INSTANCE` has no `children`. Overrides are in `symbolData.symbolOverrides`,
each pointing at a child of the component through `guidPath`. Components from a library live on a hidden
internal page; run with `--keep-internal` and look up the instance's `componentId` there.

## Library

The converter is also a Rust library:

```rust
use figma2json::{convert_with_options, ConvertOptions};
use std::path::Path;

let bytes = std::fs::read("out/canvas.fig")?;
let json = convert_with_options(&bytes, Some(Path::new("out")), &ConvertOptions::default())?;
println!("{}", serde_json::to_string_pretty(&json)?);
```

`convert_raw` returns the decoded file without transformations. The individual passes are public in
`figma2json::schema`.

## How it works

1. **Container.** A `.fig` is a ZIP holding `canvas.fig`, images and metadata. `canvas.fig` starts with a
   `fig-kiwi` (or `fig-jam.`) header followed by length-prefixed chunks.
2. **Decompression.** Each chunk is raw DEFLATE or Zstandard.
3. **Decoding.** The first chunk is a Kiwi schema; the second is the
   document, decoded with that schema straight into JSON.
4. **Tree.** The flat list of node changes is assembled into a tree using each node's parent index.
5. **Blobs.** Binary blobs (vector paths, networks) are parsed and inlined where nodes reference them.
6. **Passes.** A declarative pipeline (`src/pipeline.rs`) runs the transformation passes in
   `src/schema/transformations/`: CSS colors and transforms, enum simplification, design-context
   annotations, and removal of internal or default-valued fields.

Large files are handled with care: glyph and geometry data is dropped before blobs are expanded and output is
streamed to disk, so a 120 MB `.fig` converts in about 4 GB of memory. `--raw` keeps everything and needs more.

## Project layout

```text
src/
├── main.rs              CLI
├── lib.rs               library entry point
├── pipeline.rs          decode + transformation pipeline
├── error.rs, types.rs   error and shared types
├── parser/              ZIP, header, chunks, decompression
├── schema/              Kiwi decoder, tree builder, transformations/
└── blobs/               blob parsing and substitution
tools/                   Python helpers that split canvas.json into per-screen files
```

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo doc --no-deps
```

CI runs the same checks on every push and pull request, plus the test suite on the minimum supported Rust
version (1.88).

## License

MIT. See [LICENSE](LICENSE).
