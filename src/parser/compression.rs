use crate::error::{FigError, Result};
use flate2::read::DeflateDecoder;
use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use ruzstd::decoding::StreamingDecoder;
use std::io::{self, Read};

/// PNG magic signature (first two bytes: 137, 80)
const PNG_MAGIC: &[u8; 2] = &[137, 80];

/// JPEG magic signature (first two bytes: 255, 216)
const JPEG_MAGIC: &[u8; 2] = &[255, 216];

/// Check if data is already compressed (PNG or JPEG image)
///
/// Images are already compressed, so we should not attempt to decompress them.
///
/// # Arguments
/// * `bytes` - Data to check
///
/// # Returns
/// * `true` - If data starts with PNG or JPEG magic bytes
/// * `false` - Otherwise
///
/// # Examples
/// ```
/// use figma2json::parser::compression::is_already_compressed;
///
/// // PNG image
/// let png_data = &[137, 80, 78, 71, 13, 10, 26, 10];
/// assert!(is_already_compressed(png_data));
///
/// // JPEG image
/// let jpeg_data = &[255, 216, 255, 224];
/// assert!(is_already_compressed(jpeg_data));
///
/// // Regular compressed data
/// let other_data = &[120, 156, 1, 2, 3];
/// assert!(!is_already_compressed(other_data));
/// ```
pub fn is_already_compressed(bytes: &[u8]) -> bool {
    bytes.starts_with(PNG_MAGIC) || bytes.starts_with(JPEG_MAGIC)
}

/// Decompress chunk data using DEFLATE or Zstandard
///
/// Figma uses two compression formats:
/// - raw DEFLATE (no zlib header) - used in older files
/// - Zstandard - used in newer files
///
/// This function tries DEFLATE first, then falls back to Zstandard if DEFLATE fails.
/// If the data is already compressed (PNG/JPEG), it returns the data as-is.
///
/// # Arguments
/// * `bytes` - Compressed chunk data
///
/// # Returns
/// * `Ok(Vec<u8>)` - Decompressed data
/// * `Err(FigError::Decompression)` - If both decompression methods fail
///
/// # Examples
/// ```
/// use figma2json::parser::compression::decompress_chunk;
///
/// let compressed = zstd::encode_all(&b"fig-kiwi"[..], 3).unwrap();
/// assert_eq!(decompress_chunk(&compressed).unwrap(), b"fig-kiwi");
/// ```
pub fn decompress_chunk(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_already_compressed(bytes) {
        return Ok(bytes.to_vec());
    }

    decompress_deflate(bytes)
        .or_else(|_| decompress_zstd(bytes))
        .map_err(|e| {
            FigError::Decompression(format!(
                "Failed to decompress chunk (tried both DEFLATE and Zstandard): {e}"
            ))
        })
}

/// Decompress data using DEFLATE (raw, without zlib wrapper)
fn decompress_deflate(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut decompressed = Vec::new();
    DeflateDecoder::new(bytes).read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

fn decompress_zstd(mut bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut decompressed = Vec::new();
    while !bytes.is_empty() {
        match StreamingDecoder::new(&mut bytes) {
            Ok(mut decoder) => {
                decoder.read_to_end(&mut decompressed)?;
            }
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                length,
                ..
            })) => {
                bytes = usize::try_from(length)
                    .ok()
                    .and_then(|length| bytes.get(length..))
                    .ok_or_else(|| invalid_zstd("Skippable frame is truncated"))?;
            }
            Err(FrameDecoderError::ReadFrameHeaderError(
                ReadFrameHeaderError::MagicNumberReadError(_)
                | ReadFrameHeaderError::BadMagicNumber(_),
            )) => return Err(invalid_zstd("Unknown frame descriptor")),
            Err(FrameDecoderError::ReadFrameHeaderError(error)) => {
                return Err(invalid_zstd(&error.to_string()))
            }
            Err(error) => return Err(io::Error::other(error)),
        }
    }
    Ok(decompressed)
}

