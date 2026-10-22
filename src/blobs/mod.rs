//! Blob handling: binary payloads (path commands, vector networks, ...) that
//! nodes reference by index through `*Blob` fields.

pub mod parser;
pub mod substitution;

use crate::error::Result;
use base64::{engine::general_purpose, Engine as _};
use serde_json::Value as JsonValue;

// Re-export commonly used items
pub use parser::{parse_blob, parse_commands, parse_vector_network};
pub use substitution::substitute_blobs;

/// Process blobs array by encoding binary data to base64
///
/// Takes the blobs array from decoded Kiwi data and converts any `bytes` field
/// that is still an array of numbers to a base64-encoded string. The Kiwi
/// decoder already emits `bytes` as base64, so for decoded files this leaves the
/// blobs unchanged.
///
/// # Arguments
/// * `blobs` - Array of blob objects from decoded Kiwi data
///
/// # Returns
/// * `Ok(JsonValue)` - A JSON array of the processed blobs, in the same order
/// * `Err(FigError::InvalidBlob)` - If a `bytes` array holds a value that is not a byte
///
/// # Examples
/// ```
/// use figma2json::blobs::process_blobs;
/// use serde_json::json;
///
/// let blobs = vec![json!({"bytes": [72, 105]})];
/// let processed = process_blobs(blobs).unwrap();
/// assert_eq!(processed, json!([{"bytes": "SGk="}]));
/// ```
pub fn process_blobs(blobs: Vec<JsonValue>) -> Result<JsonValue> {
    blobs
        .into_iter()
        .map(|mut blob| {
            if let Some(bytes) = blob.get_mut("bytes") {
                if let JsonValue::Array(values) = bytes {
                    let encoded =
                        general_purpose::STANDARD.encode(parser::bytes_from_json(values)?);
                    *bytes = JsonValue::String(encoded);
                }
            }
            Ok(blob)
        })
        .collect::<Result<Vec<_>>>()
        .map(JsonValue::Array)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_process_blobs_with_bytes() {
        let blobs = vec![json!({
            "id": 1,
            "bytes": [72, 101, 108, 108, 111]  // "Hello" in ASCII
        })];

        let processed = process_blobs(blobs).unwrap();
        let blobs_array = processed.as_array().unwrap();

        assert_eq!(blobs_array.len(), 1);
        let blob = &blobs_array[0];

        // Check that bytes field is now a base64 string
        let bytes_value = blob.get("bytes").unwrap();
        assert!(bytes_value.is_string());
        assert_eq!(bytes_value.as_str().unwrap(), "SGVsbG8="); // "Hello" in base64
    }

    #[test]
    fn test_process_blobs_without_bytes() {
        let blobs = vec![json!({
            "id": 1,
            "type": "IMAGE"
        })];

        let processed = process_blobs(blobs).unwrap();
        let blobs_array = processed.as_array().unwrap();

        assert_eq!(blobs_array.len(), 1);
        assert_eq!(blobs_array[0].get("id").unwrap(), 1);
    }

    #[test]
    fn test_process_empty_blobs() {
        let blobs = vec![];
        let processed = process_blobs(blobs).unwrap();
        let blobs_array = processed.as_array().unwrap();
        assert_eq!(blobs_array.len(), 0);
    }

    #[test]
    fn test_process_multiple_blobs() {
        let blobs = vec![
            json!({
                "id": 1,
                "bytes": [65, 66, 67]  // "ABC"
            }),
            json!({
                "id": 2,
                "bytes": [88, 89, 90]  // "XYZ"
            }),
        ];

        let processed = process_blobs(blobs).unwrap();
        let blobs_array = processed.as_array().unwrap();

        assert_eq!(blobs_array.len(), 2);
        assert_eq!(
            blobs_array[0].get("bytes").unwrap().as_str().unwrap(),
            "QUJD"
        ); // "ABC" in base64
        assert_eq!(
            blobs_array[1].get("bytes").unwrap().as_str().unwrap(),
            "WFla"
        ); // "XYZ" in base64
    }

    #[test]
    fn test_process_blobs_rejects_non_byte_values() {
        for bad in [json!(256), json!(-1), json!("A")] {
            let blobs = vec![json!({ "bytes": [65, bad] })];
            assert!(matches!(
                process_blobs(blobs),
                Err(crate::error::FigError::InvalidBlob(_))
            ));
        }
    }
}
