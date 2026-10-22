use crate::error::Result;
use serde_json::{Map, Value as JsonValue};

/// Simplify enum objects to simple string values
///
/// Recursively traverses the JSON tree and simplifies enum objects by converting
/// verbose enum format to simple strings:
/// - FROM: `{"__enum__": "BlendMode", "value": "NORMAL"}`
/// - TO: `"NORMAL"`
///
/// This applies to all enum types in the Figma format including:
/// NodeType, BlendMode, PaintType, StrokeAlign, StrokeJoin, NodePhase,
/// WindingRule, TextAlignVertical, TextAutoResize, LineType, FontStyle,
/// EmojiImageSet, ImageScaleMode, Directionality, DocumentColorProfile, etc.
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully simplified all enum objects
///
/// # Examples
/// ```no_run
/// use figma2json::schema::simplify_enums;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "type": {
///         "__enum__": "NodeType",
///         "value": "FRAME"
///     },
///     "blendMode": {
///         "__enum__": "BlendMode",
///         "value": "NORMAL"
///     }
/// });
/// simplify_enums(&mut tree).unwrap();
/// // tree now has "type": "FRAME" and "blendMode": "NORMAL"
/// ```
pub fn simplify_enums(tree: &mut JsonValue) -> Result<()> {
    simplify_children(tree);
    Ok(())
}

/// Simplify every enum held directly by `value` (an object's values or an
/// array's elements, e.g. `variableScopes`) and recurse into everything else.
///
/// `value` itself is never replaced, so an enum object at the root is kept.
fn simplify_children(value: &mut JsonValue) {
    match value {
        JsonValue::Object(map) => map.values_mut().for_each(simplify_value),
        JsonValue::Array(items) => items.iter_mut().for_each(simplify_value),
        _ => {}
    }
}

/// Replace `value` with its enum string if it is an enum object; otherwise
/// recurse into it. Replaced values are not recursed into.
fn simplify_value(value: &mut JsonValue) {
    let simplified = value
        .as_object()
        .filter(|obj| is_enum_object(obj))
        .and_then(extract_enum_value);
    match simplified {
        Some(enum_value) => *value = JsonValue::String(enum_value),
        None => simplify_children(value),
    }
}

/// Check if an object is an enum object (has __enum__ and value fields)
fn is_enum_object(obj: &Map<String, JsonValue>) -> bool {
    obj.contains_key("__enum__") && obj.contains_key("value")
}

