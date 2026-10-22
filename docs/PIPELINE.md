# Transformation pipeline

`transform` in `src/pipeline.rs` runs the steps below in this order. The four `const` tables
(`PRUNE`, `NORMALIZE`, `CLEAN_DOCUMENT`, `CLEAN_OUTPUT`) are lists of `(Pass, When)` pairs run by
`run()`, which skips entries whose `When` does not apply. Three steps are called directly
from `transform` rather than listed in a table: blob substitution, image hashes and design context.

`When` (src/pipeline.rs):

| `When` | Runs when |
| --- | --- |
| `Always` | every mode |
| `LegacyOnly` | `ConvertOptions::legacy` (`--legacy`) |
| `DefaultOnly` | not `legacy` (the default mode) |
| `UnlessKeepInternal` | not `keep_internal` (`--keep-internal`) |

"Everywhere" below means the pass visits every JSON object in its input (via
`walk::for_each_object_mut` or its own recursion), not just nodes. A pass running on the document
does not see the root `variables` / `styles` (they are added later). A CLEAN_OUTPUT pass does.

All pass files live in `src/schema/transformations/`.

## Pass table

| # | Stage (input) | Function | File | `When` | What it does |
| --- | --- | --- | --- | --- | --- |
| 1 | PRUNE (document) | `remove_text_glyphs` | `text_glyphs_removal.rs` | Always | Removes `glyphs` from every `derivedTextData` object. |
| 2 | PRUNE | `remove_geometry_fields` | `geometry_removal.rs` | LegacyOnly | Removes `fillGeometry`, `strokeGeometry`, `windingRule`, `styleID` from every object, except objects whose own `name` starts with `icon/` or `arrows/`, or whose `symbolData.symbolOverrides[].exportSettings[]` contains an entry with `imageType`. |
| 3 | PRUNE | `remove_geometry_only` | `geometry_removal.rs` | DefaultOnly | Same exceptions; removes only `fillGeometry` and `strokeGeometry` (keeps `styleID` used by text `styleOverrideTable`s and vector networks, and `windingRule`). |
| - | blobs (document) | `substitute_blobs` (pipeline) -> `blobs::process_blobs`, `blobs::substitute_blobs` | `src/pipeline.rs`, `src/blobs/` | Always | Blob `bytes` number arrays -> base64; every `<x>Blob: n` key with a valid index and a known type (`commands`, `vectorNetwork`) becomes `<x>: <parsed>`. |
| - | image hashes (document) | `transform_image_hashes(doc, base_dir)` | `image_hash.rs` | Always | Every object under an `image` or `imageThumbnail` key with a `hash` byte array: `hash` removed, `filename: "images/<hex>"` added; if `base_dir/images/<hex>` exists and its header is PNG/JPEG/GIF/WebP/SVG, the extension is appended and the file is renamed on disk. |
| 4 | NORMALIZE (document) | `transform_matrix_to_css` | `matrix_to_css.rs` | Always | `transform` objects with all of `m00 m01 m02 m10 m11 m12` become `{x, y}` plus `rotation` (deg), `scaleX`, `scaleY`, `skewX` (deg) only when they differ from 0 / 1 by more than 1e-10. |
| 5 | NORMALIZE | `transform_colors_to_css` | `color_to_css.rs` | Always | Any object-valued field of an object that has `r`, `g`, `b` becomes `#rrggbb`, or `#rrggbbaa` when `a` differs from 1 by 0.001 or more (components clamped to 0..1, x255, rounded). |
| 6 | NORMALIZE | `simplify_enums` | `enum_simplification.rs` | Always | Every `{"__enum__": ..., "value": "X"}` becomes `"X"`. |
| 7 | NORMALIZE | `remove_default_blend_mode` | `default_blend_mode_removal.rs` | Always | Removes `blendMode` when it is the string `"NORMAL"`. |
| - | design context (document) | `add_design_context` | `design_context.rs` | DefaultOnly (`if !options.legacy` in `transform`) | Adds `id`, public `type`, `componentId` / `componentName` / `componentSetName`, `variable` / `variableCollection` / `variableId`, `fillStyle` / `strokeStyle` / `textStyle` / `effectStyle` / `gridStyle`; turns guid references into `"s:l"` strings and `guidPath`s into `"s:l/s:l"`; returns `DesignDefinitions { variables, styles }`. See [OUTPUT_FORMAT.md](OUTPUT_FORMAT.md). |
| 8 | CLEAN_DOCUMENT (document) | `remove_guid_fields` | `guid_removal.rs` | Always | Removes `guid` everywhere. |
| 9 | CLEAN_DOCUMENT | `remove_edit_info_fields` | `edit_info_removal.rs` | Always | Removes `editInfo` everywhere. |
| 10 | CLEAN_DOCUMENT | `remove_phase_fields` | `phase_removal.rs` | Always | Removes `phase` everywhere. |
| 11 | CLEAN_DOCUMENT | `remove_geometry_fields` | `geometry_removal.rs` | LegacyOnly | Second run, see below. |
| 12 | CLEAN_DOCUMENT | `remove_geometry_only` | `geometry_removal.rs` | DefaultOnly | Second run, see below. |
| 13 | CLEAN_DOCUMENT | `remove_text_layout_fields` | `text_layout_removal.rs` | Always | From every `derivedTextData`: `baselines`, `logicalIndexToCharacterOffsetMap`, `fontMetaData`, `derivedLines`, `truncatedHeight`, `truncationStartIndex`. |
| 14 | CLEAN_DOCUMENT | `remove_derived_text_layout_size` | `derived_text_layout_size_removal.rs` | Always | `derivedTextData.layoutSize`. |
| 15 | CLEAN_DOCUMENT | `remove_empty_derived_text_data` | `empty_derived_text_data_removal.rs` | Always | `derivedTextData` when it is `{}`. |
| 16 | CLEAN_DOCUMENT | `remove_text_metadata_fields` | `text_metadata_removal.rs` | Always | Everywhere: `textBidiVersion`, `textExplicitLayoutVersion`, `textUserLayoutVersion`, `textDecorationSkipInk`, `fontVariantCommonLigatures`, `fontVariantContextualLigatures`, `fontVariantNumericFigure`, `fontVariantNumericSpacing`, `fontVariations`, `fontVersion`, `emojiImageSet`, `autoRename`, `textTracking`, `textAlignVertical`, `textAutoResize`. |
| 17 | CLEAN_DOCUMENT | `remove_default_text_line_properties` | `text_line_defaults_removal.rs` | Always | In every `lines` array of objects: drops `indentationLevel: 0`, `isFirstLineOfList: false`, `lineType: "PLAIN"`, `listStartOffset: 0`, `sourceDirectionality: "AUTO"`, `styleId: 0`; removes `lines` when every element is then `{}`. |
| 18 | CLEAN_DOCUMENT | `remove_default_text_properties` | `default_text_properties_removal.rs` | Always | `letterSpacing` = `{units: "PERCENT", value: 0}` and `lineHeight` = `{units: "PERCENT", value: 100}` (tolerance 1e-10). |
| 19 | CLEAN_DOCUMENT | `simplify_text_properties` | `text_properties_simplification.rs` | Always | `letterSpacing` / `lineHeight` objects with `units` `PERCENT` / `PIXELS` become `"N%"` / `"Npx"` (integers without decimals). Other units are left as objects. |
| 20 | CLEAN_DOCUMENT | `remove_empty_font_postscript` | `empty_font_postscript_removal.rs` | Always | `fontName.postscript` when `""`. |
| 21 | CLEAN_DOCUMENT | `remove_stroke_properties` | `stroke_properties_removal.rs` | Always | `strokeAlign`, `strokeJoin`, `strokeWeight` everywhere. |
| 22 | CLEAN_DOCUMENT | `remove_border_weights` | `border_weights_removal.rs` | Always | `borderTopWeight`, `borderBottomWeight`, `borderLeftWeight`, `borderRightWeight`, `borderStrokeWeightsIndependent`. |
| 23 | CLEAN_DOCUMENT | `remove_frame_properties` | `frame_properties_removal.rs` | Always | `frameMaskDisabled`, `targetAspectRatio`. |
| 24 | CLEAN_DOCUMENT | `remove_background_properties` | `background_properties_removal.rs` | Always | `backgroundEnabled`, `backgroundOpacity`. |
| 25 | CLEAN_DOCUMENT | `remove_image_metadata_fields` | `image_metadata_removal.rs` | Always | Everywhere: `thumbHash`, `animationFrame`, `imageShouldColorManage`, `imageScaleMode`, `originalImageWidth`, `originalImageHeight`, `altText`, `imageThumbnail`. Also `rotation` and `scale` on objects whose `type` is the object `{"value": "IMAGE"}` (see known issue 1). |
| 26 | CLEAN_DOCUMENT | `remove_internal_only_nodes` | `internal_only_nodes_removal.rs` | UnlessKeepInternal | Drops every array element with `internalOnly: true`, then removes the `internalOnly` key everywhere. |
| 27 | CLEAN_DOCUMENT | `remove_default_opacity` | `default_opacity_removal.rs` | Always | `opacity` equal to 1.0 (within `f64::EPSILON`) everywhere, including paints. |
| 28 | CLEAN_DOCUMENT | `remove_default_visible` | `default_visible_removal.rs` | Always | `visible: true` everywhere. |
| 29 | CLEAN_DOCUMENT | `remove_default_rotation` | `default_rotation_removal.rs` | Always | `rotation` equal to 0 (within `f64::EPSILON`) everywhere. |
| 30 | CLEAN_DOCUMENT | `remove_default_uniform_scale_factor` | `uniform_scale_factor_removal.rs` | Always | `uniformScaleFactor` equal to 1.0. |
| - | assemble | `assemble` (+ `variables`, `styles` in default mode) | `src/pipeline.rs` | Always | Root becomes `{version, fileType, document, blobs[, variables, styles]}`. |
| 31 | CLEAN_OUTPUT (root) | `remove_document_properties` | `document_properties_removal.rs` | Always | `documentColorProfile` everywhere. |
| 32 | CLEAN_OUTPUT | `remove_root_metadata` | `root_metadata_removal.rs` | Always | Root `version`, `fileType`. |
| 33 | CLEAN_OUTPUT | `remove_root_blobs` | `blobs_removal.rs` | Always | Root `blobs`. |
| 34 | CLEAN_OUTPUT | `remove_guid_paths` | `guid_path_removal.rs` | LegacyOnly | `guidPath` everywhere. |
| 35 | CLEAN_OUTPUT | `remove_user_facing_versions` | `user_facing_version_removal.rs` | Always | `userFacingVersion`. |
| 36 | CLEAN_OUTPUT | `remove_style_ids` | `style_id_removal.rs` | Always | `styleIdForFill`, `styleIdForText`, `styleIdForStrokeFill`, `styleIdForEffect`, `styleIdForGrid`. |
| 37 | CLEAN_OUTPUT | `remove_export_settings` | `export_settings_removal.rs` | Always | `exportSettings`. |
| 38 | CLEAN_OUTPUT | `remove_plugin_data` | `plugin_data_removal.rs` | Always | `pluginData`. |
| 39 | CLEAN_OUTPUT | `remove_rectangle_corner_radii_independent` | `rectangle_corner_radii_independent_removal.rs` | Always | `rectangleCornerRadiiIndependent`. |
| 40 | CLEAN_OUTPUT | `remove_constraint_properties` | `constraint_properties_removal.rs` | LegacyOnly | `horizontalConstraint`, `verticalConstraint`. |
| 41 | CLEAN_OUTPUT | `remove_scroll_resize_properties` | `scroll_resize_properties_removal.rs` | LegacyOnly | `scrollBehavior`, `resizeToFit`. |
| 42 | CLEAN_OUTPUT | `remove_layout_aids` | `layout_aids_removal.rs` | LegacyOnly | `guides`, `layoutGrids`. |
| 43 | CLEAN_OUTPUT | `remove_detached_symbol_id` | `detached_symbol_id_removal.rs` | Always | `detachedSymbolId`. |
| 44 | CLEAN_OUTPUT | `remove_overridden_symbol_id` | `overridden_symbol_id_removal.rs` | Always | Drops array elements that are exactly `{overriddenSymbolID: {localID, sessionID}}` (see known issue 3). |
| 45 | CLEAN_OUTPUT | `remove_redundant_corner_radii` | `redundant_corner_radii_removal.rs` | Always | When an object has `cornerRadius`, removes `rectangleTopLeftCornerRadius`, `rectangleTopRightCornerRadius`, `rectangleBottomLeftCornerRadius`, `rectangleBottomRightCornerRadius`. |
| 46 | CLEAN_OUTPUT | `remove_corner_smoothing` | `corner_smoothing_removal.rs` | Always | `cornerSmoothing`. |
| 47 | CLEAN_OUTPUT | `remove_invisible_paints` | `invisible_paints_removal.rs` | Always | Removes elements with `visible: false` from `fillPaints` and `strokePaints` arrays. |
| 48 | CLEAN_OUTPUT | `remove_empty_paint_arrays` | `empty_paint_arrays_removal.rs` | Always | `fillPaints` / `strokePaints` when `[]`. |
| 49 | CLEAN_OUTPUT | `remove_redundant_padding` | `redundant_padding_removal.rs` | LegacyOnly | `stackPaddingRight` when equal to `stackHorizontalPadding`; `stackPaddingBottom` when equal to `stackVerticalPadding`. |
| 50 | CLEAN_OUTPUT | `remove_stack_child_properties` | `stack_child_properties_removal.rs` | LegacyOnly | `stackChildAlignSelf`, `stackChildPrimaryGrow`. |
| 51 | CLEAN_OUTPUT | `remove_stack_sizing_properties` | `stack_sizing_properties_removal.rs` | LegacyOnly | `stackCounterSizing`, `stackPrimarySizing`. |
| 52 | CLEAN_OUTPUT | `remove_stack_align_items` | `stack_align_items_removal.rs` | LegacyOnly | `stackCounterAlignItems`, `stackPrimaryAlignItems`. |
| 53 | CLEAN_OUTPUT | `remove_symbol_id_fields` | `symbol_id_removal.rs` | Always | `symbolID` when it is an object whose keys are only `localID` / `sessionID` (an empty object also matches). |
| 54 | CLEAN_OUTPUT | `remove_type` | `type_removal.rs` | LegacyOnly | `type` everywhere: node types and also paint, effect and any other `type` field. |
| 55 | CLEAN_OUTPUT | `remove_visible_only_objects` | `visible_only_objects_removal.rs` | Always | Bottom-up: removes object fields and array elements that are objects with `visible` as their only key. |
| 56 | CLEAN_OUTPUT | `remove_empty_objects` | `empty_objects_removal.rs` | Always | Bottom-up: removes object fields and array elements that are `{}` (including ones emptied by its own recursion). |

