use crate::error::{FigError, Result};
use base64::{engine::general_purpose, Engine as _};
use kiwi_schema::{
    ByteBuffer, DefKind, Field, Schema, Value, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT,
    TYPE_INT64, TYPE_STRING, TYPE_UINT, TYPE_UINT64,
};
use serde_json::Value as JsonValue;

/// Decode .fig file data to JSON
///
/// Takes decompressed schema and data chunks and decodes them using the Kiwi schema format.
///
/// # Arguments
/// * `schema_bytes` - Decompressed schema chunk (chunk 0)
/// * `data_bytes` - Decompressed data chunk (chunk 1)
///
/// # Returns
/// * `Ok(JsonValue)` - Decoded JSON data
/// * `Err(FigError)` - If decoding fails
///
/// # Examples
/// ```no_run
/// use figma2json::schema::decode_fig_to_json;
///
/// let schema_bytes = vec![/* decompressed schema */];
/// let data_bytes = vec![/* decompressed data */];
/// let json = decode_fig_to_json(&schema_bytes, &data_bytes).unwrap();
/// ```
pub fn decode_fig_to_json(schema_bytes: &[u8], data_bytes: &[u8]) -> Result<JsonValue> {
    // 1. Decode the binary schema
    let schema = Schema::decode(schema_bytes)
        .map_err(|_| FigError::Decode("Failed to decode Kiwi binary schema".to_string()))?;

    // 2. Find the root message type
    // In Figma .fig files, the root message is named "Message" and contains nodeChanges and blobs
    let root_type_id = schema
        .defs
        .iter()
        .find(|def| {
            def.name == "Message"
                && def.fields.iter().any(|f| f.name == "nodeChanges")
                && def.fields.iter().any(|f| f.name == "blobs")
        })
        .map(|def| def.index)
        .ok_or_else(|| {
            FigError::Decode("No root Message definition found in schema".to_string())
        })?;

    // 3. Decode the message data straight into JSON.
    //
    // This walks the Kiwi schema directly instead of building kiwi's own `Value`
    // tree first and converting it. Large files carry tens of megabytes of blob
    // bytes; as `Value::Byte` items (and then as one JSON number per byte) they
    // used several GB of memory. Byte arrays in a `bytes` field are emitted as a
    // base64 string here, which is what `blobs::process_blobs` produced anyway.
    let mut bb = ByteBuffer::new(data_bytes);
    decode_type(&schema, root_type_id, &mut bb)
        .map_err(|_| FigError::Decode("Failed to decode message data".to_string()))
}

fn float_to_json(f: f32) -> JsonValue {
    if f.is_nan() || f.is_infinite() {
        JsonValue::Null
    } else {
        JsonValue::Number(
            serde_json::Number::from_f64(f as f64).unwrap_or_else(|| serde_json::Number::from(0)),
        )
    }
}

/// Decode one value of `type_id` (mirrors `kiwi_schema::Value::decode_bb`).
fn decode_type(
    schema: &Schema,
    type_id: i32,
    bb: &mut ByteBuffer,
) -> std::result::Result<JsonValue, ()> {
    Ok(match type_id {
        TYPE_BOOL => JsonValue::Bool(bb.read_bool()?),
        TYPE_BYTE => JsonValue::Number(bb.read_byte()?.into()),
        TYPE_INT => JsonValue::Number(bb.read_var_int()?.into()),
        TYPE_UINT => JsonValue::Number(bb.read_var_uint()?.into()),
        TYPE_FLOAT => float_to_json(bb.read_var_float()?),
        TYPE_STRING => JsonValue::String(bb.read_string()?.into_owned()),
        TYPE_INT64 => JsonValue::Number(bb.read_var_int64()?.into()),
        TYPE_UINT64 => JsonValue::Number(bb.read_var_uint64()?.into()),
        _ => {
            let def = schema.defs.get(type_id as usize).ok_or(())?;
            match def.kind {
                DefKind::Enum => {
                    let index = def
                        .field_value_to_index
                        .get(&bb.read_var_uint()?)
                        .ok_or(())?;
                    let mut map = serde_json::Map::new();
                    map.insert("__enum__".to_string(), JsonValue::String(def.name.clone()));
                    map.insert(
                        "value".to_string(),
                        JsonValue::String(def.fields[*index].name.clone()),
                    );
                    JsonValue::Object(map)
                }
                DefKind::Struct => {
                    let mut map = serde_json::Map::new();
                    for field in &def.fields {
                        map.insert(field.name.clone(), decode_field(schema, field, bb)?);
                    }
                    JsonValue::Object(map)
                }
                DefKind::Message => {
                    let mut map = serde_json::Map::new();
                    loop {
                        let value = bb.read_var_uint()?;
                        if value == 0 {
                            break;
                        }
                        let index = def.field_value_to_index.get(&value).ok_or(())?;
                        let field = &def.fields[*index];
                        map.insert(field.name.clone(), decode_field(schema, field, bb)?);
                    }
                    JsonValue::Object(map)
                }
            }
        }
    })
}

