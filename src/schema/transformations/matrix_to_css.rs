use super::walk::for_each_object_mut;
use crate::error::Result;
use serde_json::{json, Map, Value as JsonValue};
use std::f64::consts::PI;

/// Transform 2D affine transformation matrices to CSS positioning properties
///
/// Recursively traverses the JSON tree and transforms "transform" objects by:
/// - Decomposing the matrix [m00, m01, m02, m10, m11, m12] into CSS properties
/// - Replacing matrix fields with: x, y, rotation, scaleX, scaleY, skewX
///
/// The decomposition follows the standard 2D affine transformation breakdown:
/// - Translation: x = m02, y = m12
/// - Scale and rotation extracted from the linear transformation matrix
/// - Skew computed from the remaining components
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
///
/// # Returns
/// * `Ok(())` - Successfully transformed all matrix transforms
///
/// # Examples
/// ```no_run
/// use figma2json::schema::transform_matrix_to_css;
/// use serde_json::json;
///
/// let mut tree = json!({
///     "transform": {
///         "m00": 1.0,
///         "m01": 0.0,
///         "m02": 100.0,
///         "m10": 0.0,
///         "m11": 1.0,
///         "m12": 50.0
///     }
/// });
/// transform_matrix_to_css(&mut tree).unwrap();
/// // tree now has "transform": {"x": 100.0, "y": 50.0, "rotation": 0.0, ...}
/// ```
pub fn transform_matrix_to_css(tree: &mut JsonValue) -> Result<()> {
    for_each_object_mut(tree, &mut |map| {
        let css_transform = map
            .get("transform")
            .and_then(JsonValue::as_object)
            .filter(|transform| has_matrix_fields(transform))
            .and_then(extract_and_decompose_matrix);
        if let Some(css_transform) = css_transform {
            map.insert("transform".to_owned(), css_transform);
        }
    });
    Ok(())
}

/// The six affine matrix components a `transform` object must carry.
const MATRIX_FIELDS: &[&str] = &["m00", "m01", "m02", "m10", "m11", "m12"];

/// Check if an object has all required matrix fields
fn has_matrix_fields(obj: &Map<String, JsonValue>) -> bool {
    MATRIX_FIELDS.iter().all(|field| obj.contains_key(*field))
}

/// Extract matrix values and decompose into CSS properties
///
/// Only includes properties that differ from their default values:
/// - x, y: always included
/// - rotation: only if not ~0.0
/// - scaleX: only if not ~1.0
/// - scaleY: only if not ~1.0
/// - skewX: only if not ~0.0
fn extract_and_decompose_matrix(obj: &Map<String, JsonValue>) -> Option<JsonValue> {
    let m00 = obj.get("m00")?.as_f64()?;
    let m01 = obj.get("m01")?.as_f64()?;
    let m02 = obj.get("m02")?.as_f64()?;
    let m10 = obj.get("m10")?.as_f64()?;
    let m11 = obj.get("m11")?.as_f64()?;
    let m12 = obj.get("m12")?.as_f64()?;

    let css = decompose_matrix(m00, m01, m02, m10, m11, m12);

    let mut result = Map::new();

    result.insert("x".to_owned(), json!(css.x));
    result.insert("y".to_owned(), json!(css.y));

    const EPSILON: f64 = 1e-10;

    if css.rotation.abs() > EPSILON {
        result.insert("rotation".to_owned(), json!(css.rotation));
    }

    if (css.scale_x - 1.0).abs() > EPSILON {
        result.insert("scaleX".to_owned(), json!(css.scale_x));
    }

    if (css.scale_y - 1.0).abs() > EPSILON {
        result.insert("scaleY".to_owned(), json!(css.scale_y));
    }

    if css.skew_x.abs() > EPSILON {
        result.insert("skewX".to_owned(), json!(css.skew_x));
    }

    Some(JsonValue::Object(result))
}

/// CSS transform properties
#[derive(Debug)]
struct CssTransform {
    x: f64,
    y: f64,
    /// Rotation in degrees.
    rotation: f64,
    scale_x: f64,
    scale_y: f64,
    /// Horizontal skew in degrees.
    skew_x: f64,
}