Not in any table: `convert_raw` runs none of these; it stops after blob substitution and
assembly (no image hash rewriting either).

## Ordering dependencies

- **Prune before blob substitution (1-3).** Glyphs and `fillGeometry` / `strokeGeometry` entries carry
  `commandsBlob` references. Removing them first means those blobs are never parsed into JSON. The
  passes do not look at blob content, so the result is the same as pruning later.
- **Geometry runs twice (2/3 and 11/12).** In legacy mode the second run matters: blob substitution
  creates `vectorNetwork` objects whose vertices, segments and regions carry `styleID` and
  `windingRule`, and the second `remove_geometry_fields` strips them. In the default mode the second
  `remove_geometry_only` finds nothing new, because substitution adds `commands` / `vectorNetwork`, not
  `fillGeometry` / `strokeGeometry`, so it is a no-op in practice (removing it would not change the
  output, but verify before doing so).
- **Colors and enums before design context (5, 6).** `add_design_context` copies variable values and
  style fields into `variables` / `styles`. They must already be hex strings and plain enum strings,
  and the `ALIAS` / `styleType` checks compare strings (`design_context.rs` module docs).
- **Blend mode after enum simplification (7 after 6).** It compares `blendMode` to the string
  `"NORMAL"`.
- **Design context before guid removal (8) and internal-only removal (26).** The index it builds needs
  every node's `guid`, and the variables, styles and library component copies live on the internal-only
  page.
