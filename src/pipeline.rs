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

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::error::{FigError, Result};
use crate::schema;
use crate::types::FileType;
use crate::{blobs, parser};

/// Options for [`convert_with_options`].
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Produce a smaller output without design context: no node ids or types, no variable
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
    transform(
        decode(bytes)?,
        Images::Dir(base_dir.unwrap_or(Path::new("."))),
        options,
    )
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

/// A file that goes next to the JSON produced by [`convert_in_memory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFile {
    /// `/`-separated path relative to the JSON, such as `images/<hash>.png`.
    pub path: String,
    /// The file contents.
    pub contents: Vec<u8>,
}

/// The result of [`convert_in_memory`].
#[derive(Debug, Clone)]
pub struct InMemoryConversion {
    /// The converted document, as [`convert_with_options`] returns it.
    pub json: Value,
    /// The other files of the ZIP container, in archive order (empty for a bare
    /// `.fig`): images under `images/`, renamed to carry their detected
    /// extension as the JSON references them, plus files such as `meta.json`
    /// and `thumbnail.png`. The `canvas.fig` entry itself is not included.
    pub files: Vec<OutputFile>,
}

/// Convert a `.fig` file to JSON without touching the filesystem.
///
/// Works like [`convert_with_options`], but a ZIP container is read in memory
/// instead of being extracted to disk, and image extensions are detected from
/// the image bytes in the archive. Every reference to an image gets the
/// detected extension. This is the entry point for targets without a
/// filesystem, such as WebAssembly in the browser.
///
/// # Errors
/// Returns a [`FigError`] if the archive cannot be read, has no `canvas.fig`,
/// or any stage of decoding or transformation fails.
///
/// # Example
/// ```no_run
/// use figma2json::{convert_in_memory, ConvertOptions};
///
/// let bytes = std::fs::read("example.fig")?;
/// let conversion = convert_in_memory(&bytes, &ConvertOptions::default())?;
/// for file in &conversion.files {
///     println!("{} ({} bytes)", file.path, file.contents.len());
/// }
/// println!("{}", serde_json::to_string_pretty(&conversion.json)?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn convert_in_memory(bytes: &[u8], options: &ConvertOptions) -> Result<InMemoryConversion> {
    if !parser::is_zip_container(bytes) {
        let json = transform(decode(bytes)?, Images::InMemory(&HashMap::new()), options)?;
        return Ok(InMemoryConversion {
            json,
            files: Vec::new(),
        });
    }

    let mut entries = parser::extract_zip_to_memory(bytes)?;
    let canvas_index = entries
        .iter()
        .position(|(path, _)| path == CANVAS_ENTRY)
        .ok_or(FigError::CanvasNotFoundInZip)?;
    let (_, canvas) = entries.remove(canvas_index);

    let json = {
        let images: HashMap<&str, &[u8]> = entries
            .iter()
            .filter(|(path, _)| path.starts_with(IMAGES_DIR))
            .map(|(path, contents)| (path.as_str(), contents.as_slice()))
            .collect();
        transform(decode(&canvas)?, Images::InMemory(&images), options)?
    };

    let files = entries
        .into_iter()
        .map(|(mut path, contents)| {
            if path.starts_with(IMAGES_DIR) {
                if let Some(extension) = schema::image_extension(&contents) {
                    path.push_str(extension);
                }
            }
            OutputFile { path, contents }
        })
        .collect();

    Ok(InMemoryConversion { json, files })
}

const CANVAS_ENTRY: &str = "canvas.fig";

const IMAGES_DIR: &str = "images/";

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
///
/// The node changes and blobs are moved (not cloned) out of the decoded
/// message: they are by far the largest values in memory.
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
/// Legacy geometry removal also strips every `windingRule`/`styleID`;
/// the default mode keeps them (text style runs, vector regions).
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
    (schema::remove_default_blend_mode, When::Always),
];

