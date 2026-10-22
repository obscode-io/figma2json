use crate::error::Result;
use serde_json::{Map, Value as JsonValue};
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::Path;

/// Transform image hash arrays to filename strings with extensions
///
/// Recursively traverses the JSON tree and transforms objects in "image" and
/// "imageThumbnail" fields by:
/// - Converting "hash" array of integers to hex-encoded "filename" string
/// - Detecting image format from file header (PNG, JPEG, WebP, GIF, SVG)
/// - Renaming physical files to include the appropriate extension
/// - Updating the filename in JSON to include the extension
/// - Removing the "hash" field
/// - Preserving all other fields (including "name")
///
/// # Arguments
/// * `tree` - The JSON tree to modify (usually the document root)
/// * `base_dir` - Base directory where image files are located (relative to output JSON)
///
/// # Returns
/// * `Ok(())` - Successfully transformed all image hashes
///
/// # Examples
/// ```no_run
/// use figma2json::schema::transform_image_hashes;
/// use serde_json::json;
/// use std::path::Path;
///
/// let mut tree = json!({
///     "image": {
///         "hash": [96, 73, 161, 122],
///         "name": "Amazon-beast"
///     }
/// });
/// transform_image_hashes(&mut tree, Path::new("/output/dir")).unwrap();
/// // tree now has "image": {"filename": "images/6049a17a.jpg", "name": "Amazon-beast"}
/// ```
pub fn transform_image_hashes(tree: &mut JsonValue, base_dir: &Path) -> Result<()> {
    transform_image_hashes_with(tree, |filename| {
        let file_path = base_dir.join(filename);
        let extension = detect_image_format(&file_path)?;
        let _ = fs::rename(&file_path, base_dir.join(format!("{filename}{extension}")));
        Some(extension)
    })
}

/// Transform image hash arrays to filename strings, asking `extension_of` for
/// each image's extension
///
/// Works like [`transform_image_hashes`] without touching the filesystem:
/// `extension_of` receives the extension-less filename of every image
/// reference (`images/<hex hash>`), in document order, and returns the
/// extension to append (such as `".png"`), or `None` to leave it bare. Use
/// [`image_extension`] to detect the extension from the image bytes, for
/// example when the images are held in memory.
///
/// # Examples
/// ```
/// use figma2json::schema::{image_extension, transform_image_hashes_with};
/// use serde_json::json;
/// use std::collections::HashMap;
///
/// let images = HashMap::from([("images/6049a17a", b"\x89PNG\r\n\x1a\n".to_vec())]);
/// let mut tree = json!({"image": {"hash": [96, 73, 161, 122], "name": "Logo"}});
/// transform_image_hashes_with(&mut tree, |filename| {
///     images.get(filename).and_then(|bytes| image_extension(bytes))
/// })
/// .unwrap();
/// assert_eq!(tree, json!({"image": {"filename": "images/6049a17a.png", "name": "Logo"}}));
/// ```
pub fn transform_image_hashes_with<F>(tree: &mut JsonValue, mut extension_of: F) -> Result<()>
where
    F: FnMut(&str) -> Option<&'static str>,
{
    transform_recursive(tree, &mut extension_of);
    Ok(())
}

/// Keys whose object values are image references.
const IMAGE_FIELDS: &[&str] = &["image", "imageThumbnail"];

const HEADER_LEN: usize = 256;

type ExtensionOf<'a> = dyn FnMut(&str) -> Option<&'static str> + 'a;

/// Walk the tree, rewriting image references in key order.
///
/// Each image field is rewritten just before its own subtree is visited, so
/// extensions are requested in document order.
fn transform_recursive(value: &mut JsonValue, extension_of: &mut ExtensionOf<'_>) {
    match value {
        JsonValue::Object(map) => {
            for (key, child) in map.iter_mut() {
                if IMAGE_FIELDS.contains(&key.as_str()) {
                    if let JsonValue::Object(image) = child {
                        transform_image(image, extension_of);
                    }
                }
                transform_recursive(child, extension_of);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                transform_recursive(item, extension_of);
            }
        }
        _ => {}
    }
}

