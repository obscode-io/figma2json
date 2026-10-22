use crate::error::{FigError, Result};
use base64::{engine::general_purpose, Engine as _};
use serde_json::Value as JsonValue;

/// Parse a blob based on its type
///
/// Takes a blob type (field name without "Blob" suffix) and the blob object,
/// extracts the bytes, and parses them into structured JSON data.
///
/// # Arguments
/// * `blob_type` - The type of blob (e.g., "commands", "vectorNetwork")
/// * `blob` - The blob object containing bytes (may be base64 string or byte array)
///
/// # Returns
/// * `Ok(Some(JsonValue))` - Successfully parsed blob data
/// * `Ok(None)` - Unknown blob type or unparseable data
/// * `Err(FigError::InvalidBlob)` - If the blob of a known type has no readable `bytes`
pub fn parse_blob(blob_type: &str, blob: &JsonValue) -> Result<Option<JsonValue>> {
    let parse: fn(&[u8]) -> Option<JsonValue> = match blob_type {
        "commands" => parse_commands,
        "vectorNetwork" => parse_vector_network,
        _ => return Ok(None), // Unknown blob type
    };
    Ok(parse(&extract_blob_bytes(blob)?))
}

/// Extract bytes from a blob object
///
/// Blobs can be stored as:
/// - Base64 string in "bytes" field
/// - Array of numbers in "bytes" field
fn extract_blob_bytes(blob: &JsonValue) -> Result<Vec<u8>> {
    let bytes_value = blob
        .get("bytes")
        .ok_or_else(|| FigError::InvalidBlob("Blob missing bytes field".to_string()))?;

    match bytes_value {
        JsonValue::String(base64_str) => general_purpose::STANDARD
            .decode(base64_str)
            .map_err(|e| FigError::InvalidBlob(format!("Failed to decode base64: {e}"))),
        JsonValue::Array(bytes_array) => bytes_from_json(bytes_array),
        _ => Err(FigError::InvalidBlob(
            "Blob bytes field is neither string nor array".to_string(),
        )),
    }
}

/// Convert a JSON array of byte values (`0..=255`) to bytes.
///
/// A value that is not a byte is an error rather than being silently truncated.
pub(crate) fn bytes_from_json(values: &[JsonValue]) -> Result<Vec<u8>> {
    values
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| FigError::InvalidBlob(format!("Blob byte out of range: {value}")))
        })
        .collect()
}

/// Little-endian reader over a byte slice. Every read returns `None` once the
/// data runs out, so a truncated blob ends parsing instead of panicking.
struct ByteReader<'a> {
    bytes: &'a [u8],
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn remaining(&self) -> usize {
        self.bytes.len()
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let head: [u8; N] = self.bytes.get(..N)?.try_into().ok()?;
        self.bytes = &self.bytes[N..];
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take().map(|[byte]: [u8; 1]| byte)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    /// A `u32` count, as `usize`.
    fn count(&mut self) -> Option<usize> {
        self.u32().and_then(|n| usize::try_from(n).ok())
    }

    fn f32(&mut self) -> Option<f32> {
        self.take().map(f32::from_le_bytes)
    }
}

/// Parse binary path commands into JSON array
///
/// Converts binary path command data into a flat JSON array in the format:
/// `["M", x, y, "L", x, y, "Q", cx, cy, x, y, "C", cx1, cy1, cx2, cy2, x, y, "Z"]`
///
/// Command types:
/// - 0: Z (close path, no coordinates)
/// - 1: M (move to, 2 floats: x, y)
/// - 2: L (line to, 2 floats: x, y)
/// - 3: Q (quadratic curve, 4 floats: cx, cy, x, y)
/// - 4: C (cubic curve, 6 floats: cx1, cy1, cx2, cy2, x, y)
///
/// All coordinates are stored as little-endian f32 values.
///
/// # Arguments
/// * `bytes` - Binary command data
///
/// # Returns
/// * `Some(JsonValue)` - Array of commands and coordinates
/// * `None` - If data is invalid or incomplete
pub fn parse_commands(bytes: &[u8]) -> Option<JsonValue> {
    let mut reader = ByteReader::new(bytes);
    let mut commands = Vec::new();

    while !reader.is_empty() {
        let (command, coordinates) = match reader.u8()? {
            0 => ("Z", 0),
            1 => ("M", 2),
            2 => ("L", 2),
            3 => ("Q", 4),
            4 => ("C", 6),
            _ => return None, // Unknown command type
        };
        commands.push(JsonValue::String(command.to_string()));
        for _ in 0..coordinates {
            commands.push(json_number(reader.f32()?));
        }
    }

    Some(JsonValue::Array(commands))
}

