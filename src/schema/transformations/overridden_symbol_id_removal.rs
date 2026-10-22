use crate::error::Result;
use serde_json::{Map, Value as JsonValue};

/// Remove standalone overriddenSymbolID objects from arrays
///
/// Recursively traverses the JSON tree and filters out objects that contain
/// ONLY an `overriddenSymbolID` field (which itself contains only `localID` and `sessionID`).
///
/// These are Figma component swap metadata objects that appear in arrays like
/// `symbolOverrides`. They indicate that a nested component was swapped but don't
/// provide any visual rendering information needed for HTML/CSS conversion.
///
/// Objects that have `overriddenSymbolID` along with other fields (like `textData`,
/// `visible`, etc.) are preserved, as the other fields contain rendering information.
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully removed all standalone overriddenSymbolID objects
///
/// # Examples
/// ```no_run
/// use figma2json::schema::remove_overridden_symbol_id;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "symbolOverrides": [
///         {
///             "overriddenSymbolID": {
///                 "localID": 123,
///                 "sessionID": 456
///             }
///         },
///         {
///             "overriddenSymbolID": {
///                 "localID": 789,
///                 "sessionID": 12
///             },
///             "textData": {
///                 "characters": "Hello"
///             }
///         }
///     ]
/// });
/// remove_overridden_symbol_id(&mut tree).unwrap();
/// // First object removed, second preserved (has textData)
/// ```
pub fn remove_overridden_symbol_id(tree: &mut JsonValue) -> Result<()> {
    remove_standalone(tree);
    Ok(())
}

/// Check if an object contains ONLY an overriddenSymbolID field
/// with only localID and sessionID
fn is_standalone_overridden_symbol_id(obj: &Map<String, JsonValue>) -> bool {
    if obj.len() != 1 {
        return false;
    }
    obj.get("overriddenSymbolID")
        .and_then(JsonValue::as_object)
        .is_some_and(|inner| {
            inner.len() == 2 && inner.contains_key("localID") && inner.contains_key("sessionID")
        })
}

