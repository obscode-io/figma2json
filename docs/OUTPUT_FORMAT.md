# Output format

What `convert_with_options` (and therefore the CLI) produces. Field lists come from
`src/pipeline.rs`, `src/schema/transformations/design_context.rs` and the removal passes listed in
[PIPELINE.md](PIPELINE.md). Object keys are always sorted alphabetically, because `serde_json` is used
without `preserve_order`.

Most node fields are Figma's own internal field names, passed through after the cleanup passes.
figma2json does not define a fixed node schema: a field is kept unless a pass removes or rewrites it.

## Root

| Mode | Root keys |
| --- | --- |
| default | `document`, `variables`, `styles` |
| `--legacy` | `document` |
| `--raw` (`*.raw.json`, `convert_raw`) | `version`, `fileType`, `document`, `blobs` |

`version` and `fileType` (`"figma"` / `"figjam"`) and `blobs` exist in the assembled root of every mode,
but CLEAN_OUTPUT removes them (`remove_root_metadata`, `remove_root_blobs`) except in raw output.

## Example (default mode)

This is the real output of the synthetic file from `tools/make_test_fig.py --zip` (ZIP mode, so the image
was found and renamed), shortened with `...`:

```json
{
  "document": {
    "children": [
      {
        "children": [
          {
            "children": [
              {
                "commands": ["M", 0.0, 0.0, "L", 10.0, 0.0, "L", 10.0, 10.0, "Z"],
                "id": "1:3",
                "name": "Vector",
                "type": "VECTOR",
                "vectorNetwork": {
                  "regions": [{ "loops": [{ "segments": [0] }], "styleID": 1, "windingRule": "NONZERO" }],
                  "segments": [{ "end": { "dx": 0.0, "dy": 0.0, "vertex": 1 }, "start": { ... }, "styleID": 0 }],
                  "vertices": [{ "styleID": 0, "x": 0.0, "y": 0.0 }, { "styleID": 0, "x": 5.0, "y": 5.0 }]
                }
              }
            ],
            "fillPaints": [
              { "color": "#ff8000" },
              {
                "image": { "filename": "images/a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3.png", "name": "pixel" },
                "type": "IMAGE"
              }
            ],
            "id": "1:2",
            "name": "Frame",
            "type": "FRAME"
          }
        ],
        "id": "0:1",
        "name": "Page 1",
        "type": "CANVAS"
      }
    ],
    "id": "0:0",
    "name": "Document",
    "type": "DOCUMENT"
  },
  "styles": [],
  "variables": []
}
```

Every node in the input had `visible: true` and `opacity: 1.0`, and every paint had `opacity: 1.0` and
`visible: true`: all of them were removed as defaults. The solid paint had no `type` in the input, so none
appears in the output.

## The node tree

`document` is the node `0:0` (type `DOCUMENT`). Children are in `children`, ordered by Figma's
`parentIndex.position` (`src/schema/tree.rs`). Pages are `CANVAS` nodes. A node without children has
no `children` key. Instances are not expanded: an `INSTANCE` has no `children`; its overrides are in
`symbolData.symbolOverrides`.

### Fields added or rewritten

