//! Keep the design-system context that implementation work needs.
//!
//! The rest of the pipeline strips Figma internals to keep the output small. Some
//! of what it strips is exactly what an implementer (or an AI agent) needs to build
//! a design faithfully, so this pass runs first and turns it into readable data:
//!
//! - **Node ids**: every node gets `id` (`"sessionID:localID"`, the same id Figma
//!   shows in `?node-id=` URLs), so nodes, prototype targets and component links
//!   can be referenced.
//! - **Node types**: internal names become Figma's public names (`SYMBOL` →
//!   `COMPONENT` / `COMPONENT_SET`, `ROUNDED_RECTANGLE` → `RECTANGLE`).
//! - **Internal references**: objects shaped `{localID, sessionID}` (component ids,
//!   prototype targets, property ids) and `guidPath`s become `"s:l"` strings, which
//!   the later removal passes leave alone.
//! - **Variables**: every variable binding (`ALIAS`) gets `variable` (its name),
//!   `variableCollection` and `variableId` (matching `id` in the root `variables`). Without this, only the resolved value survives.
//! - **Styles**: `styleIdForFill` / `Text` / `Stroke` / `Effect` / `Grid` become
//!   `fillStyle` / `textStyle` / ... with the style's name.
//! - **Instances**: `componentId`, `componentName` and, for variants,
//!   `componentSetName`.
//! - **Definitions**: all variable collections (with modes and per-mode values) and
//!   all styles are returned so they can be written to the root of the output.
//!
//! Must run after `simplify_enums` and `transform_colors_to_css` (so values are
//! plain strings and hex colors) and before `remove_guid_fields` and
//! `remove_internal_only_nodes` (which delete what this pass reads).

use crate::error::Result;
use serde_json::{json, Map, Value as JsonValue};
use std::collections::HashMap;

/// Sentinel Figma uses for "no style" (`{sessionID: u32::MAX, localID: u32::MAX}`).
const NO_ID: &str = "4294967295:4294967295";

/// Variable collections and styles found in the file, ready for the output root.
#[derive(Debug, Default)]
pub struct DesignDefinitions {
    pub variables: JsonValue,
    pub styles: JsonValue,
}

#[derive(Debug, Clone)]
struct NodeInfo {
    name: String,
    node_type: String,
    parent: Option<String>,
    is_state_group: bool,
}

#[derive(Default)]
struct Index {
    /// guid string -> node summary (every node in the file)
    nodes: HashMap<String, NodeInfo>,
    /// library key -> guid string (variables, collections, styles, components)
    by_key: HashMap<String, String>,
    /// guid of each variable -> guid of its collection
    variable_set: HashMap<String, String>,
}

/// Format a `{sessionID, localID}` object as `"sessionID:localID"`.
fn guid_string(value: &JsonValue) -> Option<String> {
    let obj = value.as_object()?;
    let session = obj.get("sessionID")?.as_u64()?;
    let local = obj.get("localID")?.as_u64()?;
    Some(format!("{}:{}", session, local))
}

/// True for an object holding exactly `sessionID` and `localID`.
fn is_guid_object(value: &JsonValue) -> bool {
    match value.as_object() {
        Some(obj) => obj.len() == 2 && guid_string(value).is_some(),
        None => false,
    }
}

/// Resolve a reference that is either `{guid: {...}}`, `{assetRef: {key}}`, a bare
/// guid object, or an already-converted `"s:l"` string, to a guid string.
fn resolve_ref(reference: &JsonValue, index: &Index) -> Option<String> {
    if let Some(s) = reference.as_str() {
        return (s != NO_ID).then(|| s.to_string());
    }
    if let Some(g) = guid_string(reference) {
        return (g != NO_ID).then_some(g);
    }
    if let Some(g) = reference.get("guid") {
        return resolve_ref(g, index);
    }
    let key = reference.get("assetRef")?.get("key")?.as_str()?;
    index.by_key.get(key).cloned()
}

