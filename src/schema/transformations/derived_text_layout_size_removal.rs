use super::walk::for_each_object_mut;
use crate::error::Result;
use serde_json::Value as JsonValue;

/// Remove layoutSize field from derivedTextData objects
///
/// Recursively traverses the JSON tree and removes the "layoutSize" field from
/// "derivedTextData" objects. The layoutSize is redundant because it typically
/// matches the node's "size" field, so removing it reduces JSON size without
/// losing information.
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully removed all layoutSize fields from derivedTextData
///
/// # Examples
/// ```no_run
/// use figma2json::schema::remove_derived_text_layout_size;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "derivedTextData": {
///         "layoutSize": {"x": 100.0, "y": 50.0},
///         "otherInfo": "preserved"
///     },
///     "size": {"x": 100.0, "y": 50.0}
/// });
/// remove_derived_text_layout_size(&mut tree).unwrap();
/// // derivedTextData now has only "otherInfo"
/// ```
pub fn remove_derived_text_layout_size(tree: &mut JsonValue) -> Result<()> {
    for_each_object_mut(tree, &mut |map| {
        if let Some(JsonValue::Object(data)) = map.get_mut("derivedTextData") {
            data.remove("layoutSize");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_remove_layout_size() {
        let mut tree = json!({
            "name": "Text",
            "derivedTextData": {
                "layoutSize": {"x": 100.0, "y": 50.0},
                "otherInfo": "test"
            },
            "size": {"x": 100.0, "y": 50.0}
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        let derived_text_data = tree.get("derivedTextData").unwrap();
        assert!(derived_text_data.get("layoutSize").is_none());
        assert_eq!(
            derived_text_data.get("otherInfo").unwrap().as_str(),
            Some("test")
        );
        // size field should be preserved
        assert!(tree.get("size").is_some());
    }

    #[test]
    fn test_preserve_other_fields() {
        let mut tree = json!({
            "name": "Text",
            "derivedTextData": {
                "layoutSize": {"x": 200.0, "y": 100.0},
                "fontFamily": "Arial",
                "fontSize": 16.0
            }
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        let derived_text_data = tree.get("derivedTextData").unwrap();
        assert!(derived_text_data.get("layoutSize").is_none());
        assert_eq!(
            derived_text_data.get("fontFamily").unwrap().as_str(),
            Some("Arial")
        );
        assert_eq!(
            derived_text_data.get("fontSize").unwrap().as_f64(),
            Some(16.0)
        );
    }

    #[test]
    fn test_no_layout_size() {
        let mut tree = json!({
            "name": "Text",
            "derivedTextData": {
                "fontFamily": "Helvetica",
                "fontSize": 14.0
            }
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        let derived_text_data = tree.get("derivedTextData").unwrap();
        // derivedTextData without layoutSize should be unchanged
        assert!(derived_text_data.get("layoutSize").is_none());
        assert_eq!(
            derived_text_data.get("fontFamily").unwrap().as_str(),
            Some("Helvetica")
        );
    }

    #[test]
    fn test_no_derived_text_data() {
        let mut tree = json!({
            "name": "Rectangle",
            "width": 100,
            "height": 200
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        // Tree without derivedTextData should be unchanged
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Rectangle"));
        assert_eq!(tree.get("width").unwrap().as_i64(), Some(100));
        assert!(tree.get("derivedTextData").is_none());
    }

    #[test]
    fn test_nested_objects() {
        let mut tree = json!({
            "children": [
                {
                    "name": "Text1",
                    "derivedTextData": {
                        "layoutSize": {"x": 50.0, "y": 25.0},
                        "info1": "data1"
                    }
                },
                {
                    "name": "Text2",
                    "derivedTextData": {
                        "layoutSize": {"x": 60.0, "y": 30.0},
                        "info2": "data2"
                    }
                }
            ]
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        // Both layoutSize fields should be removed
        assert!(tree["children"][0]["derivedTextData"]
            .get("layoutSize")
            .is_none());
        assert_eq!(
            tree["children"][0]["derivedTextData"]["info1"].as_str(),
            Some("data1")
        );

        assert!(tree["children"][1]["derivedTextData"]
            .get("layoutSize")
            .is_none());
        assert_eq!(
            tree["children"][1]["derivedTextData"]["info2"].as_str(),
            Some("data2")
        );
    }

    #[test]
    fn test_deeply_nested() {
        let mut tree = json!({
            "document": {
                "children": [
                    {
                        "type": "TEXT",
                        "derivedTextData": {
                            "layoutSize": {"x": 300.0, "y": 150.0},
                            "characters": "Hello"
                        }
                    }
                ]
            }
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        let derived_text_data = &tree["document"]["children"][0]["derivedTextData"];
        assert!(derived_text_data.get("layoutSize").is_none());
        assert_eq!(derived_text_data["characters"].as_str(), Some("Hello"));
    }

    #[test]
    fn test_empty_derived_text_data() {
        let mut tree = json!({
            "name": "Text",
            "derivedTextData": {}
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        let derived_text_data = tree.get("derivedTextData").unwrap();
        // Empty derivedTextData should remain empty
        assert!(derived_text_data.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_layout_size_outside_derived_text_data() {
        let mut tree = json!({
            "name": "Node",
            "layoutSize": {"x": 100.0, "y": 100.0},
            "derivedTextData": {
                "layoutSize": {"x": 50.0, "y": 50.0}
            }
        });

        remove_derived_text_layout_size(&mut tree).unwrap();

        // layoutSize outside derivedTextData should be preserved
        assert!(tree.get("layoutSize").is_some());
        assert_eq!(tree["layoutSize"]["x"].as_f64(), Some(100.0));

        // layoutSize inside derivedTextData should be removed
        assert!(tree["derivedTextData"].get("layoutSize").is_none());
    }
}