| Field | Where | Source |
| --- | --- | --- |
| `id` | every node (default mode) | `"sessionID:localID"`, the same value as `?node-id=` in Figma URLs (`design_context.rs`) |
| `type` | every node (default mode; removed in legacy) | Public names: `SYMBOL` -> `COMPONENT`, `FRAME`/`SYMBOL` with `isStateGroup` -> `COMPONENT_SET`, `ROUNDED_RECTANGLE` -> `RECTANGLE`; other types unchanged |
| `componentId`, `componentName`, `componentSetName` | `INSTANCE` nodes | From `symbolData.symbolID` (local guid or library `assetRef.key` resolved to the local copy). `componentSetName` only when the component's parent is a state group. `componentId` is set whenever `symbolID` resolves to an id (a local guid, or a library key whose local copy is in the file); the names only when that node is in the file. |
| `variable`, `variableCollection`, `variableId` | next to every `"dataType": "ALIAS"` whose alias resolves to a `VARIABLE` node | Variable name, collection name, variable id (matches `id` in root `variables`) |
| `fillStyle`, `strokeStyle`, `textStyle`, `effectStyle`, `gridStyle` | nodes with `styleIdForFill` / `styleIdForStrokeFill` / `styleIdForText` / `styleIdForEffect` / `styleIdForGrid` | Name of the style node. The `styleIdFor*` fields themselves are removed. |
| `"s:l"` strings | any field or array element that was `{sessionID, localID}` or a single-key `{guid: {...}}` / resolvable `{assetRef: {key}}` wrapper (component ids, prototype targets, property ids, aliases) | `design_context.rs`. The "no id" sentinel `4294967295:4294967295` becomes `null` in object fields (but see PIPELINE.md known issue 2 for arrays). |
| `guidPath` | symbol overrides (default mode) | `{guids: [...]}` -> `"s:l/s:l"`, the path to the overridden child |
| `transform` | nodes | `{x, y}` plus `rotation`, `scaleX`, `scaleY`, `skewX` when not default (`matrix_to_css.rs`) |
| colors | any `{r, g, b[, a]}` field | `#rrggbb`, or `#rrggbbaa` when alpha is not 1 (`color_to_css.rs`) |
| enums | everywhere | `{"__enum__": "X", "value": "V"}` -> `"V"` |
| `letterSpacing`, `lineHeight` | text nodes, text styles | `"12px"`, `"150%"` (`text_properties_simplification.rs`); defaults (0%, 100%) removed on nodes |
| `commands`, `vectorNetwork` | vector nodes | Parsed from `commandsBlob` / `vectorNetworkBlob` (`src/blobs/parser.rs`) |
| `image.filename` | `image` objects (paints) | `"images/<hex hash>"` plus `.png` / `.jpg` / `.gif` / `.webp` / `.svg` when the file was found in `base_dir` |

### Fields kept that the default mode relies on

Auto-layout alignment and sizing (`stackPrimaryAlignItems`, `stackCounterAlignItems`,
`stackPrimarySizing`, `stackCounterSizing`), child settings (`stackChildPrimaryGrow`,
`stackChildAlignSelf`), every padding value Figma stores (`stackHorizontalPadding`,
`stackPaddingRight`, `stackVerticalPadding`, `stackPaddingBottom`; a missing side is 0), constraints
(`horizontalConstraint`, `verticalConstraint`), `scrollBehavior`, `resizeToFit`, `guides`,
`layoutGrids`, `guidPath`, `styleID` / `windingRule` (text style runs and vector regions). All of these
are removed in legacy mode.

### Fields always removed

See the pass table in [PIPELINE.md](PIPELINE.md). In short: `guid`, `parentIndex`, `editInfo`,
`phase`, `fillGeometry` / `strokeGeometry` (except on `icon/` / `arrows/` nodes and nodes with image
export settings in their overrides), glyphs and text layout caches, text metadata, stroke
weight/align/join, border weights, default `opacity` / `visible` / `rotation` / `blendMode` /
`uniformScaleFactor`, image metadata, `internalOnly` nodes (unless `--keep-internal`), root
`version` / `fileType` / `blobs`, `documentColorProfile`, `userFacingVersion`, `styleIdFor*`,
`exportSettings`, `pluginData`, `rectangleCornerRadiiIndependent`, `detachedSymbolId`,
`cornerSmoothing`, per-corner radii when `cornerRadius` exists, invisible paints, empty paint arrays,
objects with only `visible`, and empty objects.

## `variables`

An array of variable collections (`VARIABLE_SET` nodes), in tree order, built by
`Definitions::collect`, `nest_variables` and `record_uses` in `design_context.rs`:

```json
{
  "id": "1:2",
  "name": "Colors",
  "modes": [{ "id": "1:10", "name": "Light" }, { "id": "1:11", "name": "Dark" }],
  "uses": 1,
  "variables": [
    {
      "id": "1:4",
      "name": "text/primary",
      "type": "COLOR",
      "valuesByMode": {
        "Dark": "#ffffff",
        "Light": { "alias": "gray/900", "aliasCollection": "Colors" }
      },
      "uses": 1
    }
  ]
}
```

(Produced by running `add_design_context` on a synthetic document; `key` and `scopes` were `null` there
and are omitted above.)