- **Design context before style-id removal (36).** It reads `styleIdFor*` to add `fillStyle` and the
  other style names.
- **Text: layout removal (13, 14) before empty `derivedTextData` removal (15)**, and **default text
  properties (18) before text simplification (19)**, because 18 matches the object form
  `{units, value}` that 19 replaces with a string.
- **Default visibility (28) before visible-only cleanup (55).** After `visible: true` is gone, objects
  left with only `visible` (in practice `false`) are dropped. In legacy mode `guidPath` is removed (34),
  so a symbol override that only hid a child (`{guidPath, visible: false}`) becomes `{visible: false}`
  and is dropped by 55.
- **Invisible paints (47) before empty paint arrays (48).**
- **Corner radii independence flag (39) is removed before redundant radii (45)** looks at
  `cornerRadius` (see known issue 4).
- **`remove_empty_objects` (56) runs last** to clean up `{}` left behind by earlier passes, for example
  `{guid: ...}` wrappers emptied by guid removal in legacy mode, or paints left with only `type` in
  legacy mode after 54.
- **Root-level passes (31-56) also see `variables` and `styles`.** Passes 1-30 only see the document.

## Known issues and suspicious behaviour

These are observations from reading and probing the code. They are not fixed; fixing any of them
changes the output and needs a CHANGELOG entry.

