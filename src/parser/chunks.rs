use crate::error::{FigError, Result};
use crate::types::ParsedFile;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

/// Minimum file size for a valid .fig file (8 bytes header + 4 bytes version)
const MIN_FILE_SIZE: usize = 12;

/// Minimum chunk header size (4 bytes for length)
const CHUNK_HEADER_SIZE: usize = 4;

/// Name of the Kiwi-encoded document inside a ZIP-packaged .fig
const CANVAS_ENTRY: &str = "canvas.fig";

/// Extract canvas.fig from a ZIP archive
///
/// A .fig saved from Figma is a ZIP archive containing a `canvas.fig` file
/// (the document itself) plus its images. This function extracts that file.
///
/// # Arguments
/// * `bytes` - Raw ZIP file bytes
///
/// # Returns
/// * `Ok(Vec<u8>)` - Extracted canvas.fig file contents
/// * `Err(FigError::Zip)` / `Err(FigError::Io)` - If the archive cannot be read
/// * `Err(FigError::CanvasNotFoundInZip)` - If it has no `canvas.fig` entry
///
/// # Examples
/// ```no_run
/// use figma2json::parser::extract_from_zip;
///
/// let zip_bytes = std::fs::read("example.fig").unwrap();
/// let canvas_bytes = extract_from_zip(&zip_bytes).unwrap();
/// ```
pub fn extract_from_zip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    // First entry named "canvas.fig"
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if file.name() == CANVAS_ENTRY {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok(contents);
        }
    }

    Err(FigError::CanvasNotFoundInZip)
}

/// Extract entire ZIP archive to a directory
///
/// Extracts all files from a ZIP archive to the specified directory,
/// preserving the directory structure from the ZIP file. Entries whose path
/// would land outside `target_dir` (absolute paths, `..`) are skipped.
///
/// # Arguments
/// * `bytes` - Raw ZIP file bytes
/// * `target_dir` - Directory to extract files to (created if missing; existing
///   files with the same names are overwritten)
///
/// # Returns
/// * `Ok(())` - If extraction succeeds
/// * `Err(FigError::Zip)` - If the archive cannot be read
/// * `Err(FigError::Io)` - If a file or directory cannot be written
///
/// # Examples
/// ```no_run
/// use figma2json::parser::extract_zip_to_directory;
/// use std::path::Path;
///
/// let zip_bytes = std::fs::read("example.zip").unwrap();
/// extract_zip_to_directory(&zip_bytes, Path::new("output")).unwrap();
/// ```
pub fn extract_zip_to_directory(bytes: &[u8], target_dir: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    fs::create_dir_all(target_dir)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        // `enclosed_name` rejects paths that would escape `target_dir`
        let Some(output_path) = file.enclosed_name().map(|name| target_dir.join(name)) else {
            continue;
        };

        if file.is_dir() {
            fs::create_dir_all(&output_path)?;
        } else {
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output_file = fs::File::create(&output_path)?;
            io::copy(&mut file, &mut output_file)?;
        }
    }

    Ok(())
}

/// Extract chunks from a .fig file (version format)
///
/// Parses the version format used by Evan Wallace's approach:
/// ```text
/// [8 bytes] Magic header: "fig-kiwi" or "fig-jam."
/// [4 bytes] Version (uint32, little-endian)
/// [4 bytes] Chunk 0 length (uint32, little-endian)
/// [N bytes] Compressed chunk 0 (schema)
/// [4 bytes] Chunk 1 length (uint32, little-endian)
/// [N bytes] Compressed chunk 1 (data)
/// [...] Additional chunks (images, etc.)
/// ```
///
/// # Arguments
/// * `bytes` - Raw .fig file bytes, magic header included (the header itself is
///   not checked here; see [`detect_file_type`](crate::parser::detect_file_type))
///
/// # Returns
/// * `Ok(ParsedFile)` - Parsed file with version and chunks
/// * `Err(FigError::FileTooSmall)` - If the file is shorter than header + version
/// * `Err(FigError::IncompleteChunk)` - If a chunk is longer than the remaining data
/// * `Err(FigError::NotEnoughChunks)` - If there is no schema and data chunk
///
/// # Examples
/// ```no_run
/// use figma2json::parser::extract_chunks;
///
/// let bytes = std::fs::read("example.canvas.fig").unwrap();
/// let parsed = extract_chunks(&bytes).unwrap();
/// println!("Version: {}", parsed.version);
/// println!("Chunks: {}", parsed.chunks.len());
/// ```
pub fn extract_chunks(bytes: &[u8]) -> Result<ParsedFile> {
    // Version follows the 8-byte magic header (present iff the file has MIN_FILE_SIZE bytes)
    let version = read_u32_le(bytes, 8).ok_or(FigError::FileTooSmall {
        expected: MIN_FILE_SIZE,
        actual: bytes.len(),
    })?;

    // Chunks start at offset 12. Trailing bytes too short for a chunk header are ignored.
    let mut chunks = Vec::new();
    let mut offset = MIN_FILE_SIZE;

    while let Some(chunk_length) = read_u32_le(bytes, offset) {
        // u32 -> usize is lossless on every target with at least 32-bit pointers
        let chunk_length = chunk_length as usize;
        let start = offset + CHUNK_HEADER_SIZE;
        let chunk = bytes[start..]
            .get(..chunk_length)
            .ok_or(FigError::IncompleteChunk {
                offset,
                expected: chunk_length,
                actual: bytes.len() - start,
            })?;

        chunks.push(chunk.to_vec());
        offset = start + chunk_length;
    }

    // Validate we have at least 2 chunks (schema + data)
    if chunks.len() < 2 {
        return Err(FigError::NotEnoughChunks {
            expected: 2,
            actual: chunks.len(),
        });
    }

    Ok(ParsedFile::new(version, chunks))
}