/// Replace an image object's `hash` array with a `filename` carrying the
/// extension `extension_of` returns.
///
/// Objects whose `hash` is missing or not a valid byte array are left as-is.
fn transform_image(image: &mut Map<String, JsonValue>, extension_of: &mut ExtensionOf<'_>) {
    let Some(mut filename) = image
        .get("hash")
        .and_then(JsonValue::as_array)
        .and_then(|hash| hash_to_filename(hash))
    else {
        return;
    };

    if let Some(extension) = extension_of(&filename) {
        filename.push_str(extension);
    }

    image.remove("hash");
    image.insert("filename".to_owned(), JsonValue::String(filename));
}

/// Convert a hash array of integers to a filename string
///
/// Converts each integer to its 2-digit hex representation and concatenates
/// them with "images/" prefix.
///
/// # Returns
/// * `Some(String)` - The filename string (e.g., "images/6049a17a...")
/// * `None` - If any element is not a valid u8 integer
fn hash_to_filename(hash: &[JsonValue]) -> Option<String> {
    let mut filename = String::with_capacity("images/".len() + hash.len() * 2);
    filename.push_str("images/");
    for value in hash {
        let byte = u8::try_from(value.as_u64()?).ok()?;
        let _ = write!(filename, "{byte:02x}");
    }
    Some(filename)
}

/// Detect image format from file header (magic bytes)
///
/// Reads the first 256 bytes of a file and passes them to [`image_extension`].
///
/// # Returns
/// * `Some(&str)` - The file extension (".png", ".jpg", ".webp", ".gif" or ".svg")
/// * `None` - If format cannot be detected or file cannot be read
fn detect_image_format(file_path: &Path) -> Option<&'static str> {
    let mut buffer = [0u8; HEADER_LEN];
    let bytes_read = fs::File::open(file_path).ok()?.read(&mut buffer).ok()?;
    image_extension(&buffer[..bytes_read])
}

