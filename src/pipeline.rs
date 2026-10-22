//! The conversion pipeline behind [`convert`], [`convert_with_options`] and
//! [`convert_raw`].
//!
//! Conversion has two phases:
//!
//! 1. **Decode** ([`decode`]): unwrap the ZIP container if there is one, detect the
//!    file type, split the file into chunks, decompress them, decode the Kiwi
//!    message and build the node tree from its `nodeChanges`.
//! 2. **Transform** ([`transform`]): run the transformation passes, in the stages
//!    listed below, over the document and then over the assembled output root.
//!
//! The passes that touch every node uniformly are listed in the `const` stage
//! tables; each entry says which mode it runs in ([`When`]). Order matters: some
//! passes rely on the output of earlier ones (blend-mode removal needs simplified
//! enums, design context needs the guids that guid removal deletes, ...).

use std::path::Path;

use serde_json::{json, Value};

use crate::error::{FigError, Result};
use crate::schema;
use crate::types::FileType;
use crate::{blobs, parser};

/// Options for [`convert_with_options`].
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Reproduce the original fig2json (pre-0.4) output: no node ids or types, no variable
    /// or style names, no `variables`/`styles` at the root, and auto-layout
    /// alignment, sizing, constraints, scroll behavior, layout grids and override
    /// paths removed. Off by default because implementing a design needs them.
    pub legacy: bool,
    /// Keep internal-only nodes, such as the hidden page holding local copies of the
    /// library components (and variables/styles) the file uses. Those copies are the
    /// only place a library component's contents appear in the file.
    pub keep_internal: bool,
}

/// Convert a `.fig` file to JSON with the default options.
///
/// The bytes may be a bare `.fig` file or the ZIP container Figma saves. Images
/// referenced by the document are expected in `base_dir` (the current directory
/// when `None`); they are renamed in place to carry their detected extension.
///
/// Equivalent to [`convert_with_options`] with [`ConvertOptions::default()`].
///
/// # Errors
/// Returns a [`FigError`] if any stage of decoding or transformation fails.
///
/// # Example
/// ```no_run
/// use std::path::Path;
///
/// let bytes = std::fs::read("example.fig")?;
/// let json = figma2json::convert(&bytes, Some(Path::new("/output/dir")))?;
/// println!("{}", serde_json::to_string_pretty(&json)?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn convert(bytes: &[u8], base_dir: Option<&Path>) -> Result<Value> {
    convert_with_options(bytes, base_dir, &ConvertOptions::default())
}

/// Convert a `.fig` file to JSON with options (see [`ConvertOptions`]).
///
/// By default the output keeps what is needed to implement a design: node `id`s
/// and `type`s, component links on instances, the names of the variables and
/// styles each property is bound to, auto-layout alignment/sizing and
/// constraints, plus `variables` (collections, modes, values) and `styles` at the
/// root of the output.
///
/// # Errors
/// Returns a [`FigError`] if any stage of decoding or transformation fails.
pub fn convert_with_options(
    bytes: &[u8],
    base_dir: Option<&Path>,
    options: &ConvertOptions,
) -> Result<Value> {
    transform(decode(bytes)?, base_dir, options)
}

/// Convert a `.fig` file to JSON without any transformation passes.
///
/// The output is the decoded node tree with blob references substituted, plus
/// the file's `version`, `fileType` and (base64-encoded) `blobs`: every
/// Figma-specific field and internal structure is kept.
///
/// # Errors
/// Returns a [`FigError`] if decoding fails.
///
/// # Example
/// ```no_run
/// let bytes = std::fs::read("example.fig")?;
/// let json = figma2json::convert_raw(&bytes)?;
/// println!("{}", serde_json::to_string_pretty(&json)?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn convert_raw(bytes: &[u8]) -> Result<Value> {
    let Decoded {
        version,
        file_type,
        mut document,
        blobs,
    } = decode(bytes)?;
    let blobs = substitute_blobs(&mut document, blobs)?;
    Ok(assemble(version, file_type, document, blobs))
}

// ---------------------------------------------------------------------------
// Decode phase
// ---------------------------------------------------------------------------

/// A decoded `.fig` file: the node tree plus the still-unprocessed blobs.
struct Decoded {
    version: u32,
    file_type: FileType,
    /// The node tree built from `nodeChanges`, rooted at node `0:0`.
    document: Value,
    /// The raw `blobs` array of the Kiwi message (binary data as byte arrays).
    blobs: Vec<Value>,
}

