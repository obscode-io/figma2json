use super::walk::for_each_object_mut;
use crate::error::Result;
use serde_json::{Map, Value as JsonValue};

/// Remove geometry-related fields from all objects in the JSON tree
///
/// Recursively traverses the JSON tree and removes geometry fields:
/// - "fillGeometry" - Path commands for fill shapes (M, L, Q, Z, etc.)
/// - "strokeGeometry" - Path commands for stroke shapes
/// - "windingRule" - SVG winding rule property
/// - "styleID" - Internal style reference
///
/// These fields contain detailed path geometry that is overkill for simple
/// shapes in HTML/CSS rendering.
///
/// **Exception**: Geometry is preserved for icons and images, which are identified by:
/// - Having exportSettings with imageType (SVG/PNG) in symbolData.symbolOverrides
/// - Having node names starting with "icon/" or "arrows/"
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully removed all geometry fields (except for icons/images)
///
/// # Examples
/// ```no_run
/// use figma2json::schema::remove_geometry_fields;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "name": "Rectangle",
///     "fillGeometry": [
///         {
///             "commands": ["M", 0.0, 0.0, "L", 100.0, 0.0, "Z"],
///             "styleID": 0,
///             "windingRule": {
///                 "__enum__": "WindingRule",
///                 "value": "NONZERO"
///             }
///         }
///     ],
///     "size": {"x": 100.0, "y": 100.0}
/// });
/// remove_geometry_fields(&mut tree).unwrap();
/// // tree now has only "name" and "size" fields
/// ```
pub fn remove_geometry_fields(tree: &mut JsonValue) -> Result<()> {
    remove_geometry(tree, ALL_GEOMETRY_FIELDS);
    Ok(())
}

/// Like [`remove_geometry_fields`], but only removes `fillGeometry` / `strokeGeometry`.
///
/// `remove_geometry_fields` also strips every `styleID` and `windingRule` key in the tree,
/// including `styleID` in text `styleOverrideTable`s (which `characterStyleIDs` point at, so
/// mixed-style text can no longer be mapped) and in vector networks. This keeps those.
pub fn remove_geometry_only(tree: &mut JsonValue) -> Result<()> {
    remove_geometry(tree, PATH_GEOMETRY_FIELDS);
    Ok(())
}

/// Path geometry keys, removed by both passes.
const PATH_GEOMETRY_FIELDS: &[&str] = &["fillGeometry", "strokeGeometry"];

/// Everything [`remove_geometry_fields`] removes.
const ALL_GEOMETRY_FIELDS: &[&str] = &["fillGeometry", "strokeGeometry", "windingRule", "styleID"];

/// Name prefixes that mark a node as an icon whose geometry must be kept.
const ICON_NAME_PREFIXES: &[&str] = &["icon/", "arrows/"];

/// Remove `fields` from every object that is not an icon or image node.
fn remove_geometry(tree: &mut JsonValue, fields: &[&str]) {
    for_each_object_mut(tree, &mut |map| {
        if !should_preserve_geometry(map) {
            for field in fields {
                map.remove(*field);
            }
        }
    });
}

