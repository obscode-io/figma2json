use super::walk::for_each_object_mut;
use crate::error::Result;
use serde_json::Value as JsonValue;

/// Remove empty postscript field from fontName objects
///
/// Recursively traverses the JSON tree and removes the "postscript" field from
/// "fontName" objects when it is an empty string. The postscript field is used
/// to specify the PostScript font name, but when empty it provides no information
/// and can be safely removed to reduce JSON size.
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully removed all empty postscript fields
///
/// # Examples
/// ```no_run
/// use figma2json::schema::remove_empty_font_postscript;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "fontName": {
///         "family": "Inter",
///         "style": "Regular",
///         "postscript": ""
///     }
/// });
/// remove_empty_font_postscript(&mut tree).unwrap();
/// // fontName now has only "family" and "style" fields
/// ```
pub fn remove_empty_font_postscript(tree: &mut JsonValue) -> Result<()> {
    for_each_object_mut(tree, &mut |map| {
        if let Some(JsonValue::Object(font)) = map.get_mut("fontName") {
            if font.get("postscript").and_then(JsonValue::as_str) == Some("") {
                font.remove("postscript");
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_remove_empty_postscript() {
        let mut tree = json!({
            "name": "Text",
            "fontName": {
                "family": "Inter",
                "style": "Regular",
                "postscript": ""
            }
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        let font_name = tree.get("fontName").unwrap();
        assert!(font_name.get("postscript").is_none());
        assert_eq!(font_name.get("family").unwrap().as_str(), Some("Inter"));
        assert_eq!(font_name.get("style").unwrap().as_str(), Some("Regular"));
    }

    #[test]
    fn test_preserve_non_empty_postscript() {
        let mut tree = json!({
            "name": "Text",
            "fontName": {
                "family": "Helvetica",
                "style": "Bold",
                "postscript": "Helvetica-Bold"
            }
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        let font_name = tree.get("fontName").unwrap();
        assert_eq!(
            font_name.get("postscript").unwrap().as_str(),
            Some("Helvetica-Bold")
        );
        assert_eq!(font_name.get("family").unwrap().as_str(), Some("Helvetica"));
        assert_eq!(font_name.get("style").unwrap().as_str(), Some("Bold"));
    }

    #[test]
    fn test_no_postscript_field() {
        let mut tree = json!({
            "name": "Text",
            "fontName": {
                "family": "Arial",
                "style": "Regular"
            }
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        let font_name = tree.get("fontName").unwrap();
        assert!(font_name.get("postscript").is_none());
        assert_eq!(font_name.get("family").unwrap().as_str(), Some("Arial"));
        assert_eq!(font_name.get("style").unwrap().as_str(), Some("Regular"));
    }

    #[test]
    fn test_no_font_name() {
        let mut tree = json!({
            "name": "Rectangle",
            "width": 100,
            "height": 200
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        assert_eq!(tree.get("name").unwrap().as_str(), Some("Rectangle"));
        assert_eq!(tree.get("width").unwrap().as_i64(), Some(100));
        assert!(tree.get("fontName").is_none());
    }

    #[test]
    fn test_nested_objects() {
        let mut tree = json!({
            "children": [
                {
                    "name": "Text1",
                    "fontName": {
                        "family": "Inter",
                        "postscript": ""
                    }
                },
                {
                    "name": "Text2",
                    "fontName": {
                        "family": "Roboto",
                        "postscript": ""
                    }
                }
            ]
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        assert!(tree["children"][0]["fontName"].get("postscript").is_none());
        assert_eq!(
            tree["children"][0]["fontName"]["family"].as_str(),
            Some("Inter")
        );
        assert!(tree["children"][1]["fontName"].get("postscript").is_none());
        assert_eq!(
            tree["children"][1]["fontName"]["family"].as_str(),
            Some("Roboto")
        );
    }

    #[test]
    fn test_mixed_empty_and_non_empty() {
        let mut tree = json!({
            "children": [
                {
                    "name": "Text1",
                    "fontName": {
                        "family": "Inter",
                        "postscript": ""
                    }
                },
                {
                    "name": "Text2",
                    "fontName": {
                        "family": "Helvetica",
                        "postscript": "Helvetica-Bold"
                    }
                }
            ]
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        assert!(tree["children"][0]["fontName"].get("postscript").is_none());
        assert_eq!(
            tree["children"][1]["fontName"]["postscript"].as_str(),
            Some("Helvetica-Bold")
        );
    }

    #[test]
    fn test_deeply_nested() {
        let mut tree = json!({
            "document": {
                "children": [
                    {
                        "type": "TEXT",
                        "fontName": {
                            "family": "Times",
                            "style": "Italic",
                            "postscript": ""
                        }
                    }
                ]
            }
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        let font_name = &tree["document"]["children"][0]["fontName"];
        assert!(font_name.get("postscript").is_none());
        assert_eq!(font_name["family"].as_str(), Some("Times"));
        assert_eq!(font_name["style"].as_str(), Some("Italic"));
    }

    #[test]
    fn test_postscript_non_string() {
        let mut tree = json!({
            "fontName": {
                "family": "Test",
                "postscript": 123
            }
        });

        remove_empty_font_postscript(&mut tree).unwrap();

        let font_name = tree.get("fontName").unwrap();
        assert!(font_name.get("postscript").is_some());
        assert_eq!(font_name.get("postscript").unwrap().as_i64(), Some(123));
    }
}