/// Decode a `.fig` file (bare or in a ZIP container) into its node tree and blobs.
fn decode(bytes: &[u8]) -> Result<Decoded> {
    let unzipped;
    let bytes = if parser::is_zip_container(bytes) {
        unzipped = parser::extract_from_zip(bytes)?;
        &unzipped[..]
    } else {
        bytes
    };

    let file_type = parser::detect_file_type(bytes)?;
    let parsed = parser::extract_chunks(bytes)?;

    let mut message = {
        let schema_chunk = parsed.schema_chunk().ok_or(FigError::NotEnoughChunks {
            expected: 1,
            actual: 0,
        })?;
        let schema_bytes = parser::decompress_chunk(schema_chunk)?;
        let data_chunk = parsed.data_chunk().ok_or(FigError::NotEnoughChunks {
            expected: 2,
            actual: parsed.chunks.len(),
        })?;
        let data_bytes = parser::decompress_chunk(data_chunk)?;
        schema::decode_fig_to_json(&schema_bytes, &data_bytes)?
    };

    // Move (rather than clone) the node changes and blobs out of the decoded
    // message: they are by far the largest values in memory.
    let node_changes = take_array(&mut message, "nodeChanges")?;
    let document = schema::build_tree(node_changes)?;
    let blobs = take_array(&mut message, "blobs")?;

    Ok(Decoded {
        version: parsed.version,
        file_type,
        document,
        blobs,
    })
}

/// Move the array at `message[key]` out of the decoded message.
fn take_array(message: &mut Value, key: &str) -> Result<Vec<Value>> {
    message
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .ok_or_else(|| FigError::InvalidDocument(format!("No {key} found in decoded data")))
}

/// Base64-encode the blobs and replace blob references in `document` (such as
/// `commandsBlob: 5`) with the parsed blob content (`commands: [...]`).
///
/// Returns the processed blobs.
fn substitute_blobs(document: &mut Value, blobs: Vec<Value>) -> Result<Vec<Value>> {
    let blobs = match blobs::process_blobs(blobs)? {
        Value::Array(blobs) => blobs,
        _ => {
            return Err(FigError::InvalidBlob(
                "processed blobs are not an array".to_string(),
            ))
        }
    };
    blobs::substitute_blobs(document, &blobs)?;
    Ok(blobs)
}

/// Build the output root object around the document tree.
fn assemble(version: u32, file_type: FileType, document: Value, blobs: Vec<Value>) -> Value {
    json!({
        "version": version,
        "fileType": file_type.as_str(),
        "document": document,
        "blobs": blobs,
    })
}

// ---------------------------------------------------------------------------
// Transform phase
// ---------------------------------------------------------------------------

/// A transformation pass over a JSON value.
type Pass = fn(&mut Value) -> Result<()>;

/// In which conversion mode a pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    /// In every mode.
    Always,
    /// Only with [`ConvertOptions::legacy`].
    LegacyOnly,
    /// Only without [`ConvertOptions::legacy`] (the default mode).
    DefaultOnly,
    /// Unless [`ConvertOptions::keep_internal`] is set.
    UnlessKeepInternal,
}

impl When {
    fn applies(self, options: &ConvertOptions) -> bool {
        match self {
            When::Always => true,
            When::LegacyOnly => options.legacy,
            When::DefaultOnly => !options.legacy,
            When::UnlessKeepInternal => !options.keep_internal,
        }
    }
}

/// Stage 1, on the document, before blob substitution.
///
/// Text glyphs and path geometry are discarded from the final output anyway;
/// removing them before substitution means their blobs are never expanded into
/// JSON (on large files that expansion alone used gigabytes of memory). Neither
/// pass depends on blob content, so the output is unchanged.
///
/// Legacy geometry removal also strips every `windingRule`/`styleID`, as upstream
/// did; the default mode keeps them (text style runs, vector regions).
const PRUNE: &[(Pass, When)] = &[
    (schema::remove_text_glyphs, When::Always),
    (schema::remove_geometry_fields, When::LegacyOnly),
    (schema::remove_geometry_only, When::DefaultOnly),
];

/// Stage 2, on the document, after blob substitution and image hash
/// transformation: convert Figma encodings to CSS-friendly values.
const NORMALIZE: &[(Pass, When)] = &[
    (schema::transform_matrix_to_css, When::Always),
    (schema::transform_colors_to_css, When::Always),
    (schema::simplify_enums, When::Always),
    // Needs simplified enums.
    (schema::remove_default_blend_mode, When::Always),
];