/// Decode a field, handling arrays (mirrors `kiwi_schema::Value::decode_field_bb`).
fn decode_field(
    schema: &Schema,
    field: &Field,
    bb: &mut ByteBuffer,
) -> std::result::Result<JsonValue, ()> {
    if !field.is_array {
        return decode_type(schema, field.type_id, bb);
    }
    let len = bb.read_var_uint()? as usize;
    if field.type_id == TYPE_BYTE {
        let bytes = bb.read_bytes(len)?;
        if field.name == "bytes" {
            return Ok(JsonValue::String(general_purpose::STANDARD.encode(bytes)));
        }
        return Ok(JsonValue::Array(
            bytes
                .iter()
                .map(|b| JsonValue::Number((*b).into()))
                .collect(),
        ));
    }
    // Cap the pre-allocation: `len` comes from the file and may be corrupt.
    let mut array = Vec::with_capacity(len.min(1 << 16));
    for _ in 0..len {
        array.push(decode_type(schema, field.type_id, bb)?);
    }
    Ok(JsonValue::Array(array))
}

/// Convert a Kiwi Value to serde_json Value (kept for API compatibility and tests)
#[allow(dead_code)]
fn kiwi_value_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Byte(n) => JsonValue::Number((*n).into()),
        Value::Int(n) => JsonValue::Number((*n).into()),
        Value::UInt(n) => JsonValue::Number((*n).into()),
        Value::Float(f) => {
            // Handle special float values
            if f.is_nan() || f.is_infinite() {
                JsonValue::Null
            } else {
                JsonValue::Number(
                    serde_json::Number::from_f64(*f as f64)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                )
            }
        }
        Value::String(s) => JsonValue::String(s.clone()),
        Value::Int64(n) => JsonValue::Number((*n).into()),
        Value::UInt64(n) => JsonValue::Number((*n).into()),
        Value::Array(arr) => JsonValue::Array(arr.iter().map(kiwi_value_to_json).collect()),
        Value::Enum(enum_name, variant_name) => {
            // Represent enum as object with type field
            let mut map = serde_json::Map::new();
            map.insert(
                "__enum__".to_string(),
                JsonValue::String(enum_name.to_string()),
            );
            map.insert(
                "value".to_string(),
                JsonValue::String(variant_name.to_string()),
            );
            JsonValue::Object(map)
        }
        Value::Object(_type_name, fields) => {
            // Convert to JSON object
            let mut map = serde_json::Map::new();

            for (field_name, field_value) in fields {
                map.insert(field_name.to_string(), kiwi_value_to_json(field_value));
            }

            JsonValue::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_kiwi_value_to_json_bool() {
        let value = Value::Bool(true);
        let json = kiwi_value_to_json(&value);
        assert_eq!(json, JsonValue::Bool(true));
    }

    #[test]
    fn test_kiwi_value_to_json_int() {
        let value = Value::Int(42);
        let json = kiwi_value_to_json(&value);
        assert_eq!(json, JsonValue::Number(42.into()));
    }

    #[test]
    fn test_kiwi_value_to_json_string() {
        let value = Value::String("hello".to_string());
        let json = kiwi_value_to_json(&value);
        assert_eq!(json, JsonValue::String("hello".to_string()));
    }

    #[test]
    fn test_kiwi_value_to_json_array() {
        let value = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let json = kiwi_value_to_json(&value);
        assert_eq!(
            json,
            JsonValue::Array(vec![
                JsonValue::Number(1.into()),
                JsonValue::Number(2.into()),
                JsonValue::Number(3.into()),
            ])
        );
    }

    #[test]
    fn test_kiwi_value_to_json_object() {
        let mut fields = HashMap::new();
        fields.insert("x", Value::Int(10));
        fields.insert("y", Value::Int(20));

        let value = Value::Object("Point", fields);
        let json = kiwi_value_to_json(&value);

        match json {
            JsonValue::Object(map) => {
                assert_eq!(map.get("x"), Some(&JsonValue::Number(10.into())));
                assert_eq!(map.get("y"), Some(&JsonValue::Number(20.into())));
            }
            _ => panic!("Expected JSON object"),
        }
    }
}