/// Little-endian `u32` at `offset`, or `None` when fewer than 4 bytes remain.
fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let word = bytes.get(offset..end)?;
    Some(u32::from_le_bytes(word.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_chunks_minimal() {
        // Create a minimal valid .fig file structure
        let mut bytes = Vec::new();

        // Magic header
        bytes.extend_from_slice(b"fig-kiwi");

        // Version (48, little-endian)
        bytes.extend_from_slice(&48u32.to_le_bytes());

        // Chunk 0: length 5
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(b"chunk");

        // Chunk 1: length 4
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(b"data");

        let result = extract_chunks(&bytes).unwrap();
        assert_eq!(result.version, 48);
        assert_eq!(result.chunks.len(), 2);
        assert_eq!(result.chunks[0], b"chunk");
        assert_eq!(result.chunks[1], b"data");
    }

    #[test]
    fn test_extract_chunks_multiple() {
        // Test with multiple chunks (schema + data + images)
        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"fig-kiwi");
        bytes.extend_from_slice(&101u32.to_le_bytes());

        // Three chunks
        for i in 0..3 {
            let chunk_data = format!("chunk{}", i);
            bytes.extend_from_slice(&(chunk_data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(chunk_data.as_bytes());
        }

        let result = extract_chunks(&bytes).unwrap();
        assert_eq!(result.version, 101);
        assert_eq!(result.chunks.len(), 3);
    }

    #[test]
    fn test_extract_chunks_file_too_small() {
        let bytes = b"fig-kiwi\x00";
        let result = extract_chunks(bytes);
        assert!(result.is_err());
        match result {
            Err(FigError::FileTooSmall { .. }) => (),
            _ => panic!("Expected FileTooSmall error"),
        }
    }

    #[test]
    fn test_extract_chunks_incomplete_chunk() {
        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"fig-kiwi");
        bytes.extend_from_slice(&48u32.to_le_bytes());

        // Chunk with length 100 but only 5 bytes of data
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(b"short");

        let result = extract_chunks(&bytes);
        assert!(result.is_err());
        match result {
            Err(FigError::IncompleteChunk { .. }) => (),
            _ => panic!("Expected IncompleteChunk error"),
        }
    }

    #[test]
    fn test_extract_chunks_not_enough_chunks() {
        let mut bytes = Vec::new();

        bytes.extend_from_slice(b"fig-kiwi");
        bytes.extend_from_slice(&48u32.to_le_bytes());

        // Only one chunk (need at least 2)
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(b"chunk");

        let result = extract_chunks(&bytes);
        assert!(result.is_err());
        match result {
            Err(FigError::NotEnoughChunks { expected, actual }) => {
                assert_eq!(expected, 2);
                assert_eq!(actual, 1);
            }
            _ => panic!("Expected NotEnoughChunks error"),
        }
    }

    #[test]
    fn test_extract_chunks_reports_incomplete_chunk_position() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"fig-kiwi");
        bytes.extend_from_slice(&48u32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(b"short");

        match extract_chunks(&bytes) {
            Err(FigError::IncompleteChunk {
                offset,
                expected,
                actual,
            }) => {
                assert_eq!(offset, 12);
                assert_eq!(expected, u32::MAX as usize);
                assert_eq!(actual, 5);
            }
            other => panic!("Expected IncompleteChunk error, got {other:?}"),
        }
    }

    #[test]
    fn test_extract_chunks_ignores_trailing_partial_header() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"fig-kiwi");
        bytes.extend_from_slice(&48u32.to_le_bytes());
        for chunk in [&b"schema"[..], b"data"] {
            bytes.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            bytes.extend_from_slice(chunk);
        }
        bytes.extend_from_slice(&[1, 2, 3]);

        let parsed = extract_chunks(&bytes).unwrap();
        assert_eq!(parsed.chunks, vec![b"schema".to_vec(), b"data".to_vec()]);
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in entries {
            writer
                .start_file(*name, zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_extract_from_zip() {
        let zip = zip_with(&[("images/a", b"img"), ("canvas.fig", b"fig-kiwi")]);
        assert_eq!(extract_from_zip(&zip).unwrap(), b"fig-kiwi");

        let zip = zip_with(&[("other.fig", b"fig-kiwi")]);
        assert!(matches!(
            extract_from_zip(&zip),
            Err(FigError::CanvasNotFoundInZip)
        ));
    }

    #[test]
    fn test_extract_zip_to_directory_skips_escaping_paths() {
        let root =
            std::env::temp_dir().join(format!("figma2json-extract-test-{}", std::process::id()));
        let target = root.join("out");
        let zip = zip_with(&[
            ("canvas.fig", b"fig-kiwi"),
            ("images/a", b"img"),
            ("../escaped", b"no"),
        ]);

        extract_zip_to_directory(&zip, &target).unwrap();

        assert_eq!(fs::read(target.join("canvas.fig")).unwrap(), b"fig-kiwi");
        assert_eq!(fs::read(target.join("images/a")).unwrap(), b"img");
        assert!(!root.join("escaped").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