/// Extract the value from an enum object
///
/// Returns `None` if the value field is missing or not a string.
fn extract_enum_value(obj: &Map<String, JsonValue>) -> Option<String> {
    obj.get("value")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_simplify_node_type() {
        let mut tree = json!({
            "name": "Frame",
            "type": {
                "__enum__": "NodeType",
                "value": "FRAME"
            }
        });

        simplify_enums(&mut tree).unwrap();

        assert_eq!(tree.get("type").unwrap().as_str(), Some("FRAME"));
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Frame"));
    }

    #[test]
    fn test_simplify_blend_mode() {
        let mut tree = json!({
            "blendMode": {
                "__enum__": "BlendMode",
                "value": "NORMAL"
            }
        });

        simplify_enums(&mut tree).unwrap();

        assert_eq!(tree.get("blendMode").unwrap().as_str(), Some("NORMAL"));
    }

    #[test]
    fn test_simplify_paint_type() {
        let mut tree = json!({
            "type": {
                "__enum__": "PaintType",
                "value": "SOLID"
            }
        });

        simplify_enums(&mut tree).unwrap();

        assert_eq!(tree.get("type").unwrap().as_str(), Some("SOLID"));
    }

    #[test]
    fn test_simplify_multiple_enums() {
        let mut tree = json!({
            "type": {
                "__enum__": "NodeType",
                "value": "ROUNDED_RECTANGLE"
            },
            "blendMode": {
                "__enum__": "BlendMode",
                "value": "NORMAL"
            },
            "strokeAlign": {
                "__enum__": "StrokeAlign",
                "value": "INSIDE"
            }
        });

        simplify_enums(&mut tree).unwrap();

        assert_eq!(
            tree.get("type").unwrap().as_str(),
            Some("ROUNDED_RECTANGLE")
        );
        assert_eq!(tree.get("blendMode").unwrap().as_str(), Some("NORMAL"));
        assert_eq!(tree.get("strokeAlign").unwrap().as_str(), Some("INSIDE"));
    }

    #[test]
    fn test_simplify_nested_enums() {
        let mut tree = json!({
            "name": "Root",
            "type": {
                "__enum__": "NodeType",
                "value": "DOCUMENT"
            },
            "children": [
                {
                    "name": "Child1",
                    "type": {
                        "__enum__": "NodeType",
                        "value": "FRAME"
                    }
                },
                {
                    "name": "Child2",
                    "phase": {
                        "__enum__": "NodePhase",
                        "value": "CREATED"
                    }
                }
            ]
        });

        simplify_enums(&mut tree).unwrap();

        // Root enum simplified
        assert_eq!(tree.get("type").unwrap().as_str(), Some("DOCUMENT"));

        // Children enums simplified
        assert_eq!(tree["children"][0]["type"].as_str(), Some("FRAME"));
        assert_eq!(tree["children"][1]["phase"].as_str(), Some("CREATED"));
    }

    #[test]
    fn test_simplify_deeply_nested_enums() {
        let mut tree = json!({
            "document": {
                "children": [
                    {
                        "fillPaints": [
                            {
                                "type": {
                                    "__enum__": "PaintType",
                                    "value": "IMAGE"
                                },
                                "blendMode": {
                                    "__enum__": "BlendMode",
                                    "value": "NORMAL"
                                }
                            }
                        ]
                    }
                ]
            }
        });

        simplify_enums(&mut tree).unwrap();

        // Deeply nested enums simplified
        let paint = &tree["document"]["children"][0]["fillPaints"][0];
        assert_eq!(paint["type"].as_str(), Some("IMAGE"));
        assert_eq!(paint["blendMode"].as_str(), Some("NORMAL"));
    }

    #[test]
    fn test_preserve_non_enum_objects() {
        let mut tree = json!({
            "name": "Rectangle",
            "transform": {
                "x": 100.0,
                "y": 200.0,
                "rotation": 0.0
            },
            "type": {
                "__enum__": "NodeType",
                "value": "FRAME"
            }
        });

        simplify_enums(&mut tree).unwrap();

        // Enum simplified
        assert_eq!(tree.get("type").unwrap().as_str(), Some("FRAME"));

        // Non-enum object preserved
        assert_eq!(tree["transform"]["x"].as_f64(), Some(100.0));
        assert_eq!(tree["transform"]["y"].as_f64(), Some(200.0));
        assert_eq!(tree["transform"]["rotation"].as_f64(), Some(0.0));
    }

    #[test]
    fn test_no_enums() {
        let mut tree = json!({
            "name": "Rectangle",
            "width": 100,
            "height": 200,
            "visible": true
        });

        simplify_enums(&mut tree).unwrap();

        // Tree without enums should be unchanged
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Rectangle"));
        assert_eq!(tree.get("width").unwrap().as_i64(), Some(100));
        assert_eq!(tree.get("height").unwrap().as_i64(), Some(200));
        assert_eq!(tree.get("visible").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_different_enum_types() {
        let mut tree = json!({
            "textAlignVertical": {
                "__enum__": "TextAlignVertical",
                "value": "TOP"
            },
            "textAutoResize": {
                "__enum__": "TextAutoResize",
                "value": "WIDTH_AND_HEIGHT"
            },
            "lineType": {
                "__enum__": "LineType",
                "value": "PLAIN"
            },
            "fontStyle": {
                "__enum__": "FontStyle",
                "value": "NORMAL"
            }
        });

        simplify_enums(&mut tree).unwrap();

        // All enum types simplified
        assert_eq!(tree.get("textAlignVertical").unwrap().as_str(), Some("TOP"));
        assert_eq!(
            tree.get("textAutoResize").unwrap().as_str(),
            Some("WIDTH_AND_HEIGHT")
        );
        assert_eq!(tree.get("lineType").unwrap().as_str(), Some("PLAIN"));
        assert_eq!(tree.get("fontStyle").unwrap().as_str(), Some("NORMAL"));
    }

    #[test]
    fn test_enum_in_array() {
        let mut tree = json!({
            "paints": [
                {
                    "type": {
                        "__enum__": "PaintType",
                        "value": "SOLID"
                    }
                },
                {
                    "type": {
                        "__enum__": "PaintType",
                        "value": "IMAGE"
                    }
                }
            ]
        });

        simplify_enums(&mut tree).unwrap();

        // All enums in array simplified
        assert_eq!(tree["paints"][0]["type"].as_str(), Some("SOLID"));
        assert_eq!(tree["paints"][1]["type"].as_str(), Some("IMAGE"));
    }

    #[test]
    fn test_is_enum_object() {
        let enum_obj = serde_json::from_value::<serde_json::Map<String, JsonValue>>(json!({
            "__enum__": "BlendMode",
            "value": "NORMAL"
        }))
        .unwrap();
        assert!(is_enum_object(&enum_obj));

        let not_enum = serde_json::from_value::<serde_json::Map<String, JsonValue>>(json!({
            "x": 10,
            "y": 20
        }))
        .unwrap();
        assert!(!is_enum_object(&not_enum));

        let incomplete = serde_json::from_value::<serde_json::Map<String, JsonValue>>(json!({
            "__enum__": "BlendMode"
        }))
        .unwrap();
        assert!(!is_enum_object(&incomplete));
    }

    #[test]
    fn test_extract_enum_value() {
        let enum_obj = serde_json::from_value::<serde_json::Map<String, JsonValue>>(json!({
            "__enum__": "BlendMode",
            "value": "NORMAL"
        }))
        .unwrap();

        let value = extract_enum_value(&enum_obj).unwrap();
        assert_eq!(value, "NORMAL");
    }

    #[test]
    fn test_simplify_enums_inside_arrays() {
        let mut tree = json!({
            "variableScopes": [
                {"__enum__": "VariableScope", "value": "FRAME_FILL"},
                {"__enum__": "VariableScope", "value": "TEXT_FILL"}
            ],
            "mixed": [{"__enum__": "OpenTypeFeature", "value": "LIGA"}, {"name": "x"}]
        });

        simplify_enums(&mut tree).unwrap();

        assert_eq!(tree["variableScopes"], json!(["FRAME_FILL", "TEXT_FILL"]));
        assert_eq!(tree["mixed"], json!(["LIGA", {"name": "x"}]));
    }
}