/// Stage 3, on the document, after design context has been captured (it needs
/// the guids and internal-only nodes these passes remove).
const CLEAN_DOCUMENT: &[(Pass, When)] = &[
    (schema::remove_guid_fields, When::Always),
    (schema::remove_edit_info_fields, When::Always),
    (schema::remove_phase_fields, When::Always),
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
///
/// Redundant padding removal is legacy only: Figma omits padding that is 0, so
/// once right/bottom are also removed when equal to left/top, a missing value
/// could mean either. The default output keeps every padding value Figma
/// stores (missing = 0).
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

#[derive(Clone, Copy)]
enum Images<'a> {
    Dir(&'a Path),
    InMemory(&'a HashMap<&'a str, &'a [u8]>),
}

/// Run every transformation stage over a decoded file and assemble the output.
fn transform(decoded: Decoded, images: Images<'_>, options: &ConvertOptions) -> Result<Value> {
    let Decoded {
        version,
        file_type,
        mut document,
        blobs,
    } = decoded;

    run(PRUNE, &mut document, options)?;
    let blobs = substitute_blobs(&mut document, blobs)?;

    match images {
        Images::Dir(base_dir) => schema::transform_image_hashes(&mut document, base_dir)?,
        Images::InMemory(images) => schema::transform_image_hashes_with(&mut document, |path| {
            images
                .get(path)
                .and_then(|bytes| schema::image_extension(bytes))
        })?,
    }
    run(NORMALIZE, &mut document, options)?;

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
        transform(sample(), Images::Dir(base_dir), options).unwrap()
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
        assert_eq!(message["nodeChanges"], json!([]));
        assert!(matches!(
            take_array(&mut message, "blobs"),
            Err(FigError::InvalidDocument(msg)) if msg == "No blobs found in decoded data"
        ));
    }
}

#[cfg(test)]
mod in_memory_tests {
    use super::*;
    use kiwi_schema::{
        Def, DefKind, Field, Schema, Value as Kiwi, TYPE_BYTE, TYPE_STRING, TYPE_UINT,
    };
    use std::io::{Cursor, Write};

    const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0];

    fn field(name: &str, type_id: i32, is_array: bool, value: u32) -> Field {
        Field {
            name: name.to_owned(),
            type_id,
            is_array,
            value,
        }
    }

    fn guid(local_id: u32) -> Kiwi<'static> {
        Kiwi::Object(
            "GUID",
            HashMap::from([
                ("sessionID", Kiwi::UInt(0)),
                ("localID", Kiwi::UInt(local_id)),
            ]),
        )
    }

    fn image(hash: &[u8], name: &str) -> Kiwi<'static> {
        Kiwi::Object(
            "Image",
            HashMap::from([
                (
                    "hash",
                    Kiwi::Array(hash.iter().copied().map(Kiwi::Byte).collect()),
                ),
                ("name", Kiwi::String(name.to_owned())),
            ]),
        )
    }

    fn node(
        local_id: u32,
        parent: Option<u32>,
        kind: &'static str,
        name: &str,
        paints: Vec<Kiwi<'static>>,
    ) -> Kiwi<'static> {
        let mut fields = HashMap::from([
            ("guid", guid(local_id)),
            ("type", Kiwi::Enum("NodeType", kind)),
            ("name", Kiwi::String(name.to_owned())),
        ]);
        if let Some(parent) = parent {
            let index = HashMap::from([
                ("guid", guid(parent)),
                ("position", Kiwi::String("!".into())),
            ]);
            fields.insert("parentIndex", Kiwi::Object("ParentIndex", index));
        }
        if !paints.is_empty() {
            fields.insert("fillPaints", Kiwi::Array(paints));
        }
        Kiwi::Object("NodeChange", fields)
    }

    fn fig(hashes: &[&[u8]]) -> Vec<u8> {
        let schema = Schema::new(vec![
            Def::new(
                "NodeType".into(),
                DefKind::Enum,
                vec![
                    field("DOCUMENT", 0, false, 1),
                    field("CANVAS", 0, false, 2),
                    field("FRAME", 0, false, 3),
                ],
            ),
            Def::new(
                "GUID".into(),
                DefKind::Struct,
                vec![
                    field("sessionID", TYPE_UINT, false, 1),
                    field("localID", TYPE_UINT, false, 2),
                ],
            ),
            Def::new(
                "ParentIndex".into(),
                DefKind::Message,
                vec![
                    field("guid", 1, false, 1),
                    field("position", TYPE_STRING, false, 2),
                ],
            ),
            Def::new(
                "Image".into(),
                DefKind::Message,
                vec![
                    field("hash", TYPE_BYTE, true, 1),
                    field("name", TYPE_STRING, false, 2),
                ],
            ),
            Def::new(
                "Paint".into(),
                DefKind::Message,
                vec![field("image", 3, false, 1)],
            ),
            Def::new(
                "NodeChange".into(),
                DefKind::Message,
                vec![
                    field("guid", 1, false, 1),
                    field("parentIndex", 2, false, 2),
                    field("type", 0, false, 3),
                    field("name", TYPE_STRING, false, 4),
                    field("fillPaints", 4, true, 5),
                ],
            ),
            Def::new(
                "Blob".into(),
                DefKind::Message,
                vec![field("bytes", TYPE_BYTE, true, 1)],
            ),
            Def::new(
                "Message".into(),
                DefKind::Message,
                vec![field("nodeChanges", 5, true, 1), field("blobs", 6, true, 2)],
            ),
        ]);
        let paints = hashes
            .iter()
            .enumerate()
            .map(|(i, hash)| {
                Kiwi::Object(
                    "Paint",
                    HashMap::from([("image", image(hash, &format!("Image {i}")))]),
                )
            })
            .collect();
        let message = Kiwi::Object(
            "Message",
            HashMap::from([
                (
                    "nodeChanges",
                    Kiwi::Array(vec![
                        node(0, None, "DOCUMENT", "Document", Vec::new()),
                        node(1, Some(0), "CANVAS", "Page 1", Vec::new()),
                        node(2, Some(1), "FRAME", "Frame", paints),
                    ]),
                ),
                ("blobs", Kiwi::Array(Vec::new())),
            ]),
        );

        let mut deflate =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        deflate.write_all(&schema.encode()).unwrap();
        let schema_chunk = deflate.finish().unwrap();
        let data_chunk = zstd::encode_all(&message.encode(&schema)[..], 3).unwrap();

        let mut file = b"fig-kiwi".to_vec();
        file.extend(48u32.to_le_bytes());
        for chunk in [schema_chunk, data_chunk] {
            file.extend(u32::try_from(chunk.len()).unwrap().to_le_bytes());
            file.extend(chunk);
        }
        file
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in entries {
            writer
                .start_file(*name, zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn filenames(json: &Value) -> Vec<&str> {
        json["document"]["children"][0]["children"][0]["fillPaints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|paint| paint["image"]["filename"].as_str().unwrap())
            .collect()
    }

    fn all_options() -> Vec<ConvertOptions> {
        [(false, false), (true, false), (false, true), (true, true)]
            .into_iter()
            .map(|(legacy, keep_internal)| ConvertOptions {
                legacy,
                keep_internal,
            })
            .collect()
    }

    #[test]
    fn bare_fig_matches_convert_with_options() {
        let fig = fig(&[&[0xab, 0xcd]]);
        let base_dir = Path::new("/nonexistent-figma2json-test-dir");
        for options in all_options() {
            let conversion = convert_in_memory(&fig, &options).unwrap();
            assert_eq!(
                conversion.json,
                convert_with_options(&fig, Some(base_dir), &options).unwrap()
            );
            assert_eq!(filenames(&conversion.json), ["images/abcd"]);
            assert!(conversion.files.is_empty());
        }
    }

    #[test]
    fn zip_container_matches_extracting_to_disk() {
        let canvas = fig(&[&[0x01, 0x02], &[0x03, 0x04], &[0x05, 0x06]]);
        let archive = zip(&[
            ("thumbnail.png", PNG),
            ("canvas.fig", &canvas),
            ("images/0102", PNG),
            ("images/0304", JPEG),
            ("images/0506", b"not an image"),
            ("images/ffff", JPEG),
            ("meta.json", b"{}"),
        ]);

        for options in all_options() {
            let conversion = convert_in_memory(&archive, &options).unwrap();
            assert_eq!(
                filenames(&conversion.json),
                ["images/0102.png", "images/0304.jpg", "images/0506"]
            );
            let paths: Vec<_> = conversion
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect();
            assert_eq!(
                paths,
                [
                    "thumbnail.png",
                    "images/0102.png",
                    "images/0304.jpg",
                    "images/0506",
                    "images/ffff.jpg",
                    "meta.json"
                ]
            );
            assert_eq!(conversion.files[2].contents, JPEG);

            let dir = std::env::temp_dir().join(format!(
                "figma2json-in-memory-test-{}-{}-{}",
                std::process::id(),
                options.legacy,
                options.keep_internal
            ));
            parser::extract_zip_to_directory(&archive, &dir).unwrap();
            let on_disk = convert_with_options(&canvas, Some(&dir), &options).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
            assert_eq!(conversion.json, on_disk);
        }
    }

    #[test]
    fn every_reference_to_an_image_gets_its_extension() {
        let canvas = fig(&[&[0x01, 0x02], &[0x01, 0x02]]);
        let archive = zip(&[("canvas.fig", &canvas), ("images/0102", PNG)]);
        let conversion = convert_in_memory(&archive, &ConvertOptions::default()).unwrap();
        assert_eq!(
            filenames(&conversion.json),
            ["images/0102.png", "images/0102.png"]
        );
    }

    #[test]
    fn zip_without_canvas_is_an_error() {
        let archive = zip(&[("other.fig", &fig(&[]))]);
        assert!(matches!(
            convert_in_memory(&archive, &ConvertOptions::default()),
            Err(FigError::CanvasNotFoundInZip)
        ));
        assert!(matches!(
            convert_in_memory(b"fig-kiwi", &ConvertOptions::default()),
            Err(FigError::FileTooSmall { .. })
        ));
    }
}