/// Stage 3, on the document, after design context has been captured (it needs
/// the guids and internal-only nodes these passes remove).
const CLEAN_DOCUMENT: &[(Pass, When)] = &[
    (schema::remove_guid_fields, When::Always),
    (schema::remove_edit_info_fields, When::Always),
    (schema::remove_phase_fields, When::Always),
    // Second geometry run: catches geometry inside substituted blobs.
    (schema::remove_geometry_fields, When::LegacyOnly),
    (schema::remove_geometry_only, When::DefaultOnly),
    (schema::remove_text_layout_fields, When::Always),
    (schema::remove_derived_text_layout_size, When::Always),
    (schema::remove_empty_derived_text_data, When::Always),
    (schema::remove_text_metadata_fields, When::Always),
    (schema::remove_default_text_line_properties, When::Always),
    (schema::remove_default_text_properties, When::Always),
    (schema::simplify_text_properties, When::Always),
    (schema::remove_empty_font_postscript, When::Always),
    (schema::remove_stroke_properties, When::Always),
    (schema::remove_border_weights, When::Always),
    (schema::remove_frame_properties, When::Always),
    (schema::remove_background_properties, When::Always),
    (schema::remove_image_metadata_fields, When::Always),
    (schema::remove_internal_only_nodes, When::UnlessKeepInternal),
    (schema::remove_default_opacity, When::Always),
    (schema::remove_default_visible, When::Always),
    (schema::remove_default_rotation, When::Always),
    (schema::remove_default_uniform_scale_factor, When::Always),
];

/// Stage 4, on the assembled output root (`document`, `blobs`, `version`,
/// `fileType` and, in the default mode, `variables` and `styles`).
const CLEAN_OUTPUT: &[(Pass, When)] = &[
    (schema::remove_document_properties, When::Always),
    (schema::remove_root_metadata, When::Always),
    (schema::remove_root_blobs, When::Always),
    (schema::remove_guid_paths, When::LegacyOnly),
    (schema::remove_user_facing_versions, When::Always),
    (schema::remove_style_ids, When::Always),
    (schema::remove_export_settings, When::Always),
    (schema::remove_plugin_data, When::Always),
    (
        schema::remove_rectangle_corner_radii_independent,
        When::Always,
    ),
    (schema::remove_constraint_properties, When::LegacyOnly),
    (schema::remove_scroll_resize_properties, When::LegacyOnly),
    (schema::remove_layout_aids, When::LegacyOnly),
    (schema::remove_detached_symbol_id, When::Always),
    (schema::remove_overridden_symbol_id, When::Always),
    (schema::remove_redundant_corner_radii, When::Always),
    (schema::remove_corner_smoothing, When::Always),
    (schema::remove_invisible_paints, When::Always),
    (schema::remove_empty_paint_arrays, When::Always),
    // Legacy only: Figma omits padding that is 0, so once right/bottom are also
    // removed when equal to left/top, a missing value could mean either. The
    // default output keeps every padding value Figma stores (missing = 0).
    (schema::remove_redundant_padding, When::LegacyOnly),
    (schema::remove_stack_child_properties, When::LegacyOnly),
    (schema::remove_stack_sizing_properties, When::LegacyOnly),
    (schema::remove_stack_align_items, When::LegacyOnly),
    (schema::remove_symbol_id_fields, When::Always),
    (schema::remove_type, When::LegacyOnly),
    (schema::remove_visible_only_objects, When::Always),
    (schema::remove_empty_objects, When::Always),
];

/// Run the passes of `stage` that apply under `options`, in order.
fn run(stage: &[(Pass, When)], value: &mut Value, options: &ConvertOptions) -> Result<()> {
    stage
        .iter()
        .filter(|(_, when)| when.applies(options))
        .try_for_each(|(pass, _)| pass(value))
}