1. **`remove_image_metadata_fields` never removes `rotation` / `scale` from image paints.** It detects
   an image paint by `type` being an object with `value == "IMAGE"`
   (`image_metadata_removal.rs`), but it runs in CLEAN_DOCUMENT, after `simplify_enums` (NORMALIZE) has
   turned `type` into the string `"IMAGE"`. In the pipeline the check is never true. Verified with
   `simplify_enums` followed by `remove_image_metadata_fields` on
   `{"type": {"__enum__": "PaintType", "value": "IMAGE"}, "rotation": 90.0, "scale": 0.5}`: `rotation`
   and `scale` are kept. `rotation: 0` is still removed by `remove_default_rotation` (29); non-zero
   rotation and every `scale` stay. The unit tests in that file use the unsimplified form, so they pass.
2. **The "no id" sentinel is handled differently in objects and arrays.** In `design_context.rs`
   `annotate_object` turns a field holding `{sessionID: 4294967295, localID: 4294967295}` (bare or
   wrapped in `{guid: ...}`) into `null` via `id_or_null`. In `annotate`'s array branch, a wrapped
   reference goes through `id_or_null` (-> `null`), but a bare guid object becomes
   `JsonValue::String(id)`, so the array element is the string `"4294967295:4294967295"`. Verified:
   `{"x": NO_ID, "arr": [NO_ID], "wr": [{"guid": NO_ID}]}` becomes
   `{"x": null, "arr": ["4294967295:4294967295"], "wr": [null]}`.