fn invalid_zstd(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn test_is_already_compressed_png() {
        let png_data = vec![137, 80, 78, 71, 13, 10, 26, 10];
        assert!(is_already_compressed(&png_data));
    }

    #[test]
    fn test_is_already_compressed_jpeg() {
        let jpeg_data = vec![255, 216, 255, 224, 0, 16];
        assert!(is_already_compressed(&jpeg_data));
    }

    #[test]
    fn test_is_not_compressed() {
        let data = vec![120, 156, 1, 2, 3, 4, 5];
        assert!(!is_already_compressed(&data));
    }

    #[test]
    fn test_is_not_compressed_too_small() {
        let data = vec![137];
        assert!(!is_already_compressed(&data));
    }

    #[test]
    fn test_decompress_deflate() {
        let original = b"Hello, Figma! This is a test string for compression.";

        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_chunk(&compressed).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn test_decompress_zstd() {
        let original = b"Hello, Figma! This is a test string for Zstandard compression.";

        let compressed = zstd::encode_all(&original[..], 3).unwrap();

        let decompressed = decompress_chunk(&compressed).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn test_decompress_already_compressed_png() {
        let png_data = vec![137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3, 4];
        let result = decompress_chunk(&png_data).unwrap();
        assert_eq!(result, png_data);
    }

    #[test]
    fn test_decompress_already_compressed_jpeg() {
        let jpeg_data = vec![255, 216, 255, 224, 0, 16, 1, 2, 3];
        let result = decompress_chunk(&jpeg_data).unwrap();
        assert_eq!(result, jpeg_data);
    }

    #[test]
    fn test_decompress_invalid_data() {
        let invalid_data = vec![1, 2, 3, 4, 5];
        let result = decompress_chunk(&invalid_data);
        assert!(result.is_err());
    }

    fn zstd_with_checksum(data: &[u8], level: i32) -> Vec<u8> {
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), level).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn sample_data(len: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_u32;
        (0..len)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                if i % 7 == 0 {
                    (state % 251) as u8
                } else {
                    b"fig-kiwi nodeChanges"[i % 20]
                }
            })
            .collect()
    }

    #[test]
    fn test_decompress_zstd_matches_reference_decoder() {
        for len in [0, 1, 1000, 300_000, 2_500_000] {
            let original = sample_data(len);
            for level in [1, 3, 19] {
                let compressed = zstd::encode_all(&original[..], level).unwrap();
                assert_eq!(decompress_zstd(&compressed).unwrap(), original);
                let compressed = zstd_with_checksum(&original, level);
                assert_eq!(decompress_zstd(&compressed).unwrap(), original);
            }
        }
    }

    #[test]
    fn test_decompress_zstd_concatenated_frames() {
        let mut compressed = zstd::encode_all(&b"first frame, "[..], 3).unwrap();
        compressed.extend(zstd::encode_all(&b"second frame"[..], 3).unwrap());
        assert_eq!(
            decompress_chunk(&compressed).unwrap(),
            zstd::decode_all(&compressed[..]).unwrap()
        );
    }

    #[test]
    fn test_decompress_zstd_skips_skippable_frames() {
        let mut compressed = 0x184D_2A50_u32.to_le_bytes().to_vec();
        compressed.extend(3u32.to_le_bytes());
        compressed.extend(b"abc");
        compressed.extend(zstd::encode_all(&b"payload"[..], 3).unwrap());
        assert_eq!(zstd::decode_all(&compressed[..]).unwrap(), b"payload");
        assert_eq!(decompress_zstd(&compressed).unwrap(), b"payload");

        compressed.truncate(10);
        assert!(decompress_zstd(&compressed).is_err());
    }

    #[test]
    fn test_decompress_error_names_both_formats() {
        let error = decompress_chunk(b"ab").unwrap_err().to_string();
        assert!(
            error.ends_with(
                "Failed to decompress chunk (tried both DEFLATE and Zstandard): Unknown frame descriptor"
            ),
            "{error}"
        );
    }

    #[test]
    fn test_decompress_truncated_zstd_fails() {
        let compressed = zstd::encode_all(&sample_data(10_000)[..], 3).unwrap();
        assert!(decompress_chunk(&compressed[..compressed.len() / 2]).is_err());
    }
}
