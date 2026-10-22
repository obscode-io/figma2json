//! Shared traversal helpers for the transformation passes.

use serde_json::{Map, Value as JsonValue};

/// Call `f` on every JSON object in `value`, depth-first.
///
/// Each object is visited before the values it contains, so anything `f`
/// removes from an object is never visited, and anything it inserts is.
pub(crate) fn for_each_object_mut<F>(value: &mut JsonValue, f: &mut F)
where
    F: FnMut(&mut Map<String, JsonValue>),
{
    match value {
        JsonValue::Object(map) => {
            f(map);
            for child in map.values_mut() {
                for_each_object_mut(child, f);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                for_each_object_mut(item, f);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn visits_nested_objects_and_arrays() {
        let mut tree = json!({"a": {"b": [{"c": 1}, 2]}, "d": [[{"e": 3}]]});
        let mut count = 0;
        for_each_object_mut(&mut tree, &mut |_| count += 1);
        assert_eq!(count, 4);
    }

    #[test]
    fn removed_values_are_not_visited() {
        let mut tree = json!({"skip": {"inner": {}}, "keep": {}});
        let mut count = 0;
        for_each_object_mut(&mut tree, &mut |map| {
            map.remove("skip");
            count += 1;
        });
        assert_eq!(count, 2);
    }
}