/// Parse binary vector network into JSON object
///
/// Converts binary vector network data into a structured JSON object:
/// ```json
/// {
///   "vertices": [{"styleID": 0, "x": 1.0, "y": 2.0}, ...],
///   "segments": [{"styleID": 0, "start": {...}, "end": {...}}, ...],
///   "regions": [{"styleID": 0, "windingRule": "ODD", "loops": [...]}, ...]
/// }
/// ```
///
/// Binary format:
/// - Header: vertexCount (u32), segmentCount (u32), regionCount (u32)
/// - Vertices: styleID (u32), x (f32), y (f32) - repeated vertexCount times
/// - Segments: styleID (u32), startVertex (u32), start.dx (f32), start.dy (f32),
///   endVertex (u32), end.dx (f32), end.dy (f32) - repeated segmentCount times
/// - Regions: styleID+windingRule (u32), loopCount (u32),
///   then for each loop: indexCount (u32), indices (u32[]) - repeated regionCount times
///
/// All values are little-endian.
///
/// # Arguments
/// * `bytes` - Binary vector network data
///
/// # Returns
/// * `Some(JsonValue)` - Object with vertices, segments, and regions
/// * `None` - If data is invalid or incomplete
pub fn parse_vector_network(bytes: &[u8]) -> Option<JsonValue> {
    let mut reader = ByteReader::new(bytes);

    let vertex_count = reader.count()?;
    let segment_count = reader.count()?;
    let region_count = reader.count()?;

    // The counts come from the file, so the vectors grow as data is actually read
    // instead of being allocated up front from them.
    let mut vertices = Vec::new();
    for _ in 0..vertex_count {
        let style_id = reader.u32()?;
        let x = reader.f32()?;
        let y = reader.f32()?;
        vertices.push(serde_json::json!({
            "styleID": style_id,
            "x": json_number(x),
            "y": json_number(y)
        }));
    }

    let is_index_below = |index: u32, len: usize| usize::try_from(index).is_ok_and(|i| i < len);

    let mut segments = Vec::new();
    for _ in 0..segment_count {
        let style_id = reader.u32()?;
        let start_vertex = reader.u32()?;
        let start_dx = reader.f32()?;
        let start_dy = reader.f32()?;
        let end_vertex = reader.u32()?;
        let end_dx = reader.f32()?;
        let end_dy = reader.f32()?;

        if !is_index_below(start_vertex, vertex_count) || !is_index_below(end_vertex, vertex_count)
        {
            return None;
        }

        segments.push(serde_json::json!({
            "styleID": style_id,
            "start": {
                "vertex": start_vertex,
                "dx": json_number(start_dx),
                "dy": json_number(start_dy)
            },
            "end": {
                "vertex": end_vertex,
                "dx": json_number(end_dx),
                "dy": json_number(end_dy)
            }
        }));
    }

    let mut regions = Vec::new();
    for _ in 0..region_count {
        // styleID and winding rule are packed into one u32
        let style_and_rule = reader.u32()?;
        let winding_rule = if style_and_rule & 1 != 0 {
            "NONZERO"
        } else {
            "ODD"
        };
        let style_id = style_and_rule >> 1;

        let loop_count = reader.count()?;
        let mut loops = Vec::new();
        for _ in 0..loop_count {
            let index_count = reader.count()?;
            // Reject a count larger than the remaining data before reading any index
            if index_count > reader.remaining() / 4 {
                return None;
            }
            let mut indices = Vec::with_capacity(index_count);
            for _ in 0..index_count {
                let segment_index = reader.u32()?;
                if !is_index_below(segment_index, segment_count) {
                    return None;
                }
                indices.push(JsonValue::from(segment_index));
            }
            loops.push(serde_json::json!({ "segments": indices }));
        }

        regions.push(serde_json::json!({
            "styleID": style_id,
            "windingRule": winding_rule,
            "loops": loops
        }));
    }

    Some(serde_json::json!({
        "vertices": vertices,
        "segments": segments,
        "regions": regions
    }))
}

