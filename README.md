# figma2json

> Fork of [kreako/fig2json](https://github.com/kreako/fig2json), renamed `figma2json`. It keeps the design
> context needed to implement a design (node ids, token and style names, auto-layout alignment and sizing,
> library components) and fixes several bugs; see `CHANGELOG.md`.

Convert Figma `.fig` files to clean, optimized JSON for AI-powered design implementation.

## Overview

`figma2json` extracts and transforms Figma design files into structured JSON, making it easy for LLMs to understand and implement your designs. The tool removes Figma-specific metadata and applies intelligent transformations to produce JSON optimized for HTML/CSS conversion.

## How to Use

### 1. Save Your Figma Design Locally

In Figma, go to the top-left menu:

- Click **File** → **Save local copy...**
- Save the `.fig` file to your computer

### 2. Convert to JSON

Run `figma2json` on your saved file:

```bash
figma2json your-design.fig output-directory
```

This will:

- Extract all contents to `output-directory/`
- Convert the design to `output-directory/canvas.json`

### 3. Implement with AI

Ask your LLM to implement the design:

```
Implement the design found in output-directory/canvas.json
```

The clean JSON structure makes it easier for AI to understand your design and hopefully generate accurate HTML/CSS.

**Tip:** For easier LLM consumption, consider using [jq](https://github.com/jqlang/jq) to extract specific parts of the design or to further process the JSON before sending it to your LLM.

## Installation

```bash
cargo install --path .   # from this directory; figma2json is not published on crates.io
```

Or build from source:

```bash
cargo build --release
# Binary will be at target/release/figma2json
```

## Usage

### Basic Usage

A `.fig` saved from Figma (**File → Save local copy**) is a ZIP archive. Convert it into a directory:

```bash
figma2json design.fig output-dir
# output-dir/canvas.json, output-dir/images/, and the rest of the archive
```

Add `--keep-internal` to keep local copies of the library components the file uses:

```bash
figma2json design.fig output-dir --keep-internal
```

A bare `canvas.fig` (for example the one extracted into `output-dir/`) can be converted to stdout or a file:

```bash
figma2json output-dir/canvas.fig
figma2json output-dir/canvas.fig -o output.json
```

### Command Line Flags

| Flag                  | Description                                                                                                                      |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `-o, --output <FILE>` | Output JSON file path (default: stdout). Only for a bare `canvas.fig`; a ZIP `.fig` writes `<output-dir>/canvas.json`.          |
| `--compact`           | Output compact JSON instead of pretty-printed (default is indented).                                                             |
| `-v, --verbose`       | Enable verbose output for debugging.                                                                                             |
| `--raw`               | Generate both transformed `.json` and raw `.raw.json` files. The raw version contains the original data without transformations. |
| `--keep-internal`     | Keep internal-only nodes (`"internalOnly": true`), including the hidden page with local copies of the library components, variables and styles the file uses. Needed to see inside library components. |
| `--legacy`            | Produce the pre-0.4 output: no node ids/types, no variable or style names, no root `variables`/`styles`, and auto-layout alignment/sizing, constraints, scroll behavior, layout grids and override paths removed. |

### Examples

**Extract and convert with verbose output:**

```bash
figma2json design.fig output-dir --verbose
```

**Compact JSON output:**

```bash
figma2json design.fig output-dir --compact
```

**Generate both transformed and raw JSON:**

```bash
figma2json design.fig output-dir --raw
# Creates: output-dir/canvas.json and output-dir/canvas.raw.json
```

**Pipe to other tools:**

```bash
figma2json output-dir/canvas.fig | jq '.variables[].name'
```

## Transformations

The tool applies intelligent transformations to clean up the JSON:

- **Removes default values**: `blendMode: "NORMAL"`, default letter spacing, line height
- **Removes Figma-specific metadata**: edit history, phases, plugin data, image thumbnails, text glyphs
- **Removes redundant fields**: Derived layout sizes, empty font properties
- **Filters internal nodes**: Removes `internalOnly` elements (after reading the variable and style definitions they hold)
- **Preserves geometry**: Keeps SVG paths for icons and images
- **Optimizes structure**: Only essential fields for HTML/CSS rendering

It keeps what implementing a design needs:

| In the output | What it is |
| --- | --- |
| `id` on every node | Figma node id (`"sessionID:localID"`, the same as `?node-id=` in Figma URLs) |
| `type` on every node | `FRAME`, `TEXT`, `INSTANCE`, `COMPONENT`, `COMPONENT_SET`, `RECTANGLE`, `VECTOR`, ... (also `type` on paints and effects) |
| `componentId`, `componentName`, `componentSetName` on instances | Which component (and variant) an instance uses |
| `variable`, `variableCollection`, `variableId` next to every `"dataType": "ALIAS"` | The variable (design token) a color, number or string is bound to; `variableId` matches `id` in the root `variables` |
| `fillStyle`, `strokeStyle`, `textStyle`, `effectStyle`, `gridStyle` | The name of the shared style a node uses |
| `stackPrimaryAlignItems`, `stackCounterAlignItems` | Auto-layout justify / align |
| `stackPrimarySizing`, `stackCounterSizing` | Auto-layout hug / fixed sizing |
| `stackChildPrimaryGrow`, `stackChildAlignSelf` | Fill container / stretch on children |
| `stackHorizontalPadding`, `stackPaddingRight`, `stackVerticalPadding`, `stackPaddingBottom` | Left, right, top, bottom padding. Figma omits a side that is 0, so missing = 0 |
| `stackPrimarySizing`, `stackCounterSizing` (defaults) | Figma omits defaults: missing `stackPrimarySizing` = hug, missing `stackCounterSizing` = fixed; missing alignment = `MIN` (start) |
| `horizontalConstraint`, `verticalConstraint`, `scrollBehavior`, `layoutGrids` | Constraints, fixed/sticky scrolling, layout grids |
| `transitionNodeID`, `guidPath`, `overriddenSymbolID`, `symbolIdValue` | Prototype targets, which child an override applies to, and swapped-in components, as node ids |
| `textData.characterStyleIDs` + `styleOverrideTable[].styleID` | Mixed-style text: each character's style id points at an override entry (0 = the node's own style) |
| root `variables` | Every variable collection with its modes, and each variable's value per mode (`{"alias": ...}` for references), `scopes` and `uses` |
| root `styles` | Every shared style (fill, text, effect, grid) with its values and `uses` |

Note that instance contents are not expanded: an `INSTANCE` node has no `children`. Its component is
`componentId`, and text or property changes are in `symbolData.symbolOverrides`, each targeting a child of
the component by `guidPath`. When the component comes from a library, its definition is a local copy on a
hidden internal page: add `--keep-internal` to keep that page, then look up `componentId` there.

Use `--legacy` for the previous, smaller output, and `--raw` to also generate the untransformed JSON.

## Output Structure

After extraction, you'll find:

```
output-directory/
├── canvas.json          # Main design file (transformed)
├── canvas.raw.json      # Raw untransformed data (if --raw flag used)
└── [other extracted files]
```

The `canvas.json` file contains the complete design tree with all layers, styles, and properties needed for implementation:

```json
{
  "document": { "type": "DOCUMENT", "children": [ /* pages, frames, ... */ ] },
  "variables": [
    {
      "name": "2. Colors",
      "modes": [{ "id": "1:0", "name": "Light" }, { "id": "1:1", "name": "Dark" }],
      "uses": 4924,
      "variables": [
        {
          "name": "text/primary",
          "type": "COLOR",
          "valuesByMode": { "Light": { "alias": "gray/900", "aliasCollection": "1. Primitives" }, "Dark": "#ffffff" },
          "uses": 866
        }
      ]
    }
  ],
  "styles": [{ "name": "Body/M-Regular", "styleType": "TEXT", "fontSize": 14, "lineHeight": "20px", "uses": 120 }]
}
```

### Memory

Large files are decoded straight to JSON and blob data that is discarded anyway (text glyphs,
path geometry) is dropped before it is expanded. A 4.7 MB `canvas.fig` (122 MB `.fig` with images)
converts with about 4 GB peak memory. `--raw` keeps everything and needs several times more.

## FAQ

### What inspired this project?

This project is a fork of [kreako/fig2json](https://github.com/kreako/fig2json), which was inspired by [Evan Wallace's Figma File Parser](https://madebyevan.com/figma/fig-file-parser/).

### How was this built?

This project was mostly vibe-coded with Claude Code.

### Which libraries do the variables come from?

Each collection and style from a library has a `sourceLibraryKey`. Local ones have none. Collections with the
same `sourceLibraryKey` come from the same library file.

### What's the difference between transformed and raw JSON?

- **Transformed JSON** (default): Clean, optimized structure with Figma metadata removed and defaults stripped. Best for AI consumption and HTML/CSS generation.
- **Raw JSON** (`--raw` flag): Original decoded data with all Figma-specific fields intact. Useful for debugging or advanced use cases.

### Does this work with Figma plugins?

No, this tool works with locally saved `.fig` files. You need to use Figma's "Save local copy" feature to export the file first.

## License

MIT
