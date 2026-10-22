//! # figma2json
//!
//! Convert Figma `.fig` files (and the ZIP containers Figma saves them in) to
//! JSON that is compact enough to hand to an LLM or a code generator.
//!
//! The usual entry point is [`convert`] (or [`convert_with_options`]), which
//! decodes the file and runs the transformation passes that simplify Figma's
//! internal representation into CSS-friendly values. [`convert_raw`] stops after
//! decoding and returns the full, untransformed node tree.
//!
//! ## Example
//!
//! ```no_run
//! use std::path::Path;
//! use figma2json::{convert_with_options, ConvertOptions};
//!
//! let bytes = std::fs::read("example.fig")?;
//! let options = ConvertOptions { keep_internal: true, ..ConvertOptions::default() };
//! let json = convert_with_options(&bytes, Some(Path::new("output-dir")), &options)?;
//! println!("{}", serde_json::to_string_pretty(&json)?);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The lower-level stages are available in [`parser`] (container, header and
//! chunk handling), [`schema`] (Kiwi decoding, tree building and the individual
//! transformation passes) and [`blobs`].

#![forbid(unsafe_code)]

pub mod blobs;
pub mod error;
pub mod parser;
pub mod schema;
pub mod types;

mod pipeline;

pub use error::{FigError, Result};
pub use pipeline::{
    convert, convert_in_memory, convert_raw, convert_with_options, ConvertOptions,
    InMemoryConversion, OutputFile,
};
pub use types::{FileType, ParsedFile};