/// Filter standalone overriddenSymbolID objects out of every array, then recurse
/// into the remaining elements.
fn remove_standalone(value: &mut JsonValue) {
    match value {
        JsonValue::Object(map) => map.values_mut().for_each(remove_standalone),
        JsonValue::Array(items) => {
            items.retain(|item| {
                !item
                    .as_object()
                    .is_some_and(is_standalone_overridden_symbol_id)
            });
            items.iter_mut().for_each(remove_standalone);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_remove_standalone_overridden_symbol_id() {
        let mut tree = json!({
            "symbolOverrides": [
                {
                    "overriddenSymbolID": {
                        "localID": 7979,
                        "sessionID": 8184
                    }
                },
                {
                    "textData": {
                        "characters": "Roles"
                    }
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree.get("symbolOverrides").unwrap().as_array().unwrap();
        assert_eq!(overrides.len(), 1);
        assert!(overrides[0].get("textData").is_some());
    }

    #[test]
    fn test_preserve_overridden_symbol_id_with_other_fields() {
        let mut tree = json!({
            "symbolOverrides": [
                {
                    "overriddenSymbolID": {
                        "localID": 441,
                        "sessionID": 56
                    },
                    "visible": false
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree.get("symbolOverrides").unwrap().as_array().unwrap();
        // Object preserved because it has other fields
        assert_eq!(overrides.len(), 1);
        assert!(overrides[0].get("overriddenSymbolID").is_some());
        assert_eq!(overrides[0].get("visible").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_remove_multiple_standalone_objects() {
        let mut tree = json!({
            "symbolOverrides": [
                {
                    "overriddenSymbolID": {
                        "localID": 1,
                        "sessionID": 2
                    }
                },
                {
                    "textData": {
                        "characters": "Keep"
                    }
                },
                {
                    "overriddenSymbolID": {
                        "localID": 3,
                        "sessionID": 4
                    }
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree.get("symbolOverrides").unwrap().as_array().unwrap();
        assert_eq!(overrides.len(), 1);
        assert!(overrides[0].get("textData").is_some());
    }

    #[test]
    fn test_all_standalone_removed() {
        let mut tree = json!({
            "symbolOverrides": [
                {
                    "overriddenSymbolID": {
                        "localID": 1,
                        "sessionID": 2
                    }
                },
                {
                    "overriddenSymbolID": {
                        "localID": 3,
                        "sessionID": 4
                    }
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree.get("symbolOverrides").unwrap().as_array().unwrap();
        // All standalone objects removed, empty array
        assert_eq!(overrides.len(), 0);
    }

    #[test]
    fn test_no_standalone_objects() {
        let mut tree = json!({
            "symbolOverrides": [
                {
                    "textData": {
                        "characters": "One"
                    }
                },
                {
                    "visible": true
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree.get("symbolOverrides").unwrap().as_array().unwrap();
        // All objects preserved
        assert_eq!(overrides.len(), 2);
    }

    #[test]
    fn test_nested_arrays() {
        let mut tree = json!({
            "parent": {
                "symbolOverrides": [
                    {
                        "overriddenSymbolID": {
                            "localID": 123,
                            "sessionID": 456
                        }
                    },
                    {
                        "textData": {
                            "characters": "Text"
                        }
                    }
                ]
            }
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree["parent"]["symbolOverrides"].as_array().unwrap();
        assert_eq!(overrides.len(), 1);
        assert!(overrides[0].get("textData").is_some());
    }

    #[test]
    fn test_deeply_nested_structure() {
        let mut tree = json!({
            "document": {
                "children": [
                    {
                        "symbolData": {
                            "symbolOverrides": [
                                {
                                    "overriddenSymbolID": {
                                        "localID": 789,
                                        "sessionID": 12
                                    }
                                },
                                {
                                    "opacity": 0.5
                                }
                            ]
                        }
                    }
                ]
            }
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree["document"]["children"][0]["symbolData"]["symbolOverrides"]
            .as_array()
            .unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].get("opacity").unwrap().as_f64(), Some(0.5));
    }

    #[test]
    fn test_overridden_symbol_id_with_extra_fields() {
        // If overriddenSymbolID object has extra fields beyond localID/sessionID,
        // the entire object should be preserved
        let mut tree = json!({
            "symbolOverrides": [
                {
                    "overriddenSymbolID": {
                        "localID": 123,
                        "sessionID": 456,
                        "extraField": "value"
                    }
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree.get("symbolOverrides").unwrap().as_array().unwrap();
        // Preserved because overriddenSymbolID has extra fields
        assert_eq!(overrides.len(), 1);
    }

    #[test]
    fn test_overridden_symbol_id_missing_session_id() {
        // If overriddenSymbolID is missing sessionID, preserve the object
        let mut tree = json!({
            "symbolOverrides": [
                {
                    "overriddenSymbolID": {
                        "localID": 123
                    }
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree.get("symbolOverrides").unwrap().as_array().unwrap();
        // Preserved because it's malformed
        assert_eq!(overrides.len(), 1);
    }

    #[test]
    fn test_non_object_array_elements() {
        let mut tree = json!({
            "data": [1, 2, 3, "string", null, true]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let data = tree.get("data").unwrap().as_array().unwrap();
        // Non-object elements should be preserved
        assert_eq!(data.len(), 6);
    }

    #[test]
    fn test_mixed_array_with_primitives() {
        let mut tree = json!({
            "mixed": [
                {
                    "overriddenSymbolID": {
                        "localID": 1,
                        "sessionID": 2
                    }
                },
                "string",
                42,
                {
                    "name": "Keep"
                }
            ]
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let mixed = tree.get("mixed").unwrap().as_array().unwrap();
        assert_eq!(mixed.len(), 3);
        assert_eq!(mixed[0].as_str(), Some("string"));
        assert_eq!(mixed[1].as_i64(), Some(42));
        assert!(mixed[2].get("name").is_some());
    }

    #[test]
    fn test_real_world_example() {
        // Based on actual data from archives/roles-members.json
        let mut tree = json!({
            "symbolData": {
                "symbolOverrides": [
                    {
                        "textData": {
                            "characters": "Roles"
                        }
                    },
                    {
                        "overriddenSymbolID": {
                            "localID": 7974,
                            "sessionID": 8184
                        }
                    },
                    {
                        "textData": {
                            "characters": "Members"
                        }
                    },
                    {
                        "overriddenSymbolID": {
                            "localID": 7979,
                            "sessionID": 8184
                        }
                    },
                    {
                        "textData": {
                            "characters": "Audit"
                        }
                    },
                    {
                        "visible": false
                    },
                    {
                        "overrideLevel": 1,
                        "textData": {
                            "characters": "Commands"
                        }
                    }
                ]
            }
        });

        remove_overridden_symbol_id(&mut tree).unwrap();

        let overrides = tree["symbolData"]["symbolOverrides"].as_array().unwrap();
        // Should remove only the 2 standalone overriddenSymbolID objects
        assert_eq!(overrides.len(), 5);

        // Verify the kept objects
        assert!(overrides[0].get("textData").is_some());
        assert_eq!(
            overrides[0]["textData"]["characters"].as_str(),
            Some("Roles")
        );

        assert!(overrides[1].get("textData").is_some());
        assert_eq!(
            overrides[1]["textData"]["characters"].as_str(),
            Some("Members")
        );

        assert!(overrides[2].get("textData").is_some());
        assert_eq!(
            overrides[2]["textData"]["characters"].as_str(),
            Some("Audit")
        );

        assert_eq!(overrides[3].get("visible").unwrap().as_bool(), Some(false));

        assert!(overrides[4].get("overrideLevel").is_some());
        assert!(overrides[4].get("textData").is_some());
    }
}