/// Decompose a 2D affine transformation matrix into CSS properties
///
/// Matrix format:
/// [m00  m01  m02]   [a  c  tx]
/// [m10  m11  m12] = [b  d  ty]
/// [0    0    1  ]   [0  0  1 ]
///
/// Decomposition algorithm:
/// 1. Translation: tx, ty are directly m02, m12
/// 2. Compute scale_x from the magnitude of the first column
/// 3. Compute rotation from the angle of the first column
/// 4. Compute scale_y from the determinant divided by scale_x
/// 5. Compute skew_x from the dot product of columns
fn decompose_matrix(m00: f64, m01: f64, m02: f64, m10: f64, m11: f64, m12: f64) -> CssTransform {
    let x = m02;
    let y = m12;

    let scale_x = (m00 * m00 + m10 * m10).sqrt();

    let rotation_rad = m10.atan2(m00);
    let rotation = rotation_rad * (180.0 / PI);

    let determinant = m00 * m11 - m01 * m10;
    let scale_y = if scale_x.abs() > 1e-10 {
        determinant / scale_x
    } else {
        (m01 * m01 + m11 * m11).sqrt()
    };

    let skew_x_rad = if scale_x.abs() > 1e-10 {
        let dot_product = m00 * m01 + m10 * m11;
        let scale_x_squared = m00 * m00 + m10 * m10;
        (dot_product / scale_x_squared).atan()
    } else {
        0.0
    };
    let skew_x = skew_x_rad * (180.0 / PI);

    CssTransform {
        x,
        y,
        rotation,
        scale_x,
        scale_y,
        skew_x,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn approx_eq(a: f64, b: f64, epsilon: f64) -> bool {
        (a - b).abs() < epsilon
    }

    #[test]
    fn test_identity_matrix() {
        let mut tree = json!({
            "transform": {
                "m00": 1.0,
                "m01": 0.0,
                "m02": 0.0,
                "m10": 0.0,
                "m11": 1.0,
                "m12": 0.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(transform.get("rotation").is_none());
        assert!(transform.get("scaleX").is_none());
        assert!(transform.get("scaleY").is_none());
        assert!(transform.get("skewX").is_none());
    }

    #[test]
    fn test_pure_translation() {
        let mut tree = json!({
            "transform": {
                "m00": 1.0,
                "m01": 0.0,
                "m02": 100.0,
                "m10": 0.0,
                "m11": 1.0,
                "m12": 50.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 100.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), 50.0, 1e-10));
        assert!(transform.get("rotation").is_none());
        assert!(transform.get("scaleX").is_none());
        assert!(transform.get("scaleY").is_none());
        assert!(transform.get("skewX").is_none());
    }

    #[test]
    fn test_pure_scale() {
        let mut tree = json!({
            "transform": {
                "m00": 2.0,
                "m01": 0.0,
                "m02": 0.0,
                "m10": 0.0,
                "m11": 3.0,
                "m12": 0.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(transform["scaleX"].as_f64().unwrap(), 2.0, 1e-10));
        assert!(approx_eq(transform["scaleY"].as_f64().unwrap(), 3.0, 1e-10));
        assert!(transform.get("rotation").is_none());
        assert!(transform.get("skewX").is_none());
    }

    #[test]
    fn test_pure_rotation_45_degrees() {
        let cos45 = std::f64::consts::FRAC_1_SQRT_2;
        let sin45 = std::f64::consts::FRAC_1_SQRT_2;

        let mut tree = json!({
            "transform": {
                "m00": cos45,
                "m01": -sin45,
                "m02": 0.0,
                "m10": sin45,
                "m11": cos45,
                "m12": 0.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(
            transform["rotation"].as_f64().unwrap(),
            45.0,
            1e-8
        ));
        assert!(transform.get("scaleX").is_none());
        assert!(transform.get("scaleY").is_none());
        assert!(transform.get("skewX").is_none());
    }

    #[test]
    fn test_pure_rotation_90_degrees() {
        let mut tree = json!({
            "transform": {
                "m00": 0.0,
                "m01": -1.0,
                "m02": 0.0,
                "m10": 1.0,
                "m11": 0.0,
                "m12": 0.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(
            transform["rotation"].as_f64().unwrap(),
            90.0,
            1e-8
        ));
        assert!(transform.get("scaleX").is_none());
        assert!(transform.get("scaleY").is_none());
        assert!(transform.get("skewX").is_none());
    }

    #[test]
    fn test_combined_translation_scale_rotation() {
        let angle = 30.0 * PI / 180.0;
        let cos_a = angle.cos();
        let sin_a = angle.sin();
        let sx = 2.0;
        let sy = 3.0;

        let mut tree = json!({
            "transform": {
                "m00": sx * cos_a,
                "m01": -sy * sin_a,
                "m02": 100.0,
                "m10": sx * sin_a,
                "m11": sy * cos_a,
                "m12": 50.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 100.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), 50.0, 1e-10));
        assert!(approx_eq(
            transform["rotation"].as_f64().unwrap(),
            30.0,
            1e-8
        ));
        assert!(approx_eq(transform["scaleX"].as_f64().unwrap(), 2.0, 1e-10));
        assert!(approx_eq(transform["scaleY"].as_f64().unwrap(), 3.0, 1e-10));
        assert!(transform.get("skewX").is_none());
    }

    #[test]
    fn test_nested_objects() {
        let mut tree = json!({
            "name": "Root",
            "transform": {
                "m00": 1.0,
                "m01": 0.0,
                "m02": 10.0,
                "m10": 0.0,
                "m11": 1.0,
                "m12": 20.0
            },
            "children": [
                {
                    "name": "Child1",
                    "transform": {
                        "m00": 2.0,
                        "m01": 0.0,
                        "m02": 5.0,
                        "m10": 0.0,
                        "m11": 2.0,
                        "m12": 10.0
                    }
                }
            ]
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let root_transform = tree.get("transform").unwrap();
        assert!(approx_eq(
            root_transform["x"].as_f64().unwrap(),
            10.0,
            1e-10
        ));
        assert!(approx_eq(
            root_transform["y"].as_f64().unwrap(),
            20.0,
            1e-10
        ));
        assert!(root_transform.get("rotation").is_none());
        assert!(root_transform.get("scaleX").is_none());
        assert!(root_transform.get("scaleY").is_none());
        assert!(root_transform.get("skewX").is_none());

        let child_transform = &tree["children"][0]["transform"];
        assert!(approx_eq(
            child_transform["x"].as_f64().unwrap(),
            5.0,
            1e-10
        ));
        assert!(approx_eq(
            child_transform["y"].as_f64().unwrap(),
            10.0,
            1e-10
        ));
        assert!(approx_eq(
            child_transform["scaleX"].as_f64().unwrap(),
            2.0,
            1e-10
        ));
        assert!(approx_eq(
            child_transform["scaleY"].as_f64().unwrap(),
            2.0,
            1e-10
        ));
        assert!(child_transform.get("rotation").is_none());
        assert!(child_transform.get("skewX").is_none());
    }

    #[test]
    fn test_non_transform_object_unchanged() {
        let mut tree = json!({
            "name": "Rectangle",
            "position": {
                "x": 10,
                "y": 20
            }
        });

        let original = tree.clone();
        transform_matrix_to_css(&mut tree).unwrap();

        assert_eq!(tree, original);
    }

    #[test]
    fn test_transform_without_matrix_fields_unchanged() {
        let mut tree = json!({
            "transform": {
                "x": 10,
                "y": 20
            }
        });

        let original = tree.clone();
        transform_matrix_to_css(&mut tree).unwrap();

        assert_eq!(tree, original);
    }

    #[test]
    fn test_negative_scale() {
        let mut tree = json!({
            "transform": {
                "m00": -1.0,
                "m01": 0.0,
                "m02": 0.0,
                "m10": 0.0,
                "m11": 1.0,
                "m12": 0.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), 0.0, 1e-10));
        assert!(approx_eq(
            transform["rotation"].as_f64().unwrap(),
            180.0,
            1e-8
        ));
        assert!(approx_eq(
            transform["scaleY"].as_f64().unwrap(),
            -1.0,
            1e-10
        ));
        assert!(transform.get("scaleX").is_none());
        assert!(transform.get("skewX").is_none());
    }

    #[test]
    fn test_real_world_example() {
        let mut tree = json!({
            "transform": {
                "m00": 1.0,
                "m01": 0.0,
                "m02": 248.0,
                "m10": 0.0,
                "m11": 1.0,
                "m12": -7.0
            }
        });

        transform_matrix_to_css(&mut tree).unwrap();

        let transform = tree.get("transform").unwrap();
        assert!(approx_eq(transform["x"].as_f64().unwrap(), 248.0, 1e-10));
        assert!(approx_eq(transform["y"].as_f64().unwrap(), -7.0, 1e-10));
        assert!(transform.get("rotation").is_none());
        assert!(transform.get("scaleX").is_none());
        assert!(transform.get("scaleY").is_none());
        assert!(transform.get("skewX").is_none());
    }
}