- Collection fields: `id`, `name`, `key`, `sourceLibraryKey` (library collections), `modes`
  (`{id, name}`), `variables`, `uses` (sum of its variables' uses). `null` fields are dropped.
- Variable fields: `id`, `name`, `key`, `type` (`variableResolvedType`), `scopes` (`variableScopes`),
  `description` (omitted when empty), `valuesByMode`, `uses`. `null` fields are not dropped here.
- `valuesByMode` is keyed by mode name (mode id when the name is unknown). A value object with a single
  key is unwrapped (`{"colorValue": "#fff"}` -> `"#fff"`); an alias becomes `{"alias": name}` plus
  `aliasCollection` when known, or `{"alias": null}` when it cannot be resolved.
- Variables whose collection is not in the file are collected in a final collection named
  `(unknown collection)` (its `id` is `null` and therefore dropped).
- `uses` counts references from nodes that are not on an `internalOnly` page.

Variables, collections and library copies live on Figma's internal-only page, so they are found even
though that page is removed from `document` (unless `--keep-internal`).

## `styles`

An array of style definitions: every node with a `styleType`, in tree order:

```json
{ "id": "1:5", "name": "Body/M", "styleType": "TEXT", "fontSize": 14.0, "lineHeight": "20px", "uses": 1 }
```

Fields: `id`, `name`, `key`, `styleType`, `sourceLibraryKey`, `description` (only when non-empty),
`uses`, plus by `styleType`: `FILL` -> `fillPaints`; `TEXT` -> `fontName`, `fontSize`, `lineHeight`,
`letterSpacing`, `textCase`, `textDecoration`, `paragraphSpacing`; `EFFECT` -> `effects`; `GRID` ->
`layoutGrids`. `null` fields are dropped. Style fields are copied before the document cleanup passes
run, so they can contain defaults and raw references (PIPELINE.md known issue 6).

## `--legacy`

`ConvertOptions::legacy`. Design context is not computed, so there is no root `variables` / `styles`,
no `id`, no `componentId` / `componentName` / `componentSetName`, no `variable*` names and no
`*Style` names. Additionally removed: `windingRule` and `styleID` on every object except icon/image nodes themselves (vector network vertices, segments and regions included),
`guidPath`, `horizontalConstraint`, `verticalConstraint`, `scrollBehavior`, `resizeToFit`, `guides`,
`layoutGrids`, `stackChildAlignSelf`, `stackChildPrimaryGrow`, `stackCounterSizing`,
`stackPrimarySizing`, `stackCounterAlignItems`, `stackPrimaryAlignItems`, every `type` field (nodes,
paints, effects), right/bottom padding equal to left/top, and `{localID, sessionID}` `symbolID` objects.
Guid references are not converted to `"s:l"` strings.

The legacy output of the synthetic file:

```json
{"document":{"children":[{"children":[{"children":[{"commands":["M",0.0,0.0,"L",10.0,0.0,"L",10.0,10.0,"Z"],"name":"Vector","vectorNetwork":{"regions":[{"loops":[{"segments":[0]}]}],"segments":[{"end":{"dx":0.0,"dy":0.0,"vertex":1},"start":{"dx":0.0,"dy":0.0,"vertex":0}}],"vertices":[{"x":0.0,"y":0.0},{"x":5.0,"y":5.0}]}}],"fillPaints":[{"color":"#ff8000"}],"name":"Frame"}],"name":"Page 1"}],"name":"Document"}}
```

## `--keep-internal`

`ConvertOptions::keep_internal` skips `remove_internal_only_nodes`. The output then keeps the hidden
internal-only page(s) with Figma's local copies of library components, variables and styles, and
`internalOnly: true` stays on those nodes. This is the only place a library component's contents
appear in the file; look up an instance's `componentId` there. It combines with `--legacy`.

## `--raw`

`convert_raw`: the decoded file without any transformation pass:

- root: `version` (u32 from the header), `fileType` (`"figma"` / `"figjam"`), `document`, `blobs`;
- `document`: the tree from `build_tree` (`parentIndex` removed, `children` added), with every Figma
  field, `guid` objects, enum objects `{"__enum__", "value"}`, raw transform matrices and `{r,g,b,a}`
  colors;
- blob references substituted (`commandsBlob` -> `commands`, `vectorNetworkBlob` -> `vectorNetwork`),
  including glyphs and geometry;
- `blobs`: every blob as `{"bytes": "<base64>"}`;
- image references still as `hash` byte arrays (no file renaming).

## Image files

Images are not inside `canvas.fig`; Figma's ZIP stores them as `images/<hex hash>` next to it. The CLI
extracts them in ZIP mode, and `transform_image_hashes` renames each referenced file to
`images/<hex hash>.<ext>` and writes that path into `image.filename` (relative to the JSON file).
The extension is detected from the file header: PNG, JPEG (`.jpg`), GIF, WebP, SVG. Unreferenced
images keep their extension-less names. `imageThumbnail` references are renamed the same way, but the
`imageThumbnail` field itself is removed by `remove_image_metadata_fields`.
