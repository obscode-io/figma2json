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
//!   `variableCollection` and `variableId` (matching `id` in the root `variables`).
//!   Without this, only the resolved value survives.
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

/// Prefix marking a `variable_set` entry that still holds a library key.
const KEY_PREFIX: &str = "key:";

/// Shared-style reference fields and the name fields they become.
const STYLE_FIELDS: [(&str, &str); 5] = [
    ("styleIdForFill", "fillStyle"),
    ("styleIdForStrokeFill", "strokeStyle"),
    ("styleIdForText", "textStyle"),
    ("styleIdForEffect", "effectStyle"),
    ("styleIdForGrid", "gridStyle"),
];

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

/// The id of an object holding exactly `sessionID` and `localID`.
fn bare_guid(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Object(obj) if obj.len() == 2 => guid_string(value),
        _ => None,
    }
}

/// `JsonValue::String(id)`, or `null` for the "no id" sentinel.
fn id_or_null(id: String) -> JsonValue {
    if id == NO_ID {
        JsonValue::Null
    } else {
        JsonValue::String(id)
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

/// A node's `type`, whether a plain string or a `{value}` enum object.
fn node_type(node: &JsonValue) -> &str {
    match node.get("type") {
        Some(JsonValue::String(s)) => s,
        Some(t) => t.get("value").and_then(JsonValue::as_str).unwrap_or(""),
        None => "",
    }
}

fn str_field<'a>(obj: &'a Map<String, JsonValue>, field: &str) -> Option<&'a str> {
    obj.get(field).and_then(JsonValue::as_str)
}

fn is_true(obj: &Map<String, JsonValue>, field: &str) -> bool {
    obj.get(field).and_then(JsonValue::as_bool) == Some(true)
}

impl Index {
    fn build(document: &JsonValue) -> Self {
        let mut index = Self::default();
        index.add_node(document, None);
        index.resolve_variable_set_keys();
        index
    }

    fn add_node(&mut self, node: &JsonValue, parent: Option<&str>) {
        let Some(obj) = node.as_object() else { return };
        let Some(id) = obj.get("guid").and_then(guid_string) else {
            return;
        };

        let info = NodeInfo {
            name: str_field(obj, "name").unwrap_or("").to_string(),
            node_type: node_type(node).to_string(),
            parent: parent.map(str::to_string),
            is_state_group: is_true(obj, "isStateGroup"),
        };
        if let Some(key) = str_field(obj, "key") {
            self.by_key.insert(key.to_string(), id.clone());
        }
        if info.node_type == "VARIABLE" {
            if let Some(set_ref) = obj.get("variableSetID").and_then(variable_set_ref) {
                self.variable_set.insert(id.clone(), set_ref);
            }
        }
        self.nodes.insert(id.clone(), info);

        if let Some(children) = obj.get("children").and_then(JsonValue::as_array) {
            for child in children {
                self.add_node(child, Some(&id));
            }
        }
    }

    /// Resolve the `"key:..."` placeholders left in `variable_set` once all keys are known.
    fn resolve_variable_set_keys(&mut self) {
        let Self {
            by_key,
            variable_set,
            ..
        } = self;
        for set in variable_set.values_mut() {
            if let Some(id) = set.strip_prefix(KEY_PREFIX).and_then(|key| by_key.get(key)) {
                *set = id.clone();
            }
        }
    }

    /// `(variable name, collection name)` for a variable guid.
    fn variable_names(&self, id: &str) -> Option<(String, Option<String>)> {
        let info = self.nodes.get(id)?;
        if info.node_type != "VARIABLE" {
            return None;
        }
        let collection = self
            .variable_set
            .get(id)
            .and_then(|set| self.nodes.get(set))
            .map(|set| set.name.clone());
        Some((info.name.clone(), collection))
    }
}

/// A variable's collection: `variableSetID` is `{guid}` for local collections or
/// `{assetRef}` for library ones. Keys are only all known after indexing the whole
/// tree, so library ones are stashed as `"key:..."` for now.
fn variable_set_ref(set: &JsonValue) -> Option<String> {
    set.get("guid").and_then(guid_string).or_else(|| {
        set.get("assetRef")
            .and_then(|a| a.get("key"))
            .and_then(JsonValue::as_str)
            .map(|k| format!("{}{}", KEY_PREFIX, k))
    })
}