/// Convert f32 to a JSON number; NaN and infinities (not valid in JSON) become `null`.
fn json_number(value: f32) -> JsonValue {
    serde_json::Number::from_f64(f64::from(value)).map_or(JsonValue::Null, JsonValue::Number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_commands_simple_path() {
        // M 10 20 L 30 40 Z
        let mut bytes = Vec::new();
        bytes.push(1); // M
        bytes.extend_from_slice(&10.0f32.to_le_bytes());
        bytes.extend_from_slice(&20.0f32.to_le_bytes());
        bytes.push(2); // L
        bytes.extend_from_slice(&30.0f32.to_le_bytes());
        bytes.extend_from_slice(&40.0f32.to_le_bytes());
        bytes.push(0); // Z

        let result = parse_commands(&bytes).unwrap();
        let arr = result.as_array().unwrap();

        assert_eq!(arr.len(), 7);
        assert_eq!(arr[0].as_str(), Some("M"));
        assert_eq!(arr[1].as_f64(), Some(10.0));
        assert_eq!(arr[2].as_f64(), Some(20.0));
        assert_eq!(arr[3].as_str(), Some("L"));
        assert_eq!(arr[4].as_f64(), Some(30.0));
        assert_eq!(arr[5].as_f64(), Some(40.0));
        assert_eq!(arr[6].as_str(), Some("Z"));
    }

    #[test]
    fn test_parse_commands_quadratic() {
        // Q 1 2 3 4
        let mut bytes = Vec::new();
        bytes.push(3); // Q
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&3.0f32.to_le_bytes());
        bytes.extend_from_slice(&4.0f32.to_le_bytes());

        let result = parse_commands(&bytes).unwrap();
        let arr = result.as_array().unwrap();

        assert_eq!(arr.len(), 5);
        assert_eq!(arr[0].as_str(), Some("Q"));
        assert_eq!(arr[1].as_f64(), Some(1.0));
        assert_eq!(arr[2].as_f64(), Some(2.0));
        assert_eq!(arr[3].as_f64(), Some(3.0));
        assert_eq!(arr[4].as_f64(), Some(4.0));
    }

    #[test]
    fn test_parse_commands_cubic() {
        // C 1 2 3 4 5 6
        let mut bytes = Vec::new();
        bytes.push(4); // C
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&3.0f32.to_le_bytes());
        bytes.extend_from_slice(&4.0f32.to_le_bytes());
        bytes.extend_from_slice(&5.0f32.to_le_bytes());
        bytes.extend_from_slice(&6.0f32.to_le_bytes());

        let result = parse_commands(&bytes).unwrap();
        let arr = result.as_array().unwrap();

        assert_eq!(arr.len(), 7);
        assert_eq!(arr[0].as_str(), Some("C"));
        assert_eq!(arr[1].as_f64(), Some(1.0));
        assert_eq!(arr[2].as_f64(), Some(2.0));
        assert_eq!(arr[3].as_f64(), Some(3.0));
        assert_eq!(arr[4].as_f64(), Some(4.0));
        assert_eq!(arr[5].as_f64(), Some(5.0));
        assert_eq!(arr[6].as_f64(), Some(6.0));
    }

    #[test]
    fn test_parse_commands_invalid() {
        // Invalid command type
        let bytes = vec![99];
        assert!(parse_commands(&bytes).is_none());

        // Incomplete data
        let bytes = vec![1, 0]; // M with incomplete coordinates
        assert!(parse_commands(&bytes).is_none());
    }

    #[test]
    fn test_parse_vector_network_simple() {
        let mut bytes = Vec::new();

        // Header: 2 vertices, 1 segment, 0 regions
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        // Vertex 0: styleID=0, x=10, y=20
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&10.0f32.to_le_bytes());
        bytes.extend_from_slice(&20.0f32.to_le_bytes());

        // Vertex 1: styleID=0, x=30, y=40
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&30.0f32.to_le_bytes());
        bytes.extend_from_slice(&40.0f32.to_le_bytes());

        // Segment 0: styleID=0, start=(vertex=0, dx=0, dy=0), end=(vertex=1, dx=0, dy=0)
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());

        let result = parse_vector_network(&bytes).unwrap();

        assert!(result.get("vertices").is_some());
        assert!(result.get("segments").is_some());
        assert!(result.get("regions").is_some());

        let vertices = result.get("vertices").unwrap().as_array().unwrap();
        assert_eq!(vertices.len(), 2);

        let segments = result.get("segments").unwrap().as_array().unwrap();
        assert_eq!(segments.len(), 1);

        let regions = result.get("regions").unwrap().as_array().unwrap();
        assert_eq!(regions.len(), 0);
    }
}