/// Run every transformation stage over a decoded file and assemble the output.
fn transform(decoded: Decoded, base_dir: Option<&Path>, options: &ConvertOptions) -> Result<Value> {
    let Decoded {
        version,
        file_type,
        mut document,
        blobs,
    } = decoded;

    run(PRUNE, &mut document, options)?;
    let blobs = substitute_blobs(&mut document, blobs)?;

    // Image hash arrays become `images/<hex>.<ext>` filenames; the image files
    // in `base_dir` are renamed to carry the detected extension.
    schema::transform_image_hashes(&mut document, base_dir.unwrap_or(Path::new(".")))?;
    run(NORMALIZE, &mut document, options)?;

    // Capture ids, types, component links and variable/style names before
    // guid removal and internal-node removal delete what this needs.
    let definitions = if options.legacy {
        None
    } else {
        Some(schema::add_design_context(&mut document)?)
    };
    run(CLEAN_DOCUMENT, &mut document, options)?;

    let mut output = assemble(version, file_type, document, blobs);
    if let Some(definitions) = definitions {
        output["variables"] = definitions.variables;
        output["styles"] = definitions.styles;
    }
    run(CLEAN_OUTPUT, &mut output, options)?;

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(legacy: bool, keep_internal: bool) -> ConvertOptions {
        ConvertOptions {
            legacy,
            keep_internal,
        }
    }

    fn guid(local_id: u32) -> Value {
        json!({"sessionID": 0, "localID": local_id})
    }

    fn node_type(value: &str) -> Value {
        json!({"__enum__": "NodeType", "value": value})
    }

    /// A small document: a page with a frame (auto-layout, default opacity and
    /// blend mode) and an internal-only page.
    fn sample() -> Decoded {
        let document = json!({
            "guid": guid(0),
            "type": node_type("DOCUMENT"),
            "name": "Document",
            "children": [
                {
                    "guid": guid(1),
                    "type": node_type("CANVAS"),
                    "name": "Page 1",
                    "children": [{
                        "guid": guid(2),
                        "type": node_type("FRAME"),
                        "name": "Card",
                        "opacity": 1.0,
                        "visible": true,
                        "blendMode": {"__enum__": "BlendMode", "value": "NORMAL"},
                        "stackPrimaryAlignItems": {"__enum__": "StackJustify", "value": "CENTER"},
                        "editInfo": {"userId": "1"},
                    }],
                },
                {
                    "guid": guid(3),
                    "type": node_type("CANVAS"),
                    "name": "Internal Only Canvas",
                    "internalOnly": true,
                },
            ],
        });
        Decoded {
            version: 48,
            file_type: FileType::Figma,
            document,
            blobs: Vec::new(),
        }
    }

    fn convert_sample(options: &ConvertOptions) -> Value {
        let base_dir = Path::new("/nonexistent-figma2json-test-dir");
        transform(sample(), Some(base_dir), options).unwrap()
    }

    fn frame(output: &Value) -> &Value {
        &output["document"]["children"][0]["children"][0]
    }

    #[test]
    fn when_applies_to_the_matching_modes() {
        let modes = [
            options(false, false),
            options(true, false),
            options(false, true),
            options(true, true),
        ];
        let applies = |when: When| modes.each_ref().map(|o| when.applies(o));
        assert_eq!(applies(When::Always), [true, true, true, true]);
        assert_eq!(applies(When::LegacyOnly), [false, true, false, true]);
        assert_eq!(applies(When::DefaultOnly), [true, false, true, false]);
        assert_eq!(
            applies(When::UnlessKeepInternal),
            [true, true, false, false]
        );
    }

    #[test]
    fn default_mode_keeps_design_context() {
        let output = convert_sample(&ConvertOptions::default());

        // Root metadata and blobs are removed; variables/styles are added.
        let root = output.as_object().unwrap();
        assert!(!root.contains_key("version"));
        assert!(!root.contains_key("fileType"));
        assert!(!root.contains_key("blobs"));
        assert!(root.contains_key("document"));

        let frame = frame(&output);
        assert_eq!(frame["name"], "Card");
        assert_eq!(frame["type"], "FRAME");
        assert!(frame.get("id").is_some());
        assert_eq!(frame["stackPrimaryAlignItems"], "CENTER");
        for removed in ["guid", "opacity", "visible", "blendMode", "editInfo"] {
            assert!(frame.get(removed).is_none(), "{removed} should be removed");
        }

        // The internal-only page is removed.
        assert_eq!(output["document"]["children"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn legacy_mode_drops_design_context() {
        let output = convert_sample(&options(true, false));
        let root = output.as_object().unwrap();
        assert!(!root.contains_key("variables"));
        assert!(!root.contains_key("styles"));

        let frame = frame(&output);
        assert_eq!(frame["name"], "Card");
        for removed in ["id", "type", "stackPrimaryAlignItems", "guid"] {
            assert!(frame.get(removed).is_none(), "{removed} should be removed");
        }
    }

    #[test]
    fn keep_internal_keeps_internal_only_nodes() {
        for legacy in [false, true] {
            let output = convert_sample(&options(legacy, true));
            let pages = output["document"]["children"].as_array().unwrap();
            assert_eq!(pages.len(), 2);
            assert_eq!(pages[1]["name"], "Internal Only Canvas");
        }
    }

    #[test]
    fn raw_assembly_keeps_root_metadata() {
        let Decoded {
            version,
            file_type,
            mut document,
            blobs,
        } = sample();
        let blobs = substitute_blobs(&mut document, blobs).unwrap();
        let output = assemble(version, file_type, document.clone(), blobs);
        assert_eq!(
            output,
            json!({"version": 48, "fileType": "figma", "document": document, "blobs": []})
        );
    }

    #[test]
    fn missing_arrays_are_invalid_documents() {
        let mut message = json!({"nodeChanges": [1, 2]});
        assert_eq!(take_array(&mut message, "nodeChanges").unwrap().len(), 2);
        // Taken, not cloned.
        assert_eq!(message["nodeChanges"], json!([]));
        assert!(matches!(
            take_array(&mut message, "blobs"),
            Err(FigError::InvalidDocument(msg)) if msg == "No blobs found in decoded data"
        ));
    }
}