/// Public name for an internal node type, if it differs.
fn public_type(node_type: &str, is_state_group: bool) -> Option<&'static str> {
    match node_type {
        "FRAME" | "SYMBOL" if is_state_group => Some("COMPONENT_SET"),
        "SYMBOL" => Some("COMPONENT"),
        "ROUNDED_RECTANGLE" => Some("RECTANGLE"),
        _ => None,
    }
}

/// guidPath `{guids: [...]}` -> `"s:l/s:l"` (which child an override targets).
fn flatten_guid_path(obj: &mut Map<String, JsonValue>) {
    let Some(guids) = obj
        .get("guidPath")
        .and_then(|path| path.get("guids"))
        .and_then(JsonValue::as_array)
    else {
        return;
    };
    let parts: Vec<String> = guids.iter().filter_map(guid_string).collect();
    if parts.len() == guids.len() {
        obj.insert("guidPath".to_string(), JsonValue::String(parts.join("/")));
    }
}

/// Walks the document, annotating it and counting variable and style uses.
struct Annotator<'a> {
    index: &'a Index,
    /// guid of each variable and style -> how often the design uses it
    uses: HashMap<String, u64>,
}

impl Annotator<'_> {
    fn count_use(&mut self, id: String, counting: bool) {
        if counting {
            *self.uses.entry(id).or_default() += 1;
        }
    }

    /// Annotate one value (and recurse into everything below it).
    fn annotate(&mut self, value: &mut JsonValue, in_node_position: bool, counting: bool) {
        match value {
            JsonValue::Object(obj) => self.annotate_object(obj, in_node_position, counting),
            JsonValue::Array(items) => {
                for item in items.iter_mut() {
                    if let Some(id) = wrapped_ref(item, self.index) {
                        *item = id_or_null(id);
                    } else if let Some(id) = bare_guid(item) {
                        *item = JsonValue::String(id);
                    } else {
                        self.annotate(item, in_node_position, counting);
                    }
                }
            }
            _ => {}
        }
    }

    /// Annotate one object and recurse into its values.
    ///
    /// Nodes on internal-only pages (definitions: variables, styles and local
    /// copies of library components) are annotated but not counted in `uses`.
    /// `{guid: {...}}` / `{assetRef: {key}}` wrappers (instance-swap and slot
    /// values, aliases, collection ids) are replaced with the id, or
    /// `remove_guid_fields` would empty them and the value would be lost.
    fn annotate_object(
        &mut self,
        obj: &mut Map<String, JsonValue>,
        in_node_position: bool,
        counting: bool,
    ) {
        let is_node = in_node_position && obj.contains_key("guid") && obj.contains_key("type");

        let counting = counting && !(is_node && is_true(obj, "internalOnly"));

        if is_node {
            annotate_node(obj, self.index);
        }
        self.annotate_styles(obj, counting);
        self.annotate_alias(obj, counting);
        flatten_guid_path(obj);

        for (key, child) in obj.iter_mut() {
            if key == "guid" && is_node {
                continue;
            }
            if let Some(id) = wrapped_ref(child, self.index).or_else(|| bare_guid(child)) {
                *child = id_or_null(id);
            } else {
                self.annotate(child, key == "children", counting);
            }
        }
    }

    /// Add `fillStyle` / `textStyle` / ... names for shared-style references.
    fn annotate_styles(&mut self, obj: &mut Map<String, JsonValue>, counting: bool) {
        let index = self.index;
        for (field, out) in STYLE_FIELDS {
            let Some(id) = obj.get(field).and_then(|r| resolve_ref(r, index)) else {
                continue;
            };
            let Some(info) = index.nodes.get(&id) else {
                continue;
            };
            self.count_use(id, counting);
            obj.insert(out.to_string(), JsonValue::String(info.name.clone()));
        }
    }

    /// Add `variable` / `variableCollection` / `variableId` to an ALIAS value object.
    fn annotate_alias(&mut self, obj: &mut Map<String, JsonValue>, counting: bool) {
        if str_field(obj, "dataType") != Some("ALIAS") {
            return;
        }
        let Some(id) = obj
            .get("value")
            .and_then(|v| v.get("alias"))
            .and_then(|alias| resolve_ref(alias, self.index))
        else {
            return;
        };
        let Some((name, collection)) = self.index.variable_names(&id) else {
            return;
        };
        self.count_use(id.clone(), counting);
        obj.insert("variableId".to_string(), JsonValue::String(id));
        obj.insert("variable".to_string(), JsonValue::String(name));
        if let Some(collection) = collection {
            obj.insert(
                "variableCollection".to_string(),
                JsonValue::String(collection),
            );
        }
    }
}