/// Detect an image's format from its leading bytes (magic numbers)
///
/// Only the first 256 bytes are looked at.
///
/// # Returns
/// * `Some(&str)` - The file extension (".png", ".jpg", ".webp", ".gif" or ".svg")
/// * `None` - If the format is not recognized
///
/// # Examples
/// ```
/// use figma2json::schema::image_extension;
///
/// assert_eq!(image_extension(b"GIF89a\x01\x00"), Some(".gif"));
/// assert_eq!(image_extension(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), Some(".svg"));
/// assert_eq!(image_extension(b"plain text"), None);
/// ```
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF];
    const GIF: &[u8] = b"GIF8";

    let header = &bytes[..bytes.len().min(HEADER_LEN)];

    if header.len() < 4 {
        return None;
    }
    if header.starts_with(PNG) {
        return Some(".png");
    }
    if header.starts_with(JPEG) {
        return Some(".jpg");
    }
    if header.starts_with(GIF) {
        return Some(".gif");
    }
    if header.len() >= 12 && header.starts_with(b"RIFF") && &header[8..12] == b"WEBP" {
        return Some(".webp");
    }
    if let Ok(text) = std::str::from_utf8(header) {
        let text = text.to_lowercase();
        if text.contains("<svg") || (text.contains("<?xml") && text.contains("svg")) {
            return Some(".svg");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_hash_to_filename() {
        let hash = vec![
            JsonValue::from(96),
            JsonValue::from(73),
            JsonValue::from(161),
            JsonValue::from(122),
        ];

        let filename = hash_to_filename(&hash).unwrap();
        assert_eq!(filename, "images/6049a17a");
    }

    #[test]
    fn test_hash_to_filename_full() {
        let hash = vec![
            JsonValue::from(96),
            JsonValue::from(73),
            JsonValue::from(161),
            JsonValue::from(122),
            JsonValue::from(132),
            JsonValue::from(131),
            JsonValue::from(226),
            JsonValue::from(80),
            JsonValue::from(226),
            JsonValue::from(150),
            JsonValue::from(78),
            JsonValue::from(100),
            JsonValue::from(84),
            JsonValue::from(218),
            JsonValue::from(142),
            JsonValue::from(231),
            JsonValue::from(161),
            JsonValue::from(69),
            JsonValue::from(66),
            JsonValue::from(133),
        ];

        let filename = hash_to_filename(&hash).unwrap();
        assert_eq!(filename, "images/6049a17a8483e250e2964e6454da8ee7a1454285");
    }

    #[test]
    fn test_hash_to_filename_invalid() {
        let hash = vec![JsonValue::from(256)];
        assert!(hash_to_filename(&hash).is_none());
    }

    #[test]
    fn test_transform_image_field() {
        let mut tree = json!({
            "name": "Rectangle",
            "image": {
                "hash": [96, 73, 161, 122],
                "name": "Amazon-beast"
            }
        });

        transform_image_hashes(&mut tree, std::path::Path::new(".")).unwrap();

        let image = tree.get("image").unwrap();
        assert!(image.get("hash").is_none());
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/6049a17a")
        );
        assert_eq!(image.get("name").unwrap().as_str(), Some("Amazon-beast"));
    }

    #[test]
    fn test_transform_image_thumbnail_field() {
        let mut tree = json!({
            "name": "Rectangle",
            "imageThumbnail": {
                "hash": [96, 73, 161, 122, 132, 131],
                "name": "Test-Image"
            }
        });

        transform_image_hashes(&mut tree, std::path::Path::new(".")).unwrap();

        let thumbnail = tree.get("imageThumbnail").unwrap();
        assert!(thumbnail.get("hash").is_none());
        assert_eq!(
            thumbnail.get("filename").unwrap().as_str(),
            Some("images/6049a17a8483")
        );
        assert_eq!(thumbnail.get("name").unwrap().as_str(), Some("Test-Image"));
    }

    #[test]
    fn test_transform_nested_objects() {
        let mut tree = json!({
            "name": "Root",
            "children": [
                {
                    "name": "Child1",
                    "image": {
                        "hash": [96, 73],
                        "name": "Image1"
                    }
                },
                {
                    "name": "Child2",
                    "fills": [
                        {
                            "image": {
                                "hash": [161, 122],
                                "name": "Image2"
                            }
                        }
                    ]
                }
            ]
        });

        transform_image_hashes(&mut tree, std::path::Path::new(".")).unwrap();

        let child1_image = &tree["children"][0]["image"];
        assert!(child1_image.get("hash").is_none());
        assert_eq!(
            child1_image.get("filename").unwrap().as_str(),
            Some("images/6049")
        );

        let child2_image = &tree["children"][1]["fills"][0]["image"];
        assert!(child2_image.get("hash").is_none());
        assert_eq!(
            child2_image.get("filename").unwrap().as_str(),
            Some("images/a17a")
        );
    }

    #[test]
    fn test_transform_preserves_other_fields() {
        let mut tree = json!({
            "name": "Rectangle",
            "visible": true,
            "image": {
                "hash": [96, 73, 161, 122],
                "name": "Amazon-beast",
                "width": 100,
                "height": 200
            },
            "x": 10,
            "y": 20
        });

        transform_image_hashes(&mut tree, std::path::Path::new(".")).unwrap();

        assert_eq!(tree.get("name").unwrap().as_str(), Some("Rectangle"));
        assert_eq!(tree.get("visible").unwrap().as_bool(), Some(true));
        assert_eq!(tree.get("x").unwrap().as_i64(), Some(10));
        assert_eq!(tree.get("y").unwrap().as_i64(), Some(20));

        let image = tree.get("image").unwrap();
        assert!(image.get("hash").is_none());
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/6049a17a")
        );
        assert_eq!(image.get("name").unwrap().as_str(), Some("Amazon-beast"));
        assert_eq!(image.get("width").unwrap().as_i64(), Some(100));
        assert_eq!(image.get("height").unwrap().as_i64(), Some(200));
    }

    #[test]
    fn test_transform_no_hash_field() {
        let mut tree = json!({
            "name": "Rectangle",
            "image": {
                "name": "Amazon-beast",
                "url": "https://example.com/image.png"
            }
        });

        transform_image_hashes(&mut tree, std::path::Path::new(".")).unwrap();

        let image = tree.get("image").unwrap();
        assert!(image.get("hash").is_none());
        assert!(image.get("filename").is_none());
        assert_eq!(image.get("name").unwrap().as_str(), Some("Amazon-beast"));
        assert_eq!(
            image.get("url").unwrap().as_str(),
            Some("https://example.com/image.png")
        );
    }

    #[test]
    fn test_transform_both_image_and_thumbnail() {
        let mut tree = json!({
            "name": "Rectangle",
            "image": {
                "hash": [96, 73],
                "name": "Main-Image"
            },
            "imageThumbnail": {
                "hash": [161, 122],
                "name": "Thumbnail"
            }
        });

        transform_image_hashes(&mut tree, std::path::Path::new(".")).unwrap();

        let image = tree.get("image").unwrap();
        assert!(image.get("hash").is_none());
        assert_eq!(image.get("filename").unwrap().as_str(), Some("images/6049"));
        assert_eq!(image.get("name").unwrap().as_str(), Some("Main-Image"));

        let thumbnail = tree.get("imageThumbnail").unwrap();
        assert!(thumbnail.get("hash").is_none());
        assert_eq!(
            thumbnail.get("filename").unwrap().as_str(),
            Some("images/a17a")
        );
        assert_eq!(thumbnail.get("name").unwrap().as_str(), Some("Thumbnail"));
    }

    #[test]
    fn test_transform_ignores_other_hash_fields() {
        let mut tree = json!({
            "name": "Node",
            "metadata": {
                "hash": [1, 2, 3, 4],
                "type": "checksum"
            },
            "image": {
                "hash": [96, 73],
                "name": "Real-Image"
            }
        });

        transform_image_hashes(&mut tree, std::path::Path::new(".")).unwrap();

        let metadata = tree.get("metadata").unwrap();
        assert!(metadata.get("hash").is_some());
        assert!(metadata.get("filename").is_none());

        let image = tree.get("image").unwrap();
        assert!(image.get("hash").is_none());
        assert_eq!(image.get("filename").unwrap().as_str(), Some("images/6049"));
    }

    #[test]
    fn test_detect_png_format() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join("figma2json_test_png");
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("images").join("6049a17a");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&test_file).unwrap();
        file.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
            .unwrap();
        file.write_all(&[0; 100]).unwrap();
        drop(file);

        let mut tree = json!({
            "image": {
                "hash": [96, 73, 161, 122],
                "name": "Test"
            }
        });

        transform_image_hashes(&mut tree, &temp_dir).unwrap();

        let image = tree.get("image").unwrap();
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/6049a17a.png")
        );

        assert!(temp_dir.join("images/6049a17a.png").exists());
        assert!(!test_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_jpeg_format() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join("figma2json_test_jpeg");
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("images").join("a17a6049");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&test_file).unwrap();
        file.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        file.write_all(&[0; 100]).unwrap();
        drop(file);

        let mut tree = json!({
            "image": {
                "hash": [161, 122, 96, 73],
                "name": "Test"
            }
        });

        transform_image_hashes(&mut tree, &temp_dir).unwrap();

        let image = tree.get("image").unwrap();
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/a17a6049.jpg")
        );

        assert!(temp_dir.join("images/a17a6049.jpg").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_gif_format() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join("figma2json_test_gif");
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("images").join("12345678");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&test_file).unwrap();
        file.write_all(b"GIF89a").unwrap();
        file.write_all(&[0; 100]).unwrap();
        drop(file);

        let mut tree = json!({
            "image": {
                "hash": [0x12, 0x34, 0x56, 0x78],
                "name": "Test"
            }
        });

        transform_image_hashes(&mut tree, &temp_dir).unwrap();

        let image = tree.get("image").unwrap();
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/12345678.gif")
        );

        assert!(temp_dir.join("images/12345678.gif").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_webp_format() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join("figma2json_test_webp");
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("images").join("abcdef12");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&test_file).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&[0, 0, 0, 0]).unwrap();
        file.write_all(b"WEBP").unwrap();
        file.write_all(&[0; 100]).unwrap();
        drop(file);

        let mut tree = json!({
            "image": {
                "hash": [0xab, 0xcd, 0xef, 0x12],
                "name": "Test"
            }
        });

        transform_image_hashes(&mut tree, &temp_dir).unwrap();

        let image = tree.get("image").unwrap();
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/abcdef12.webp")
        );

        assert!(temp_dir.join("images/abcdef12.webp").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_svg_format() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join("figma2json_test_svg");
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("images").join("87654321");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&test_file).unwrap();
        file.write_all(b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\">")
            .unwrap();
        file.write_all(&[0; 100]).unwrap();
        drop(file);

        let mut tree = json!({
            "image": {
                "hash": [0x87, 0x65, 0x43, 0x21],
                "name": "Test"
            }
        });

        transform_image_hashes(&mut tree, &temp_dir).unwrap();

        let image = tree.get("image").unwrap();
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/87654321.svg")
        );

        assert!(temp_dir.join("images/87654321.svg").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_unknown_format_keeps_no_extension() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join("figma2json_test_unknown");
        let _ = fs::create_dir_all(&temp_dir);

        let test_file = temp_dir.join("images").join("deadbeef");
        fs::create_dir_all(test_file.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&test_file).unwrap();
        file.write_all(b"UNKNOWN FORMAT").unwrap();
        file.write_all(&[0; 100]).unwrap();
        drop(file);

        let mut tree = json!({
            "image": {
                "hash": [0xde, 0xad, 0xbe, 0xef],
                "name": "Test"
            }
        });

        transform_image_hashes(&mut tree, &temp_dir).unwrap();

        let image = tree.get("image").unwrap();
        assert_eq!(
            image.get("filename").unwrap().as_str(),
            Some("images/deadbeef")
        );

        assert!(test_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multiple_images_different_formats() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join("figma2json_test_multi");
        let _ = fs::create_dir_all(&temp_dir);

        let png_file = temp_dir.join("images").join("6049");
        fs::create_dir_all(png_file.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&png_file).unwrap();
        file.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
            .unwrap();
        drop(file);

        let jpg_file = temp_dir.join("images").join("a17a");
        let mut file = fs::File::create(&jpg_file).unwrap();
        file.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        drop(file);

        let mut tree = json!({
            "children": [
                {
                    "image": {
                        "hash": [96, 73],
                        "name": "Image1"
                    }
                },
                {
                    "fills": [
                        {
                            "image": {
                                "hash": [161, 122],
                                "name": "Image2"
                            }
                        }
                    ]
                }
            ]
        });

        transform_image_hashes(&mut tree, &temp_dir).unwrap();

        let child1_image = &tree["children"][0]["image"];
        assert_eq!(
            child1_image.get("filename").unwrap().as_str(),
            Some("images/6049.png")
        );

        let child2_image = &tree["children"][1]["fills"][0]["image"];
        assert_eq!(
            child2_image.get("filename").unwrap().as_str(),
            Some("images/a17a.jpg")
        );

        assert!(temp_dir.join("images/6049.png").exists());
        assert!(temp_dir.join("images/a17a.jpg").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_image_extension_from_bytes() {
        let mut long_svg = vec![b' '; 300];
        long_svg.extend_from_slice(b"<svg>");
        let mut riff_not_webp = b"RIFF\0\0\0\0WAVE".to_vec();
        riff_not_webp.extend_from_slice(&[0; 8]);
        let cases: &[(&[u8], Option<&str>)] = &[
            (
                &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0],
                Some(".png"),
            ),
            (&[0xFF, 0xD8, 0xFF, 0xE0], Some(".jpg")),
            (b"GIF87a", Some(".gif")),
            (b"RIFF\0\0\0\0WEBPVP8 ", Some(".webp")),
            (b"<?XML version='1.0'?><!DOCTYPE SVG>", Some(".svg")),
            (b"<SVG>", Some(".svg")),
            (&long_svg, None),
            (&riff_not_webp, None),
            (b"GIF", None),
            (&[0xFF, 0xD8], None),
            (b"", None),
        ];
        for (bytes, expected) in cases {
            assert_eq!(image_extension(bytes), *expected, "{bytes:?}");
        }
    }

    #[test]
    fn test_transform_with_callback_sees_every_reference_in_order() {
        let mut tree = json!({
            "children": [
                {"image": {"hash": [1, 2], "name": "A"}},
                {"imageThumbnail": {"hash": [3], "name": "B"}, "image": {"hash": [1, 2]}},
                {"image": {"name": "no hash"}},
            ]
        });
        let mut seen = Vec::new();
        transform_image_hashes_with(&mut tree, |filename| {
            seen.push(filename.to_owned());
            (filename == "images/0102").then_some(".png")
        })
        .unwrap();

        assert_eq!(seen, ["images/0102", "images/0102", "images/03"]);
        assert_eq!(
            tree,
            json!({
                "children": [
                    {"image": {"filename": "images/0102.png", "name": "A"}},
                    {"imageThumbnail": {"filename": "images/03", "name": "B"}, "image": {"filename": "images/0102.png"}},
                    {"image": {"name": "no hash"}},
                ]
            })
        );
    }
}
