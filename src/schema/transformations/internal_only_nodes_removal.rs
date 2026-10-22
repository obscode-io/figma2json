use crate::error::Result;
use serde_json::Value as JsonValue;

/// Remove internal-only nodes from the JSON tree
///
/// Recursively traverses the JSON tree and filters out nodes that have
/// `internalOnly: true`. These are Figma internal nodes that are not meant
/// for rendering and should not be included in the final output.
///
/// Internal-only nodes are typically removed from "children" arrays.
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully removed all internal-only nodes
///
/// # Examples
/// ```no_run
/// use figma2json::schema::remove_internal_only_nodes;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "children": [
///         {"name": "Visible", "visible": true},
///         {"name": "Internal", "internalOnly": true}
///     ]
/// });
/// remove_internal_only_nodes(&mut tree).unwrap();
/// // children array now contains only the "Visible" node
/// ```
pub fn remove_internal_only_nodes(tree: &mut JsonValue) -> Result<()> {
    remove_internal_only(tree);
    Ok(())
}

/// Drop `internalOnly: true` elements from every array (before recursing into the
/// survivors), and strip the `internalOnly` flag from every object after its
/// children have been processed.
///
/// Only array elements are removed; an internal-only object held directly by an
/// object key (or the root) merely loses its flag.
fn remove_internal_only(value: &mut JsonValue) {
    match value {
        JsonValue::Object(map) => {
            map.values_mut().for_each(remove_internal_only);
            map.remove("internalOnly");
        }
        JsonValue::Array(items) => {
            items.retain(|node| !is_internal_only(node));
            items.iter_mut().for_each(remove_internal_only);
        }
        _ => {}
    }
}

/// Whether `node` is an object with `internalOnly: true`.
fn is_internal_only(node: &JsonValue) -> bool {
    node.get("internalOnly").and_then(JsonValue::as_bool) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_remove_internal_only_node() {
        let mut tree = json!({
            "children": [
                {"name": "Visible", "visible": true},
                {"name": "Internal", "internalOnly": true, "visible": false}
            ]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let children = tree.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["name"].as_str(), Some("Visible"));
    }

    #[test]
    fn test_preserve_visible_nodes() {
        let mut tree = json!({
            "children": [
                {"name": "Node1", "visible": true},
                {"name": "Node2", "visible": true},
                {"name": "Node3", "visible": false}
            ]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let children = tree.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn test_remove_multiple_internal_nodes() {
        let mut tree = json!({
            "children": [
                {"name": "Visible1", "visible": true},
                {"name": "Internal1", "internalOnly": true},
                {"name": "Visible2", "visible": true},
                {"name": "Internal2", "internalOnly": true}
            ]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let children = tree.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0]["name"].as_str(), Some("Visible1"));
        assert_eq!(children[1]["name"].as_str(), Some("Visible2"));
    }

    #[test]
    fn test_all_internal_nodes() {
        let mut tree = json!({
            "children": [
                {"name": "Internal1", "internalOnly": true},
                {"name": "Internal2", "internalOnly": true}
            ]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let children = tree.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 0);
    }

    #[test]
    fn test_no_internal_nodes() {
        let mut tree = json!({
            "children": [
                {"name": "Node1"},
                {"name": "Node2"}
            ]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let children = tree.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn test_nested_children() {
        let mut tree = json!({
            "children": [
                {
                    "name": "Parent",
                    "children": [
                        {"name": "Child1", "visible": true},
                        {"name": "Internal", "internalOnly": true}
                    ]
                }
            ]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let parent_children = tree["children"][0]["children"].as_array().unwrap();
        assert_eq!(parent_children.len(), 1);
        assert_eq!(parent_children[0]["name"].as_str(), Some("Child1"));
    }

    #[test]
    fn test_deeply_nested() {
        let mut tree = json!({
            "document": {
                "children": [
                    {
                        "name": "Canvas",
                        "children": [
                            {"name": "Frame", "visible": true},
                            {"name": "Internal Canvas", "internalOnly": true}
                        ]
                    }
                ]
            }
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let canvas_children = tree["document"]["children"][0]["children"]
            .as_array()
            .unwrap();
        assert_eq!(canvas_children.len(), 1);
        assert_eq!(canvas_children[0]["name"].as_str(), Some("Frame"));
    }

    #[test]
    fn test_internal_only_false() {
        let mut tree = json!({
            "children": [
                {"name": "Node1", "internalOnly": false},
                {"name": "Node2", "internalOnly": true}
            ]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let children = tree.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["name"].as_str(), Some("Node1"));
        assert!(children[0].get("internalOnly").is_none());
    }

    #[test]
    fn test_remove_internal_only_field() {
        let mut tree = json!({
            "name": "Node",
            "internalOnly": false,
            "visible": true
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        assert!(tree.get("internalOnly").is_none());
        assert_eq!(tree.get("name").unwrap().as_str(), Some("Node"));
        assert_eq!(tree.get("visible").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_non_object_array_elements() {
        let mut tree = json!({
            "data": [1, 2, 3, "string"]
        });

        remove_internal_only_nodes(&mut tree).unwrap();

        let data = tree.get("data").unwrap().as_array().unwrap();
        assert_eq!(data.len(), 4);
    }
}