/// Add `id`, the public `type` and, for instances, the component it uses.
fn annotate_node(obj: &mut Map<String, JsonValue>, index: &Index) {
    if let Some(id) = obj.get("guid").and_then(guid_string) {
        obj.insert("id".to_string(), JsonValue::String(id));
    }

    let node_type = str_field(obj, "type").unwrap_or("");
    let is_instance = node_type == "INSTANCE";
    if let Some(public) = public_type(node_type, is_true(obj, "isStateGroup")) {
        obj.insert("type".to_string(), JsonValue::String(public.to_string()));
    }
    if is_instance {
        link_component(obj, index);
    }
}

/// Instance -> `componentId`, `componentName` and, for variants, `componentSetName`.
fn link_component(obj: &mut Map<String, JsonValue>, index: &Index) {
    let Some(component_id) = obj
        .get("symbolData")
        .and_then(|s| s.get("symbolID"))
        .and_then(|r| resolve_ref(r, index))
    else {
        return;
    };
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

/// Convert one `variableData` value to a plain value, or `{alias: name}`.
fn variable_value(data: &JsonValue, index: &Index) -> JsonValue {
    let value = data.get("value").unwrap_or(&JsonValue::Null);
    if data.get("dataType").and_then(JsonValue::as_str) == Some("ALIAS") {
        let target = value.get("alias").and_then(|a| resolve_ref(a, index));
        return match target.and_then(|id| index.variable_names(&id)) {
            Some((name, Some(collection))) => json!({"alias": name, "aliasCollection": collection}),
            Some((name, None)) => json!({"alias": name}),
            None => json!({"alias": null}),
        };
    }
    match value.as_object().filter(|obj| obj.len() == 1) {
        Some(obj) => obj.values().next().cloned().unwrap_or(JsonValue::Null),
        None => value.clone(),
    }
}

/// Fields copied into a style definition, by `styleType`.
fn style_fields(style_type: &str) -> &'static [&'static str] {
    match style_type {
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
    }
}

/// Variable collections, variables and styles defined in the file, in tree order.
#[derive(Default)]
struct Definitions {
    collections: Vec<JsonValue>,
    variables: Vec<JsonValue>,
    styles: Vec<JsonValue>,
}

impl Definitions {
    fn collect(document: &JsonValue, index: &Index) -> Self {
        let mut definitions = Self::default();
        definitions.add_node(document, index);
        definitions
    }

    fn add_node(&mut self, node: &JsonValue, index: &Index) {
        let Some(obj) = node.as_object() else { return };
        let id = obj.get("guid").and_then(guid_string).unwrap_or_default();

        match node_type(node) {
            "VARIABLE_SET" => self.collections.push(collection_definition(obj, id)),
            "VARIABLE" => self.variables.push(variable_definition(obj, id, index)),
            _ => {
                if let Some(style_type) = obj.get("styleType") {
                    self.styles.push(style_definition(obj, id, style_type));
                }
            }
        }

        if let Some(children) = obj.get("children").and_then(JsonValue::as_array) {
            for child in children {
                self.add_node(child, index);
            }
        }
    }
}

fn collection_definition(obj: &Map<String, JsonValue>, id: String) -> JsonValue {
    let modes: Vec<JsonValue> = obj
        .get("variableSetModes")
        .and_then(JsonValue::as_array)
        .map(|modes| {
            modes
                .iter()
                .map(|m| json!({"id": m.get("id").and_then(guid_string), "name": m.get("name")}))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": id,
        "name": obj.get("name"),
        "key": obj.get("key"),
        "sourceLibraryKey": obj.get("sourceLibraryKey"),
        "modes": modes,
    })
}