3. **`remove_overridden_symbol_id` and `remove_symbol_id_fields` only act in legacy mode in practice.**
   Both match `{localID, sessionID}` objects. In the default mode `add_design_context` has already
   turned those objects into `"s:l"` strings, so standalone `{"overriddenSymbolID": "1:2"}` entries and
   `symbolData.symbolID: "1:2"` are kept. Verified for `overriddenSymbolID`. Keeping `symbolID` may be
   intended (it is the component link), but the passes are documented as removals.
4. **Per-corner radii may be lost.** `remove_redundant_corner_radii` removes the four
   `rectangle*CornerRadius` fields whenever `cornerRadius` exists, without comparing values, and
   `rectangleCornerRadiiIndependent` (which says whether corners differ) has already been removed (39).
   If Figma stores a `cornerRadius` next to independent corner values, the per-corner values are
   dropped. Verified on `{"cornerRadius": 4, "rectangleTopLeftCornerRadius": 8}`, which becomes
   `{"cornerRadius": 4}`. Whether real files contain that combination still needs checking.
5. **A repeated image hash gets no extension after the first reference.** `transform_image` renames
   `images/<hex>` to `images/<hex>.<ext>` on the first reference. Later references find no file at
   `images/<hex>`, cannot detect the format, and keep `"images/<hex>"` without extension. Verified with
   two `image` objects sharing a hash: `images/abcd.png` and `images/abcd`. Re-running bare mode on an
   already-converted directory has the same effect for every image.
6. **`styles` (and parts of `variables`) skip document cleanup.** `Definitions::collect` copies style
   fields before annotation, and CLEAN_DOCUMENT runs on the document only. Style `fillPaints` can keep
   `visible: true`, `opacity: 1.0` and raw alias references such as
   `"colorVar": {"value": {"alias": {"guid": {"localID": 9, "sessionID": 0}}}}` (verified with a
   synthetic style), and text styles can keep `letterSpacing: "0%"` / `lineHeight: "100%"`. CLEAN_OUTPUT
   passes do reach them.
7. **Variables inside a collection can carry `null` fields.** `tidy` only runs on collection and style
   objects, not on the variables nested in `collection.variables`, so `key`, `scopes` or `type` can be
   `null` when the source node lacks them (verified).
8. **Legacy `remove_type` removes every `type`**, not just node types: paint types (`SOLID`, `IMAGE`,
   `GRADIENT_*`) and effect types (`DROP_SHADOW`, ...) are lost in `--legacy` output (verified).
9. **Design-relevant fields are dropped in the default mode too**: `strokeWeight`, `strokeAlign`,
   `strokeJoin` (21), the border weights (22) and `textAutoResize` / `textAlignVertical` (16) are removed
   with `When::Always`.
10. **`transform_colors_to_css` converts any `{r, g, b}` object that is a field value**, not only
    colors, and does not convert color objects that are direct array elements.
11. **Module docs for `transformations` land on the wrong item.** The pass list at the top of
    `src/schema/transformations/mod.rs` is written as `///` and therefore documents the first item,
    `pub mod background_properties_removal;`, not the `transformations` module. It needs `//!` instead.
12. **Images are looked up relative to `base_dir`, not the input file.** In bare mode without `-o` the
    CLI passes the current directory (`src/cli.rs`, `convert_input`), so converting
    `out/canvas.fig` from another directory leaves image filenames without extensions. Relatedly, a
    second positional argument (`OUTPUT_DIR`) is silently ignored in bare mode.
