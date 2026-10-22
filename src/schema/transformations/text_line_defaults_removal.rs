use super::walk::for_each_object_mut;
use crate::error::Result;
use serde_json::{Map, Value as JsonValue};

/// A Figma default value for a text line field.
#[derive(Clone, Copy)]
enum LineDefault {
    Int(i64),
    Bool(bool),
    Str(&'static str),
}

impl LineDefault {
    fn matches(self, value: &JsonValue) -> bool {
        match self {
            Self::Int(n) => value.as_i64() == Some(n),
            Self::Bool(b) => value.as_bool() == Some(b),
            Self::Str(s) => value.as_str() == Some(s),
        }
    }
}

/// Line fields removed when they hold their default value.
const LINE_DEFAULTS: [(&str, LineDefault); 6] = [
    ("indentationLevel", LineDefault::Int(0)),
    ("isFirstLineOfList", LineDefault::Bool(false)),
    ("lineType", LineDefault::Str("PLAIN")),
    ("listStartOffset", LineDefault::Int(0)),
    ("sourceDirectionality", LineDefault::Str("AUTO")),
    ("styleId", LineDefault::Int(0)),
];

/// Remove default text line properties from all objects in the JSON tree
///
/// Recursively traverses the JSON tree and removes line properties within
/// `textData.lines` and `textValue.lines` arrays that have default values:
/// - "indentationLevel" with value 0 (no indentation)
/// - "isFirstLineOfList" with value false (not a list item)
/// - "lineType" with value "PLAIN" (plain text)
/// - "listStartOffset" with value 0 (no list offset)
/// - "sourceDirectionality" with value "AUTO" (automatic text direction)
/// - "styleId" with value 0 (no style applied)
///
/// If all line objects in a `lines` array become empty after removing defaults,
/// the entire `lines` array is removed.
///
/// These are the default values in Figma for plain text rendering, so omitting
/// them reduces output size without losing information for HTML/CSS conversion.
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully removed all default text line fields
///
/// # Examples
/// ```no_run
/// use figma2json::schema::remove_default_text_line_properties;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "textData": {
///         "characters": "Hello",
///         "lines": [
///             {
///                 "indentationLevel": 0,
///                 "isFirstLineOfList": false,
///                 "lineType": "PLAIN",
///                 "listStartOffset": 0,
///                 "sourceDirectionality": "AUTO",
///                 "styleId": 0
///             }
///         ]
///     }
/// });
/// remove_default_text_line_properties(&mut tree).unwrap();
/// // The entire "lines" array is removed because all values were defaults
/// ```
pub fn remove_default_text_line_properties(tree: &mut JsonValue) -> Result<()> {
    for_each_object_mut(tree, &mut |map| {
        let Some(JsonValue::Array(lines)) = map.get_mut("lines") else {
            return;
        };
        for line in lines.iter_mut() {
            if let JsonValue::Object(line_obj) = line {
                remove_default_line_fields(line_obj);
            }
        }
        let all_empty = !lines.is_empty()
            && lines
                .iter()
                .all(|line| matches!(line, JsonValue::Object(obj) if obj.is_empty()));
        if all_empty {
            map.remove("lines");
        }
    });
    Ok(())
}

/// Remove default-valued fields from a single line object
fn remove_default_line_fields(line_obj: &mut Map<String, JsonValue>) {
    for (field, default) in LINE_DEFAULTS {
        if line_obj
            .get(field)
            .is_some_and(|value| default.matches(value))
        {
            line_obj.remove(field);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_remove_all_defaults_removes_lines_array() {
        let mut tree = json!({
            "textData": {
                "characters": "Hello",
                "lines": [
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        assert!(tree["textData"].get("lines").is_none());
        assert_eq!(tree["textData"]["characters"].as_str(), Some("Hello"));
    }

    #[test]
    fn test_preserve_non_default_indentation_level() {
        let mut tree = json!({
            "textData": {
                "characters": "Indented text",
                "lines": [
                    {
                        "indentationLevel": 2,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        assert!(tree["textData"].get("lines").is_some());
        let lines = tree["textData"]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["indentationLevel"].as_i64(), Some(2));
        assert!(lines[0].get("isFirstLineOfList").is_none());
        assert!(lines[0].get("lineType").is_none());
        assert!(lines[0].get("listStartOffset").is_none());
        assert!(lines[0].get("sourceDirectionality").is_none());
        assert!(lines[0].get("styleId").is_none());
    }

    #[test]
    fn test_preserve_list_item() {
        let mut tree = json!({
            "textData": {
                "characters": "• List item",
                "lines": [
                    {
                        "indentationLevel": 1,
                        "isFirstLineOfList": true,
                        "lineType": "UNORDERED_LIST",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        let lines = tree["textData"]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["indentationLevel"].as_i64(), Some(1));
        assert_eq!(lines[0]["isFirstLineOfList"].as_bool(), Some(true));
        assert_eq!(lines[0]["lineType"].as_str(), Some("UNORDERED_LIST"));
        assert!(lines[0].get("listStartOffset").is_none());
        assert!(lines[0].get("sourceDirectionality").is_none());
        assert!(lines[0].get("styleId").is_none());
    }

    #[test]
    fn test_preserve_non_zero_style_id() {
        let mut tree = json!({
            "textData": {
                "characters": "Styled text",
                "lines": [
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 5
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        let lines = tree["textData"]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["styleId"].as_i64(), Some(5));
        assert!(lines[0].get("indentationLevel").is_none());
        assert!(lines[0].get("isFirstLineOfList").is_none());
        assert!(lines[0].get("lineType").is_none());
        assert!(lines[0].get("listStartOffset").is_none());
        assert!(lines[0].get("sourceDirectionality").is_none());
    }

    #[test]
    fn test_multiple_lines_mixed() {
        let mut tree = json!({
            "textData": {
                "characters": "Multi-line text",
                "lines": [
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    },
                    {
                        "indentationLevel": 1,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        let lines = tree["textData"]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].as_object().unwrap().is_empty());
        assert_eq!(lines[1]["indentationLevel"].as_i64(), Some(1));
        assert!(lines[1].get("isFirstLineOfList").is_none());
    }

    #[test]
    fn test_nested_text_data() {
        let mut tree = json!({
            "children": [
                {
                    "textData": {
                        "characters": "First",
                        "lines": [
                            {
                                "indentationLevel": 0,
                                "isFirstLineOfList": false,
                                "lineType": "PLAIN",
                                "listStartOffset": 0,
                                "sourceDirectionality": "AUTO",
                                "styleId": 0
                            }
                        ]
                    }
                },
                {
                    "textData": {
                        "characters": "Second",
                        "lines": [
                            {
                                "indentationLevel": 1,
                                "isFirstLineOfList": false,
                                "lineType": "PLAIN",
                                "listStartOffset": 0,
                                "sourceDirectionality": "AUTO",
                                "styleId": 0
                            }
                        ]
                    }
                }
            ]
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        assert!(tree["children"][0]["textData"].get("lines").is_none());

        assert!(tree["children"][1]["textData"].get("lines").is_some());
        let lines = tree["children"][1]["textData"]["lines"].as_array().unwrap();
        assert_eq!(lines[0]["indentationLevel"].as_i64(), Some(1));
    }

    #[test]
    fn test_text_value_lines() {
        let mut tree = json!({
            "textValue": {
                "characters": "Hello",
                "lines": [
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        assert!(tree["textValue"].get("lines").is_none());
    }

    #[test]
    fn test_no_lines_field() {
        let mut tree = json!({
            "textData": {
                "characters": "Hello"
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        assert_eq!(tree["textData"]["characters"].as_str(), Some("Hello"));
    }

    #[test]
    fn test_preserve_non_default_list_start_offset() {
        let mut tree = json!({
            "textData": {
                "characters": "Numbered list",
                "lines": [
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 5,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        let lines = tree["textData"]["lines"].as_array().unwrap();
        assert_eq!(lines[0]["listStartOffset"].as_i64(), Some(5));
        assert!(lines[0].get("indentationLevel").is_none());
        assert!(lines[0].get("isFirstLineOfList").is_none());
        assert!(lines[0].get("lineType").is_none());
        assert!(lines[0].get("sourceDirectionality").is_none());
        assert!(lines[0].get("styleId").is_none());
    }

    #[test]
    fn test_deeply_nested_structure() {
        let mut tree = json!({
            "document": {
                "children": [
                    {
                        "type": "TEXT",
                        "textData": {
                            "characters": "Deep text",
                            "lines": [
                                {
                                    "indentationLevel": 0,
                                    "isFirstLineOfList": false,
                                    "lineType": "PLAIN",
                                    "listStartOffset": 0,
                                    "sourceDirectionality": "AUTO",
                                    "styleId": 0
                                }
                            ]
                        }
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        let text_data = &tree["document"]["children"][0]["textData"];
        assert!(text_data.get("lines").is_none());
        assert_eq!(text_data["characters"].as_str(), Some("Deep text"));
    }

    #[test]
    fn test_empty_lines_array() {
        let mut tree = json!({
            "textData": {
                "characters": "Hello",
                "lines": []
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        assert!(tree["textData"].get("lines").is_some());
        assert_eq!(tree["textData"]["lines"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_multiple_all_default_lines() {
        let mut tree = json!({
            "textData": {
                "characters": "Multi-line",
                "lines": [
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    },
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    },
                    {
                        "indentationLevel": 0,
                        "isFirstLineOfList": false,
                        "lineType": "PLAIN",
                        "listStartOffset": 0,
                        "sourceDirectionality": "AUTO",
                        "styleId": 0
                    }
                ]
            }
        });

        remove_default_text_line_properties(&mut tree).unwrap();

        assert!(tree["textData"].get("lines").is_none());
    }
}
