# Changelog

## 0.4.0 (figma2json)

Renamed from `fig2json` (fork of https://github.com/kreako/fig2json): the package, binary and library are now
`figma2json` (`use figma2json::...`). Versions up to 0.3.0 are the upstream `fig2json`.

### Added
- Node `id` (Figma node id) and `type` on every node; public type names (`COMPONENT`, `COMPONENT_SET` for state-group frames, `RECTANGLE`).
- Instances name their component: `componentId`, `componentName`, `componentSetName`.
- Variable bindings name their variable: `variable`, `variableCollection` and `variableId` next to every `ALIAS`.
- Shared style names on nodes: `fillStyle`, `strokeStyle`, `textStyle`, `effectStyle`, `gridStyle`.
- Root `variables` (collections, modes, per-mode values, scopes, usage counts) and `styles` (values, usage counts).
- `--keep-internal` flag (`ConvertOptions::keep_internal`): keep the hidden page with local copies of the library
  components the file uses, so instances of library components can be resolved.
- `--legacy` flag (and `ConvertOptions` / `convert_with_options` in the library) for the previous output.

### Kept (previously removed)
- Auto-layout alignment and sizing (`stackPrimaryAlignItems`, `stackCounterAlignItems`, `stackPrimarySizing`,
  `stackCounterSizing`, `stackChildPrimaryGrow`, `stackChildAlignSelf`), constraints, `scrollBehavior`,
  `layoutGrids`, override `guidPath`s. Internal `{sessionID, localID}` references become `"s:l"` strings.

### Fixed
- Right/bottom auto-layout padding was dropped whenever left/top padding existed, even when different
  (asymmetric padding came out symmetric). The default output now keeps every padding value Figma stores (Figma
  omits sides that are 0, so a missing side is 0). `--legacy` removes right/bottom only when equal to left/top.
- Instance-swap and slot property values (`symbolIdValue`, `slotContentIdValue`) and other `{guid: ...}` /
  `{assetRef: ...}` references were emptied by guid removal; they are now `"s:l"` node ids (library references
  resolve to the file's local copy when present).
- `styleID` and `windingRule` were removed everywhere by the geometry pass, which broke mixed-style text
  (`characterStyleIDs` pointed at `styleOverrideTable` entries with no `styleID`). The default output only
  removes the geometry itself; `--legacy` keeps the old behaviour.
- `styleIdForEffect` and `styleIdForGrid` references are now removed like the other `styleIdFor*` fields
  (their names are in `effectStyle` / `gridStyle`).
- Enum values inside arrays (e.g. `variableScopes`, `toggledOffOTFeatures`) were left as `{"__enum__": ...}` objects.
- Out-of-memory on large files: Kiwi data is decoded straight to JSON (blob bytes as base64, not one JSON number
  per byte), node and blob data are moved instead of cloned, glyphs and geometry are dropped before blob expansion,
  and output is streamed to disk. Example: a file that was killed at 6 GB now peaks at ~3.9 GB; a smaller one went
  from 2 GB to ~0.6 GB. Output is unchanged by these memory fixes.
- Clippy error in `image_hash.rs`.
- README examples that fail on `.fig` files saved from Figma (they are ZIP archives and need an output directory);
  clearer help text and error messages for that case.
- Writing to a closed pipe (`figma2json canvas.fig | head`) printed a "Broken pipe" error; it now exits quietly.