/// Determines if geometry data should be preserved for this node
///
/// Geometry is preserved for icons and images, which are identified by:
/// 1. Having a name starting with "icon/" or "arrows/" (common icon naming patterns)
/// 2. Having an exportSettings entry with an imageType field in
///    symbolData.symbolOverrides (indicates SVG/PNG export)
///
/// Note: We only check the node's OWN name, not names in symbolOverrides which represent
/// child components.
fn should_preserve_geometry(obj: &Map<String, JsonValue>) -> bool {
    let is_icon = obj
        .get("name")
        .and_then(JsonValue::as_str)
        .is_some_and(|name| ICON_NAME_PREFIXES.iter().any(|p| name.starts_with(p)));
    if is_icon {
        return true;
    }

    obj.get("symbolData")
        .and_then(|symbol_data| symbol_data.get("symbolOverrides"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("exportSettings").and_then(JsonValue::as_array))
        .flatten()
        .any(|setting| setting.get("imageType").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_remove_fill_geometry() {
        let mut tree = json!({
            "name": "Rectangle",
            "fillGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "L", 100.0, 0.0, "L", 100.0, 100.0, "Z"],
                    "styleID": 0,
                    "windingRule": {
                        "__enum__": "WindingRule",
                        "value": "NONZERO"
                    }
                }
            ],
            "size": {"x": 100.0, "y": 100.0}
        });

        remove_geometry_fields(&mut tree).unwrap();

        assert!(tree.get("fillGeometry").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Rectangle"));
        assert!(tree.get("size").is_some());
    }

    #[test]
    fn test_remove_stroke_geometry() {
        let mut tree = json!({
            "name": "Line",
            "strokeGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "L", 100.0, 100.0],
                    "styleID": 0,
                    "windingRule": {
                        "__enum__": "WindingRule",
                        "value": "NONZERO"
                    }
                }
            ],
            "visible": true
        });

        remove_geometry_fields(&mut tree).unwrap();

        assert!(tree.get("strokeGeometry").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Line"));
        assert_eq!(tree.get("visible").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_remove_both_geometries() {
        let mut tree = json!({
            "name": "Shape",
            "fillGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "Z"],
                    "styleID": 1
                }
            ],
            "strokeGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "L", 10.0, 10.0],
                    "styleID": 2
                }
            ],
            "opacity": 1.0
        });

        remove_geometry_fields(&mut tree).unwrap();

        assert!(tree.get("fillGeometry").is_none());
        assert!(tree.get("strokeGeometry").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Shape"));
        assert_eq!(tree.get("opacity").unwrap().as_f64(), Some(1.0));
    }

    #[test]
    fn test_remove_nested_geometry() {
        let mut tree = json!({
            "name": "Root",
            "children": [
                {
                    "name": "Child1",
                    "fillGeometry": [
                        {
                            "commands": ["M", 0.0, 0.0, "Z"],
                            "styleID": 0
                        }
                    ]
                },
                {
                    "name": "Child2",
                    "strokeGeometry": [
                        {
                            "commands": ["M", 0.0, 0.0, "L", 10.0, 10.0]
                        }
                    ]
                }
            ]
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Children geometries should be removed
        assert!(tree["children"][0].get("fillGeometry").is_none());
        assert_eq!(
            tree["children"][0].get("name").unwrap().as_str(),
            Some("Child1")
        );

        assert!(tree["children"][1].get("strokeGeometry").is_none());
        assert_eq!(
            tree["children"][1].get("name").unwrap().as_str(),
            Some("Child2")
        );
    }

    #[test]
    fn test_remove_winding_rule_standalone() {
        let mut tree = json!({
            "name": "Path",
            "windingRule": {
                "__enum__": "WindingRule",
                "value": "EVENODD"
            },
            "visible": true
        });

        remove_geometry_fields(&mut tree).unwrap();

        assert!(tree.get("windingRule").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Path"));
        assert_eq!(tree.get("visible").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_remove_style_id_standalone() {
        let mut tree = json!({
            "name": "Element",
            "styleID": 42,
            "type": "SHAPE"
        });

        remove_geometry_fields(&mut tree).unwrap();

        assert!(tree.get("styleID").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Element"));
        assert_eq!(tree.get("type").unwrap().as_str(), Some("SHAPE"));
    }

    #[test]
    fn test_remove_all_geometry_fields() {
        let mut tree = json!({
            "name": "Complex",
            "fillGeometry": [{"commands": ["M", 0.0, 0.0, "Z"]}],
            "strokeGeometry": [{"commands": ["M", 0.0, 0.0, "L", 10.0, 10.0]}],
            "windingRule": {"__enum__": "WindingRule", "value": "NONZERO"},
            "styleID": 5,
            "opacity": 1.0
        });

        remove_geometry_fields(&mut tree).unwrap();

        assert!(tree.get("fillGeometry").is_none());
        assert!(tree.get("strokeGeometry").is_none());
        assert!(tree.get("windingRule").is_none());
        assert!(tree.get("styleID").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Complex"));
        assert_eq!(tree.get("opacity").unwrap().as_f64(), Some(1.0));
    }

    #[test]
    fn test_remove_geometry_missing() {
        let mut tree = json!({
            "name": "Simple",
            "x": 10,
            "y": 20,
            "width": 100,
            "height": 100
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Tree without geometry fields should be unchanged
        assert!(tree.get("fillGeometry").is_none());
        assert!(tree.get("strokeGeometry").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Simple"));
        assert_eq!(tree.get("x").unwrap().as_i64(), Some(10));
        assert_eq!(tree.get("y").unwrap().as_i64(), Some(20));
    }

    #[test]
    fn test_remove_geometry_deeply_nested() {
        let mut tree = json!({
            "document": {
                "fillGeometry": [{"commands": ["M", 0.0, 0.0, "Z"]}],
                "children": [
                    {
                        "children": [
                            {
                                "strokeGeometry": [{"commands": ["L", 10.0, 10.0]}],
                                "name": "DeepChild"
                            }
                        ]
                    }
                ]
            }
        });

        remove_geometry_fields(&mut tree).unwrap();

        // All geometries should be removed at all levels
        assert!(tree["document"].get("fillGeometry").is_none());
        assert!(tree["document"]["children"][0]["children"][0]
            .get("strokeGeometry")
            .is_none());

        // Other fields should be preserved
        assert_eq!(
            tree["document"]["children"][0]["children"][0]
                .get("name")
                .unwrap()
                .as_str(),
            Some("DeepChild")
        );
    }

    #[test]
    fn test_remove_geometry_empty_object() {
        let mut tree = json!({});

        remove_geometry_fields(&mut tree).unwrap();

        // Empty object should remain empty
        assert_eq!(tree.as_object().unwrap().len(), 0);
    }

    #[test]
    fn test_preserve_geometry_for_icon_with_export_settings_svg() {
        let mut tree = json!({
            "name": "icon/ai",
            "fillGeometry": [
                {
                    "commands": ["M", 14.1667, 1.11133, "L", 5.83339, 1.11133, "Z"],
                    "styleID": 0
                }
            ],
            "symbolData": {
                "symbolOverrides": [
                    {
                        "exportSettings": [
                            {
                                "imageType": {
                                    "__enum__": "ImageType",
                                    "value": "SVG"
                                }
                            }
                        ]
                    }
                ]
            }
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Geometry should be preserved for icons with exportSettings
        assert!(tree.get("fillGeometry").is_some());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("icon/ai"));
    }

    #[test]
    fn test_preserve_geometry_for_icon_with_export_settings_png() {
        let mut tree = json!({
            "name": "arrows/chevron-right",
            "strokeGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "L", 10.0, 10.0],
                    "styleID": 0
                }
            ],
            "symbolData": {
                "symbolOverrides": [
                    {
                        "exportSettings": [
                            {
                                "imageType": {
                                    "__enum__": "ImageType",
                                    "value": "PNG"
                                }
                            }
                        ]
                    }
                ]
            }
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Geometry should be preserved for PNG icons
        assert!(tree.get("strokeGeometry").is_some());
        assert_eq!(
            tree.get("name").unwrap().as_str(),
            Some("arrows/chevron-right")
        );
    }

    #[test]
    fn test_preserve_geometry_for_icon_by_name_pattern() {
        let mut tree = json!({
            "name": "icon/star",
            "fillGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "Z"],
                    "styleID": 0
                }
            ]
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Geometry should be preserved based on name pattern alone
        assert!(tree.get("fillGeometry").is_some());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("icon/star"));
    }

    #[test]
    fn test_preserve_geometry_for_arrows_by_name_pattern() {
        let mut tree = json!({
            "name": "arrows/left",
            "fillGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "Z"],
                    "styleID": 0
                }
            ],
            "strokeGeometry": [
                {
                    "commands": ["L", 10.0, 10.0],
                    "styleID": 1
                }
            ]
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Geometry should be preserved for arrows
        assert!(tree.get("fillGeometry").is_some());
        assert!(tree.get("strokeGeometry").is_some());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("arrows/left"));
    }

    #[test]
    fn test_remove_geometry_for_non_icon_with_name() {
        let mut tree = json!({
            "name": "Button",
            "fillGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "Z"],
                    "styleID": 0
                }
            ]
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Geometry should be removed for regular elements
        assert!(tree.get("fillGeometry").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Button"));
    }

    #[test]
    fn test_mixed_icon_and_regular_nodes() {
        let mut tree = json!({
            "name": "Root",
            "children": [
                {
                    "name": "icon/home",
                    "fillGeometry": [
                        {
                            "commands": ["M", 0.0, 0.0, "Z"],
                            "styleID": 0
                        }
                    ]
                },
                {
                    "name": "Button",
                    "fillGeometry": [
                        {
                            "commands": ["M", 0.0, 0.0, "Z"],
                            "styleID": 0
                        }
                    ]
                }
            ]
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Icon should preserve geometry
        assert!(tree["children"][0].get("fillGeometry").is_some());
        assert_eq!(
            tree["children"][0].get("name").unwrap().as_str(),
            Some("icon/home")
        );

        // Button should have geometry removed
        assert!(tree["children"][1].get("fillGeometry").is_none());
        assert_eq!(
            tree["children"][1].get("name").unwrap().as_str(),
            Some("Button")
        );
    }

    #[test]
    fn test_preserve_geometry_in_derived_symbol_data() {
        let mut tree = json!({
            "name": "Root",
            "derivedSymbolData": [
                {
                    "fillGeometry": [
                        {
                            "commands": ["M", 0.0, 0.0, "Z"],
                            "styleID": 0
                        }
                    ]
                }
            ],
            "symbolData": {
                "symbolOverrides": [
                    {
                        "exportSettings": [
                            {
                                "imageType": {
                                    "__enum__": "ImageType",
                                    "value": "SVG"
                                }
                            }
                        ]
                    }
                ]
            }
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Node-level geometry should be preserved due to exportSettings
        // But derivedSymbolData is an array, so it gets recursed differently
        // The geometry in derivedSymbolData should be removed because derivedSymbolData elements
        // themselves don't have exportSettings
        assert!(tree["derivedSymbolData"][0].get("fillGeometry").is_none());
    }

    #[test]
    fn test_preserve_both_fill_and_stroke_geometry_for_icons() {
        let mut tree = json!({
            "name": "icon/complex",
            "fillGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "Z"],
                    "styleID": 0
                }
            ],
            "strokeGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "L", 10.0, 10.0],
                    "styleID": 1
                }
            ],
            "windingRule": {
                "__enum__": "WindingRule",
                "value": "NONZERO"
            },
            "styleID": 5
        });

        remove_geometry_fields(&mut tree).unwrap();

        // All geometry fields should be preserved for icons
        assert!(tree.get("fillGeometry").is_some());
        assert!(tree.get("strokeGeometry").is_some());
        assert!(tree.get("windingRule").is_some());
        assert!(tree.get("styleID").is_some());
    }

    #[test]
    fn test_remove_geometry_from_button_with_icon_child() {
        let mut tree = json!({
            "name": "Button",
            "fillGeometry": [
                {
                    "commands": ["M", 0.0, 0.0, "Z"],
                    "styleID": 0
                }
            ],
            "symbolData": {
                "symbolOverrides": [
                    {
                        "name": "icon/settings"
                    }
                ]
            }
        });

        remove_geometry_fields(&mut tree).unwrap();

        // Button should have geometry removed even if it has icon children in symbolOverrides
        assert!(tree.get("fillGeometry").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Button"));
    }

    #[test]
    fn test_remove_geometry_only_keeps_style_ids() {
        let mut tree = json!({
            "name": "Label",
            "fillGeometry": [{"commands": ["M", 0.0, 0.0], "styleID": 0}],
            "textData": {
                "characterStyleIDs": [0, 0, 91],
                "styleOverrideTable": [{"styleID": 91, "fontSize": 20.0}]
            },
            "vectorNetwork": {"regions": [{"styleID": 2, "windingRule": "NONZERO"}]}
        });

        remove_geometry_only(&mut tree).unwrap();

        assert!(tree.get("fillGeometry").is_none());
        assert_eq!(tree["textData"]["styleOverrideTable"][0]["styleID"], 91);
        assert_eq!(tree["vectorNetwork"]["regions"][0]["styleID"], 2);
        assert_eq!(
            tree["vectorNetwork"]["regions"][0]["windingRule"],
            "NONZERO"
        );
    }
}