/// The id inside a single-key `{guid: {...}}` or `{assetRef: {key, ...}}` wrapper. Library
/// references are resolved to the local copy's id when the file has one; otherwise `None`
/// (the wrapper is left as is).
fn wrapped_ref(value: &JsonValue, index: &Index) -> Option<String> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    if let Some(g) = obj.get("guid") {
        return guid_string(g);
    }
    let key = obj.get("assetRef")?.get("key")?.as_str()?;
    index.by_key.get(key).cloned()
}

fn node_type(node: &JsonValue) -> String {
    match node.get("type") {
        Some(JsonValue::String(s)) => s.clone(),
        Some(t) => t
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        None => String::new(),
    }
}

fn build_index(node: &JsonValue, parent: Option<&str>, index: &mut Index) {
    let Some(obj) = node.as_object() else { return };
    let Some(id) = obj.get("guid").and_then(guid_string) else {
        return;
    };

    let info = NodeInfo {
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        node_type: node_type(node),
        parent: parent.map(str::to_string),
        is_state_group: obj
            .get("isStateGroup")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    if let Some(key) = obj.get("key").and_then(|v| v.as_str()) {
        index.by_key.insert(key.to_string(), id.clone());
    }
    if info.node_type == "VARIABLE" {
        if let Some(set) = obj.get("variableSetID") {
            // variableSetID is {guid} for local collections or {assetRef} for library ones;
            // keys are indexed in a second pass, so stash the raw form for now.
            let set_ref = guid_string(set.get("guid").unwrap_or(&JsonValue::Null)).or_else(|| {
                set.get("assetRef")
                    .and_then(|a| a.get("key"))
                    .and_then(|k| k.as_str())
                    .map(|k| format!("key:{}", k))
            });
            if let Some(set_ref) = set_ref {
                index.variable_set.insert(id.clone(), set_ref);
            }
        }
    }
    index.nodes.insert(id.clone(), info);

    if let Some(children) = obj.get("children").and_then(|c| c.as_array()) {
        for child in children {
            build_index(child, Some(&id), index);
        }
    }
}

/// Resolve the "key:..." placeholders left in `variable_set` once all keys are known.
fn finish_index(index: &mut Index) {
    let by_key = index.by_key.clone();
    for set in index.variable_set.values_mut() {
        if let Some(key) = set.strip_prefix("key:") {
            if let Some(id) = by_key.get(key) {
                *set = id.clone();
            }
        }
    }
}

/// `(variable name, collection name)` for a variable guid.
fn variable_names(id: &str, index: &Index) -> Option<(String, Option<String>)> {
    let info = index.nodes.get(id)?;
    if info.node_type != "VARIABLE" {
        return None;
    }
    let collection = index
        .variable_set
        .get(id)
        .and_then(|set| index.nodes.get(set))
        .map(|set| set.name.clone());
    Some((info.name.clone(), collection))
}

/// Add `variable` / `variableCollection` to an ALIAS value object, in place.
fn annotate_alias(
    obj: &mut Map<String, JsonValue>,
    index: &Index,
    uses: &mut HashMap<String, u64>,
    counting: bool,
) {
    if obj.get("dataType").and_then(|v| v.as_str()) != Some("ALIAS") {
        return;
    }
    let target = obj
        .get("value")
        .and_then(|v| v.get("alias"))
        .and_then(|alias| resolve_ref(alias, index));
    if let Some((name, collection)) = target.as_deref().and_then(|id| variable_names(id, index)) {
        if let Some(id) = &target {
            if counting {
                *uses.entry(id.clone()).or_default() += 1;
            }
            obj.insert("variableId".to_string(), JsonValue::String(id.clone()));
        }
        obj.insert("variable".to_string(), JsonValue::String(name));
        if let Some(collection) = collection {
            obj.insert(
                "variableCollection".to_string(),
                JsonValue::String(collection),
            );
        }
    }
}

const STYLE_FIELDS: [(&str, &str); 5] = [
    ("styleIdForFill", "fillStyle"),
    ("styleIdForStrokeFill", "strokeStyle"),
    ("styleIdForText", "textStyle"),
    ("styleIdForEffect", "effectStyle"),
    ("styleIdForGrid", "gridStyle"),
];

/// Annotate one design node (and recurse into everything below it).
fn annotate(
    value: &mut JsonValue,
    index: &Index,
    in_node_position: bool,
    uses: &mut HashMap<String, u64>,
    counting: bool,
) {
    match value {
        JsonValue::Object(obj) => {
            let is_node = in_node_position && obj.contains_key("guid") && obj.contains_key("type");

            // Internal-only pages hold definitions (variables, styles, and local copies of
            // library components), not the design itself: annotate them (they can be kept
            // with `keep_internal`) but don't count them in `uses`.
            let counting = counting
                && !(is_node && obj.get("internalOnly").and_then(|v| v.as_bool()) == Some(true));

            if is_node {
                if let Some(id) = obj.get("guid").and_then(guid_string) {
                    obj.insert("id".to_string(), JsonValue::String(id));
                }

                // Public node type names
                let t = obj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // Component sets are stored as FRAME (sometimes SYMBOL) nodes with isStateGroup
                let is_state_group =
                    obj.get("isStateGroup").and_then(|v| v.as_bool()) == Some(true);
                let public = match t.as_str() {
                    "FRAME" | "SYMBOL" if is_state_group => Some("COMPONENT_SET"),
                    "SYMBOL" => Some("COMPONENT"),
                    "ROUNDED_RECTANGLE" => Some("RECTANGLE"),
                    _ => None,
                };
                if let Some(public) = public {
                    obj.insert("type".to_string(), JsonValue::String(public.to_string()));
                }

                // Instance -> component
                if t == "INSTANCE" {
                    let component = obj
                        .get("symbolData")
                        .and_then(|s| s.get("symbolID"))
                        .and_then(|r| resolve_ref(r, index));
                    if let Some(component_id) = component {
                        if let Some(info) = index.nodes.get(&component_id) {
                            obj.insert(
                                "componentName".to_string(),
                                JsonValue::String(info.name.clone()),
                            );
                            let set = info.parent.as_ref().and_then(|p| index.nodes.get(p));
                            if let Some(set) = set.filter(|s| s.is_state_group) {
                                obj.insert(
                                    "componentSetName".to_string(),
                                    JsonValue::String(set.name.clone()),
                                );
                            }
                        }
                        obj.insert("componentId".to_string(), JsonValue::String(component_id));
                    }
                }
            }

            // Shared styles (on nodes, and inside instance overrides)
            for (field, out) in STYLE_FIELDS {
                if let Some(reference) = obj.get(field) {
                    let style = resolve_ref(reference, index)
                        .and_then(|id| index.nodes.get(&id).map(|info| (id, info.name.clone())));
                    if let Some((id, name)) = style {
                        if counting {
                            *uses.entry(id).or_default() += 1;
                        }
                        obj.insert(out.to_string(), JsonValue::String(name));
                    }
                }
            }

            annotate_alias(obj, index, uses, counting);

            // guidPath {guids: [...]} -> "s:l/s:l" (which child an override targets)
            if let Some(path) = obj.get("guidPath") {
                if let Some(guids) = path.get("guids").and_then(|g| g.as_array()) {
                    let parts: Vec<String> = guids.iter().filter_map(guid_string).collect();
                    if parts.len() == guids.len() {
                        obj.insert("guidPath".to_string(), JsonValue::String(parts.join("/")));
                    }
                }
            }

            for (key, child) in obj.iter_mut() {
                if key == "guid" && is_node {
                    continue; // removed later by remove_guid_fields; `id` replaces it
                }
                // {guid: {...}} / {assetRef: {key}} wrappers (instance-swap and slot values,
                // aliases, collection ids): replace with the id, or remove_guid_fields would
                // empty them and the value would be lost.
                if let Some(id) = wrapped_ref(child, index) {
                    *child = if id == NO_ID {
                        JsonValue::Null
                    } else {
                        JsonValue::String(id)
                    };
                    continue;
                }
                if is_guid_object(child) {
                    if let Some(s) = guid_string(child) {
                        *child = if s == NO_ID {
                            JsonValue::Null
                        } else {
                            JsonValue::String(s)
                        };
                    }
                    continue;
                }
                annotate(child, index, key == "children", uses, counting);
            }
        }
        JsonValue::Array(items) => {
            for item in items.iter_mut() {
                if let Some(id) = wrapped_ref(item, index) {
                    *item = if id == NO_ID {
                        JsonValue::Null
                    } else {
                        JsonValue::String(id)
                    };
                    continue;
                }
                if is_guid_object(item) {
                    if let Some(s) = guid_string(item) {
                        *item = JsonValue::String(s);
                    }
                    continue;
                }
                annotate(item, index, in_node_position, uses, counting);
            }
        }
        _ => {}
    }
}

/// Convert one `variableData` value to a plain value, or `{alias: name}`.
fn variable_value(data: &JsonValue, index: &Index) -> JsonValue {
    let data_type = data.get("dataType").and_then(|v| v.as_str()).unwrap_or("");
    let value = data.get("value").cloned().unwrap_or(JsonValue::Null);
    if data_type == "ALIAS" {
        let target = value.get("alias").and_then(|a| resolve_ref(a, index));
        return match target.as_deref().and_then(|id| variable_names(id, index)) {
            Some((name, Some(collection))) => json!({"alias": name, "aliasCollection": collection}),
            Some((name, None)) => json!({"alias": name}),
            None => json!({"alias": null}),
        };
    }
    // {"colorValue": "#fff"} / {"floatValue": 4} / {"textValue": ".."} / {"boolValue": true}
    match value.as_object() {
        Some(obj) if obj.len() == 1 => obj.values().next().cloned().unwrap_or(JsonValue::Null),
        _ => value,
    }
}

fn collect_definitions(
    node: &JsonValue,
    index: &Index,
    out: &mut (Vec<JsonValue>, Vec<JsonValue>, Vec<JsonValue>),
) {
    let Some(obj) = node.as_object() else { return };
    let t = node_type(node);
    let id = obj.get("guid").and_then(guid_string).unwrap_or_default();
    let name = obj.get("name").cloned().unwrap_or(JsonValue::Null);

    if t == "VARIABLE_SET" {
        let modes: Vec<JsonValue> = obj
            .get("variableSetModes")
            .and_then(|m| m.as_array())
            .map(|modes| {
                modes
                    .iter()
                    .map(
                        |m| json!({"id": m.get("id").and_then(guid_string), "name": m.get("name")}),
                    )
                    .collect()
            })
            .unwrap_or_default();
        out.0.push(json!({
            "id": id,
            "name": name,
            "key": obj.get("key"),
            "sourceLibraryKey": obj.get("sourceLibraryKey"),
            "modes": modes,
        }));
    } else if t == "VARIABLE" {
        let mut values = Map::new();
        if let Some(entries) = obj
            .get("variableDataValues")
            .and_then(|v| v.get("entries"))
            .and_then(|e| e.as_array())
        {
            for entry in entries {
                let mode = entry
                    .get("modeID")
                    .and_then(guid_string)
                    .unwrap_or_default();
                let data = entry
                    .get("variableData")
                    .cloned()
                    .unwrap_or(JsonValue::Null);
                values.insert(mode, variable_value(&data, index));
            }
        }
        out.1.push(json!({
            "id": id,
            "name": name,
            "key": obj.get("key"),
            "collectionId": index.variable_set.get(&id),
            "type": obj.get("variableResolvedType"),
            "scopes": obj.get("variableScopes"),
            "description": obj.get("description"),
            "valuesByMode": values,
        }));
    } else if let Some(style_type) = obj.get("styleType") {
        let mut style = json!({
            "id": id,
            "name": name,
            "key": obj.get("key"),
            "styleType": style_type,
            "sourceLibraryKey": obj.get("sourceLibraryKey"),
        });
        let fields: &[&str] = match style_type.as_str().unwrap_or("") {
            "FILL" => &["fillPaints"],
            "TEXT" => &[
                "fontName",
                "fontSize",
                "lineHeight",
                "letterSpacing",
                "textCase",
                "textDecoration",
                "paragraphSpacing",
            ],
            "EFFECT" => &["effects"],
            "GRID" => &["layoutGrids"],
            _ => &[],
        };
        for field in fields {
            if let Some(v) = obj.get(*field) {
                style[*field] = v.clone();
            }
        }
        if let Some(d) = obj
            .get("description")
            .filter(|d| d.as_str().is_some_and(|s| !s.is_empty()))
        {
            style["description"] = d.clone();
        }
        out.2.push(style);
    }

    if let Some(children) = obj.get("children").and_then(|c| c.as_array()) {
        for child in children {
            collect_definitions(child, index, out);
        }
    }
}

/// Drop null fields (e.g. no `sourceLibraryKey` on local definitions).
fn tidy(value: &mut JsonValue) {
    if let Some(obj) = value.as_object_mut() {
        obj.retain(|_, v| !v.is_null());
    }
}

/// Annotate the document in place and return the file's variables and styles.
pub fn add_design_context(document: &mut JsonValue) -> Result<DesignDefinitions> {
    let mut index = Index::default();
    build_index(document, None, &mut index);
    finish_index(&mut index);

    let mut found = (Vec::new(), Vec::new(), Vec::new());
    collect_definitions(document, &index, &mut found);
    let (mut collections, variables, mut styles) = found;

    // Nest variables under their collection
    for collection in collections.iter_mut() {
        let cid = collection["id"].as_str().unwrap_or("").to_string();
        let mode_names: HashMap<String, String> = collection["modes"]
            .as_array()
            .map(|m| {
                m.iter()
                    .filter_map(|m| {
                        Some((
                            m["id"].as_str()?.to_string(),
                            m["name"].as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut vars: Vec<JsonValue> = variables
            .iter()
            .filter(|v| v["collectionId"].as_str() == Some(cid.as_str()))
            .cloned()
            .collect();
        for v in vars.iter_mut() {
            // Key values by mode NAME ("Desktop (1440px)") instead of mode id
            if let Some(values) = v.get_mut("valuesByMode").and_then(|x| x.as_object_mut()) {
                let renamed: Map<String, JsonValue> = std::mem::take(values)
                    .into_iter()
                    .map(|(mode, val)| (mode_names.get(&mode).cloned().unwrap_or(mode), val))
                    .collect();
                *values = renamed;
            }
            if let Some(o) = v.as_object_mut() {
                o.remove("collectionId");
                if o.get("description")
                    .is_some_and(|d| d.is_null() || d.as_str() == Some(""))
                {
                    o.remove("description");
                }
            }
        }
        collection["variables"] = JsonValue::Array(vars);
    }
    let orphans: Vec<JsonValue> = variables
        .into_iter()
        .filter(|v| {
            !collections
                .iter()
                .any(|c| c["id"].as_str().is_some() && c["id"] == v["collectionId"])
        })
        .collect();
    if !orphans.is_empty() {
        collections.push(
            json!({"id": null, "name": "(unknown collection)", "modes": [], "variables": orphans}),
        );
    }

    // Annotate the design tree, counting how often each variable and style is used
    let mut uses: HashMap<String, u64> = HashMap::new();
    annotate(document, &index, true, &mut uses, true);

    let count = |v: &JsonValue| {
        v["id"]
            .as_str()
            .and_then(|id| uses.get(id))
            .copied()
            .unwrap_or(0)
    };
    for collection in collections.iter_mut() {
        let mut total = 0;
        if let Some(vars) = collection["variables"].as_array_mut() {
            for v in vars.iter_mut() {
                let n = count(v);
                total += n;
                v["uses"] = json!(n);
            }
        }
        collection["uses"] = json!(total);
        tidy(collection);
    }
    for style in styles.iter_mut() {
        style["uses"] = json!(count(style));
        tidy(style);
    }

    // Text styles: make lineHeight / letterSpacing CSS-like, as for nodes.
    let mut styles_value = JsonValue::Array(std::mem::take(&mut styles));
    crate::schema::simplify_text_properties(&mut styles_value)?;

    Ok(DesignDefinitions {
        variables: JsonValue::Array(collections),
        styles: styles_value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guid(s: u64, l: u64) -> JsonValue {
        json!({"sessionID": s, "localID": l})
    }

    fn sample() -> JsonValue {
        json!({
            "guid": guid(0, 0), "type": "DOCUMENT", "name": "Document",
            "children": [
                {"guid": guid(0, 1), "type": "CANVAS", "name": "Page 1", "children": [
                    {"guid": guid(1, 10), "type": "FRAME", "name": "Button", "isStateGroup": true, "children": [
                        {"guid": guid(1, 11), "type": "SYMBOL", "name": "State=Default"}
                    ]},
                    {"guid": guid(1, 20), "type": "INSTANCE", "name": "Button",
                     "symbolData": {"symbolID": guid(1, 11)},
                     "styleIdForText": {"assetRef": {"key": "style-key"}},
                     "fillPaints": [{"type": "SOLID", "color": "#3656f1",
                        "colorVar": {"dataType": "ALIAS", "resolvedDataType": "COLOR",
                                     "value": {"alias": {"assetRef": {"key": "var-key"}}}}}],
                     "prototypeInteractions": [{"actions": [{"transitionNodeID": guid(1, 10)}]}]},
                    {"guid": guid(1, 30), "type": "ROUNDED_RECTANGLE", "name": "Box",
                     "styleIdForFill": {"guid": guid(4294967295, 4294967295)}}
                ]},
                {"guid": guid(0, 2), "type": "CANVAS", "name": "Internal", "internalOnly": true, "children": [
                    {"guid": guid(2, 1), "type": "VARIABLE_SET", "name": "1. Colors", "key": "set-key",
                     "variableSetModes": [{"id": guid(9, 0), "name": "Light"}]},
                    {"guid": guid(2, 2), "type": "VARIABLE", "name": "brand/primary", "key": "var-key",
                     "variableSetID": {"assetRef": {"key": "set-key"}},
                     "variableResolvedType": "COLOR",
                     "variableDataValues": {"entries": [
                        {"modeID": guid(9, 0), "variableData": {"dataType": "ALIAS",
                            "value": {"alias": {"guid": guid(2, 3)}}}}]}},
                    {"guid": guid(2, 3), "type": "VARIABLE", "name": "master/blue", "key": "base-key",
                     "variableSetID": {"guid": guid(2, 1)},
                     "variableResolvedType": "COLOR",
                     "variableDataValues": {"entries": [
                        {"modeID": guid(9, 0), "variableData": {"dataType": "COLOR",
                            "value": {"colorValue": "#3656f1"}}}]}},
                    {"guid": guid(2, 4), "type": "TEXT", "name": "Body/M-Regular", "key": "style-key",
                     "styleType": "TEXT", "fontSize": 14.0, "fontName": {"family": "Geist", "style": "Regular"},
                     "lineHeight": {"units": "PIXELS", "value": 20.0}}
                ]}
            ]
        })
    }

    #[test]
    fn test_node_ids_and_public_types() {
        let mut doc = sample();
        add_design_context(&mut doc).unwrap();
        let set = &doc["children"][0]["children"][0];
        assert_eq!(set["id"], "1:10");
        assert_eq!(set["type"], "COMPONENT_SET");
        assert_eq!(set["children"][0]["type"], "COMPONENT");
        assert_eq!(doc["children"][0]["children"][2]["type"], "RECTANGLE");
        // guid kept for remove_guid_fields to delete later
        assert!(set.get("guid").is_some());
    }

    #[test]
    fn test_instance_component_names() {
        let mut doc = sample();
        add_design_context(&mut doc).unwrap();
        let inst = &doc["children"][0]["children"][1];
        assert_eq!(inst["componentId"], "1:11");
        assert_eq!(inst["componentName"], "State=Default");
        assert_eq!(inst["componentSetName"], "Button");
        assert_eq!(inst["symbolData"]["symbolID"], "1:11");
    }

    #[test]
    fn test_variable_binding_is_named() {
        let mut doc = sample();
        add_design_context(&mut doc).unwrap();
        let var = &doc["children"][0]["children"][1]["fillPaints"][0]["colorVar"];
        assert_eq!(var["variable"], "brand/primary");
        assert_eq!(var["variableId"], "2:2");
        assert_eq!(var["variableCollection"], "1. Colors");
    }

    #[test]
    fn test_styles_and_sentinel() {
        let mut doc = sample();
        add_design_context(&mut doc).unwrap();
        assert_eq!(
            doc["children"][0]["children"][1]["textStyle"],
            "Body/M-Regular"
        );
        let boxed = &doc["children"][0]["children"][2];
        assert!(boxed.get("fillStyle").is_none());
    }

    #[test]
    fn test_prototype_target_becomes_string() {
        let mut doc = sample();
        add_design_context(&mut doc).unwrap();
        let action = &doc["children"][0]["children"][1]["prototypeInteractions"][0]["actions"][0];
        assert_eq!(action["transitionNodeID"], "1:10");
    }

    #[test]
    fn test_definitions() {
        let mut doc = sample();
        let defs = add_design_context(&mut doc).unwrap();
        let collections = defs.variables.as_array().unwrap();
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0]["name"], "1. Colors");
        let vars = collections[0]["variables"].as_array().unwrap();
        let primary = vars.iter().find(|v| v["name"] == "brand/primary").unwrap();
        assert_eq!(primary["valuesByMode"]["Light"]["alias"], "master/blue");
        let blue = vars.iter().find(|v| v["name"] == "master/blue").unwrap();
        assert_eq!(blue["valuesByMode"]["Light"], "#3656f1");
        assert_eq!(primary["uses"], 1);
        assert_eq!(blue["uses"], 0);
        assert_eq!(collections[0]["uses"], 1);
        let styles = defs.styles.as_array().unwrap();
        assert_eq!(styles[0]["name"], "Body/M-Regular");
        assert_eq!(styles[0]["uses"], 1);
        assert_eq!(styles[0]["lineHeight"], "20px");
    }

    #[test]
    fn test_internal_nodes_annotated_not_counted() {
        let mut doc = sample();
        let defs = add_design_context(&mut doc).unwrap();
        let internal_var = &doc["children"][1]["children"][1];
        assert_eq!(internal_var["id"], "2:2");
        let blue = defs.variables[0]["variables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "master/blue")
            .unwrap()
            .clone();
        // aliased by brand/primary on the internal page, but not used by the design
        assert_eq!(blue["uses"], 0);
    }

    #[test]
    fn test_wrapped_references_survive() {
        let mut doc = json!({"guid": guid(0, 0), "type": "DOCUMENT", "children": [
            {"guid": guid(1, 1), "type": "INSTANCE", "name": "Button",
             "componentPropAssignments": [
                {"defID": guid(4, 2), "varValue": {"dataType": "SYMBOL_ID",
                    "value": {"symbolIdValue": {"guid": guid(91, 157)}}}},
                {"defID": guid(4, 3), "varValue": {"dataType": "SYMBOL_ID",
                    "value": {"symbolIdValue": {"assetRef": {"key": "icon-key"}}}}},
                {"defID": guid(4, 4), "varValue": {"dataType": "SYMBOL_ID",
                    "value": {"symbolIdValue": {"assetRef": {"key": "unknown"}}}}}
             ]},
            {"guid": guid(5, 5), "type": "SYMBOL", "name": "Icon", "key": "icon-key"}]});
        add_design_context(&mut doc).unwrap();
        let a = &doc["children"][0]["componentPropAssignments"];
        assert_eq!(a[0]["varValue"]["value"]["symbolIdValue"], "91:157");
        assert_eq!(a[0]["defID"], "4:2");
        assert_eq!(a[1]["varValue"]["value"]["symbolIdValue"], "5:5");
        // unresolvable library reference is left untouched
        assert_eq!(
            a[2]["varValue"]["value"]["symbolIdValue"]["assetRef"]["key"],
            "unknown"
        );
    }

    #[test]
    fn test_guid_path_and_override_targets() {
        let mut doc = json!({"guid": guid(0, 0), "type": "DOCUMENT", "children": [
            {"guid": guid(1, 1), "type": "INSTANCE", "name": "Card",
             "symbolData": {"symbolOverrides": [
                {"guidPath": {"guids": [guid(1, 5), guid(1, 6)]}, "textData": {"characters": "Hi"}}]}}]});
        add_design_context(&mut doc).unwrap();
        let o = &doc["children"][0]["symbolData"]["symbolOverrides"][0];
        assert_eq!(o["guidPath"], "1:5/1:6");
    }
}
