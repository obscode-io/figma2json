//! Reading the .fig container: ZIP packaging, magic header, length-prefixed
//! chunks and their compression.

pub mod chunks;
pub mod compression;
pub mod header;

pub use chunks::{
    extract_chunks, extract_from_zip, extract_zip_to_directory, extract_zip_to_memory,
};
pub use compression::decompress_chunk;
pub use header::{detect_file_type, is_zip_container};