fn variable_definition(obj: &Map<String, JsonValue>, id: String, index: &Index) -> JsonValue {
    let mut values = Map::new();
    let entries = obj
        .get("variableDataValues")
        .and_then(|v| v.get("entries"))
        .and_then(JsonValue::as_array);
    for entry in entries.into_iter().flatten() {
        let mode = entry
            .get("modeID")
            .and_then(guid_string)
            .unwrap_or_default();
        let data = entry.get("variableData").unwrap_or(&JsonValue::Null);
        values.insert(mode, variable_value(data, index));
    }
    json!({
        "collectionId": index.variable_set.get(&id),
        "id": id,
        "name": obj.get("name"),
        "key": obj.get("key"),
        "type": obj.get("variableResolvedType"),
        "scopes": obj.get("variableScopes"),
        "description": obj.get("description"),
        "valuesByMode": values,
    })
}

fn style_definition(obj: &Map<String, JsonValue>, id: String, style_type: &JsonValue) -> JsonValue {
    let mut style = Map::new();
    style.insert("id".to_string(), JsonValue::String(id));
    style.insert("name".to_string(), json!(obj.get("name")));
    style.insert("key".to_string(), json!(obj.get("key")));
    style.insert("styleType".to_string(), style_type.clone());
    style.insert(
        "sourceLibraryKey".to_string(),
        json!(obj.get("sourceLibraryKey")),
    );
    for &field in style_fields(style_type.as_str().unwrap_or("")) {
        if let Some(v) = obj.get(field) {
            style.insert(field.to_string(), v.clone());
        }
    }
    if let Some(d) = obj
        .get("description")
        .filter(|d| d.as_str().is_some_and(|s| !s.is_empty()))
    {
        style.insert("description".to_string(), d.clone());
    }
    JsonValue::Object(style)
}

/// Copy each variable into its collection's `variables`, keyed by mode name; variables
/// without a known collection go into a trailing "(unknown collection)".
fn nest_variables(collections: &mut Vec<JsonValue>, variables: Vec<JsonValue>) {
    for collection in collections.iter_mut() {
        let vars: Vec<JsonValue> = {
            let cid = collection["id"].as_str().unwrap_or("");
            let mode_names = mode_names(collection);
            variables
                .iter()
                .filter(|v| v["collectionId"].as_str() == Some(cid))
                .map(|v| nested_variable(v, &mode_names))
                .collect()
        };
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
}

/// Mode id -> mode name for a collection definition.
fn mode_names(collection: &JsonValue) -> HashMap<&str, &str> {
    collection["modes"]
        .as_array()
        .map(|modes| {
            modes
                .iter()
                .filter_map(|m| Some((m["id"].as_str()?, m["name"].as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// A variable definition as listed in its collection: values keyed by mode NAME
/// ("Desktop (1440px)") instead of mode id, without `collectionId` or an empty
/// `description`.
fn nested_variable(variable: &JsonValue, mode_names: &HashMap<&str, &str>) -> JsonValue {
    let mut v = variable.clone();
    if let Some(values) = v.get_mut("valuesByMode").and_then(JsonValue::as_object_mut) {
        *values = std::mem::take(values)
            .into_iter()
            .map(|(mode, val)| {
                let name = mode_names.get(mode.as_str()).map(|n| n.to_string());
                (name.unwrap_or(mode), val)
            })
            .collect();
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("collectionId");
        if o.get("description")
            .is_some_and(|d| d.is_null() || d.as_str() == Some(""))
        {
            o.remove("description");
        }
    }
    v
}

/// Drop null fields (e.g. no `sourceLibraryKey` on local definitions).
fn tidy(value: &mut JsonValue) {
    if let Some(obj) = value.as_object_mut() {
        obj.retain(|_, v| !v.is_null());
    }
}

/// Add `uses` to every variable, collection (sum of its variables) and style, then
/// drop their null fields.
fn record_uses(
    collections: &mut [JsonValue],
    styles: &mut [JsonValue],
    uses: &HashMap<String, u64>,
) {
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
}

/// Annotate the document in place and return the file's variables and styles.
pub fn add_design_context(document: &mut JsonValue) -> Result<DesignDefinitions> {
    let index = Index::build(document);
    let Definitions {
        mut collections,
        variables,
        mut styles,
    } = Definitions::collect(document, &index);
    nest_variables(&mut collections, variables);

    let mut annotator = Annotator {
        index: &index,
        uses: HashMap::new(),
    };
    annotator.annotate(document, true, true);
    record_uses(&mut collections, &mut styles, &annotator.uses);

    let mut styles = JsonValue::Array(styles);
    crate::schema::simplify_text_properties(&mut styles)?;

    Ok(DesignDefinitions {
        variables: JsonValue::Array(collections),
        styles,
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
