use thiserror::Error;

/// Errors returned while reading and converting a `.fig` file.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum FigError {
    #[error("Invalid magic header: expected 'fig-kiwi' or 'fig-jam.', found {0:?}")]
    InvalidMagicHeader(Vec<u8>),

    #[error("File too small: expected at least {expected} bytes, found {actual}")]
    FileTooSmall { expected: usize, actual: usize },

    #[error("Incomplete chunk at offset {offset}: expected {expected} bytes, found {actual}")]
    IncompleteChunk {
        offset: usize,
        expected: usize,
        actual: usize,
    },

    #[error("Not enough chunks: expected at least {expected}, found {actual}")]
    NotEnoughChunks { expected: usize, actual: usize },

    #[error("Canvas file not found in ZIP archive")]
    CanvasNotFoundInZip,

    #[error("Decompression failed: {0}")]
    Decompression(String),

    #[error("Kiwi decoding failed: {0}")]
    Decode(String),

    #[error("Invalid document: {0}")]
    InvalidDocument(String),

    #[error("Invalid blob: {0}")]
    InvalidBlob(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
}

pub type Result<T> = std::result::Result<T, FigError>;
