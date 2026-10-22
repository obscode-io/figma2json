//! WebAssembly bindings for [`figma2json`].
//!
//! The browser app in `web/www/` loads this crate, built with
//! `wasm-bindgen --target web`, in a Web Worker. Conversion runs entirely in
//! the browser: [`convert`] takes the bytes of a `.fig` file (or the ZIP
//! container Figma saves) and returns a [`Conversion`] holding the
//! `canvas.json` text and, for a ZIP container, its images and other files.
//! [`Conversion::to_zip`] packages everything as the CLI would lay it out in
//! its output directory.
//!
//! The same API is usable from Rust: [`Conversion::new`] and
//! [`Conversion::zip`] report errors as [`String`]s instead of JavaScript
//! errors.

#![forbid(unsafe_code)]

use std::io::{Cursor, Write};

use figma2json::{ConvertOptions, OutputFile};
use wasm_bindgen::prelude::*;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Name of the converted document, as the CLI writes it.
pub const JSON_NAME: &str = "canvas.json";

/// Name of the untransformed document written with [`Options::raw`].
pub const RAW_JSON_NAME: &str = "canvas.raw.json";

/// Conversion options, matching the CLI flags of the same names.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Compact JSON instead of indented (`--compact`).
    pub compact: bool,
    /// Smaller output without design context (`--legacy`).
    pub legacy: bool,
    /// Keep internal-only nodes (`--keep-internal`).
    #[wasm_bindgen(js_name = keepInternal)]
    pub keep_internal: bool,
    /// Also produce `canvas.raw.json`, the decoded file with no
    /// transformations applied (`--raw`).
    pub raw: bool,
}

#[wasm_bindgen]
impl Options {
    /// Options with every flag off, like the CLI without flags.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Options {
        Options::default()
    }
}

impl Options {
    fn convert_options(self) -> ConvertOptions {
        ConvertOptions {
            legacy: self.legacy,
            keep_internal: self.keep_internal,
        }
    }

    fn to_json(self, value: &serde_json::Value) -> Result<String, String> {
        let json = if self.compact {
            serde_json::to_string(value)
        } else {
            serde_json::to_string_pretty(value)
        };
        json.map_err(|error| format!("Failed to serialize JSON: {error}"))
    }
}

/// A converted `.fig` file.
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct Conversion {
    json: String,
    raw_json: Option<String>,
    files: Vec<OutputFile>,
}

impl Conversion {
    /// Convert the bytes of a `.fig` file or of its ZIP container.
    ///
    /// # Errors
    /// Returns the error message when the file cannot be decoded or converted.
    pub fn new(bytes: &[u8], options: &Options) -> Result<Conversion, String> {
        let conversion = figma2json::convert_in_memory(bytes, &options.convert_options())
            .map_err(|error| error.to_string())?;
        let json = options.to_json(&conversion.json)?;
        drop(conversion.json);

        let raw_json = if options.raw {
            let raw = figma2json::convert_raw(bytes).map_err(|error| error.to_string())?;
            Some(options.to_json(&raw)?)
        } else {
            None
        };

        Ok(Conversion {
            json,
            raw_json,
            files: conversion.files,
        })
    }

    /// The files that go next to `canvas.json`, in archive order.
    pub fn files(&self) -> &[OutputFile] {
        &self.files
    }

    /// Package `canvas.json`, `canvas.raw.json` (with [`Options::raw`]) and
    /// the other files as a ZIP archive.
    ///
    /// # Errors
    /// Returns the error message when the archive cannot be written.
    pub fn zip(&self) -> Result<Vec<u8>, String> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let mut add = |path: &str, contents: &[u8]| -> zip::result::ZipResult<()> {
            let method = if is_compressed_image(contents) {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            };
            let options = FileOptions::default()
                .compression_method(method)
                .large_file(contents.len() > u32::MAX as usize);
            writer.start_file(path, options)?;
            writer.write_all(contents)?;
            Ok(())
        };

        let written = add(JSON_NAME, self.json.as_bytes())
            .and_then(|()| match &self.raw_json {
                Some(raw_json) => add(RAW_JSON_NAME, raw_json.as_bytes()),
                None => Ok(()),
            })
            .and_then(|()| {
                self.files
                    .iter()
                    .try_for_each(|file| add(&file.path, &file.contents))
            })
            .and_then(|()| writer.finish());
        written
            .map(Cursor::into_inner)
            .map_err(|error| format!("Failed to write ZIP archive: {error}"))
    }
}

#[wasm_bindgen]
impl Conversion {
    /// The converted document (`canvas.json`).
    #[wasm_bindgen(getter)]
    pub fn json(&self) -> String {
        self.json.clone()
    }

    /// The untransformed document (`canvas.raw.json`), when
    /// [`Options::raw`] was set.
    #[wasm_bindgen(getter, js_name = rawJson)]
    pub fn raw_json(&self) -> Option<String> {
        self.raw_json.clone()
    }

    /// Paths of the files that go next to `canvas.json`, such as
    /// `images/<hash>.png`.
    #[wasm_bindgen(getter, js_name = fileNames)]
    pub fn file_names(&self) -> Vec<String> {
        self.files.iter().map(|file| file.path.clone()).collect()
    }

    /// Contents of the file at `index` in [`Conversion::file_names`].
    #[wasm_bindgen(js_name = fileContents)]
    pub fn file_contents(&self, index: usize) -> Option<Vec<u8>> {
        self.files.get(index).map(|file| file.contents.clone())
    }

    /// Number of files under `images/`.
    #[wasm_bindgen(getter, js_name = imageCount)]
    pub fn image_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.path.starts_with("images/"))
            .count()
    }

    /// The ZIP archive built by [`Conversion::zip`].
    ///
    /// # Errors
    /// Throws when the archive cannot be written.
    #[wasm_bindgen(js_name = toZip)]
    pub fn to_zip(&self) -> Result<Vec<u8>, JsError> {
        self.zip().map_err(|error| JsError::new(&error))
    }
}

/// Convert the bytes of a `.fig` file or of its ZIP container.
///
/// # Errors
/// Throws an `Error` with a description of the problem when the file cannot be
/// decoded or converted.
#[wasm_bindgen]
pub fn convert(bytes: &[u8], options: &Options) -> Result<Conversion, JsError> {
    Conversion::new(bytes, options).map_err(|error| JsError::new(&error))
}

/// The version of figma2json this module was built from.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

fn is_compressed_image(contents: &[u8]) -> bool {
    matches!(
        figma2json::schema::image_extension(contents),
        Some(".png" | ".jpg" | ".gif" | ".webp")
    )
}
