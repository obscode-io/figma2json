use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use figma2json::ConvertOptions;
use figma2json_web::{Conversion, Options, JSON_NAME, RAW_JSON_NAME};
use kiwi_schema::{Def, DefKind, Field, Schema, Value as Kiwi, TYPE_BYTE, TYPE_STRING, TYPE_UINT};

const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
const SVG: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg'/>";

fn field(name: &str, type_id: i32, is_array: bool, value: u32) -> Field {
    Field {
        name: name.to_owned(),
        type_id,
        is_array,
        value,
    }
}

fn guid(local_id: u32) -> Kiwi<'static> {
    Kiwi::Object(
        "GUID",
        HashMap::from([
            ("sessionID", Kiwi::UInt(0)),
            ("localID", Kiwi::UInt(local_id)),
        ]),
    )
}

fn node(local_id: u32, parent: Option<u32>, kind: &'static str, paints: &[&[u8]]) -> Kiwi<'static> {
    let mut fields = HashMap::from([
        ("guid", guid(local_id)),
        ("type", Kiwi::Enum("NodeType", kind)),
        ("name", Kiwi::String(kind.to_lowercase())),
    ]);
    if let Some(parent) = parent {
        let index = HashMap::from([
            ("guid", guid(parent)),
            ("position", Kiwi::String("!".into())),
        ]);
        fields.insert("parentIndex", Kiwi::Object("ParentIndex", index));
    }
    if !paints.is_empty() {
        let paints = paints
            .iter()
            .map(|hash| {
                let hash = Kiwi::Array(hash.iter().copied().map(Kiwi::Byte).collect());
                let image = Kiwi::Object("Image", HashMap::from([("hash", hash)]));
                Kiwi::Object("Paint", HashMap::from([("image", image)]))
            })
            .collect();
        fields.insert("fillPaints", Kiwi::Array(paints));
    }
    Kiwi::Object("NodeChange", fields)
}

fn canvas_fig() -> Vec<u8> {
    let schema = Schema::new(vec![
        Def::new(
            "NodeType".into(),
            DefKind::Enum,
            vec![
                field("DOCUMENT", 0, false, 1),
                field("CANVAS", 0, false, 2),
                field("FRAME", 0, false, 3),
            ],
        ),
        Def::new(
            "GUID".into(),
            DefKind::Struct,
            vec![
                field("sessionID", TYPE_UINT, false, 1),
                field("localID", TYPE_UINT, false, 2),
            ],
        ),
        Def::new(
            "ParentIndex".into(),
            DefKind::Message,
            vec![
                field("guid", 1, false, 1),
                field("position", TYPE_STRING, false, 2),
            ],
        ),
        Def::new(
            "Image".into(),
            DefKind::Message,
            vec![field("hash", TYPE_BYTE, true, 1)],
        ),
        Def::new(
            "Paint".into(),
            DefKind::Message,
            vec![field("image", 3, false, 1)],
        ),
        Def::new(
            "NodeChange".into(),
            DefKind::Message,
            vec![
                field("guid", 1, false, 1),
                field("parentIndex", 2, false, 2),
                field("type", 0, false, 3),
                field("name", TYPE_STRING, false, 4),
                field("fillPaints", 4, true, 5),
            ],
        ),
        Def::new(
            "Blob".into(),
            DefKind::Message,
            vec![field("bytes", TYPE_BYTE, true, 1)],
        ),
        Def::new(
            "Message".into(),
            DefKind::Message,
            vec![field("nodeChanges", 5, true, 1), field("blobs", 6, true, 2)],
        ),
    ]);
    let message = Kiwi::Object(
        "Message",
        HashMap::from([
            (
                "nodeChanges",
                Kiwi::Array(vec![
                    node(0, None, "DOCUMENT", &[]),
                    node(1, Some(0), "CANVAS", &[]),
                    node(2, Some(1), "FRAME", &[&[0xaa, 0x01], &[0xaa, 0x02]]),
                ]),
            ),
            ("blobs", Kiwi::Array(Vec::new())),
        ]),
    );

    let mut file = b"fig-kiwi".to_vec();
    file.extend(48u32.to_le_bytes());
    for chunk in [schema.encode(), message.encode(&schema)] {
        let mut deflate =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        deflate.write_all(&chunk).unwrap();
        let chunk = deflate.finish().unwrap();
        file.extend(u32::try_from(chunk.len()).unwrap().to_le_bytes());
        file.extend(chunk);
    }
    file
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in entries {
        writer
            .start_file(*name, zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).unwrap();
            (file.name().to_owned(), contents)
        })
        .collect()
}

fn fig_container() -> Vec<u8> {
    zip(&[
        ("canvas.fig", &canvas_fig()),
        ("images/aa01", PNG),
        ("images/aa02", SVG),
        ("meta.json", b"{}"),
    ])
}

#[test]
fn converts_a_zip_container_like_the_cli() {
    let canvas = canvas_fig();
    let container = fig_container();
    let dir = std::env::temp_dir().join(format!("figma2json-web-test-{}", std::process::id()));
    figma2json::parser::extract_zip_to_directory(&container, &dir).unwrap();
    let cli_json =
        figma2json::convert_with_options(&canvas, Some(&dir), &ConvertOptions::default()).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let conversion = Conversion::new(&container, &Options::new()).unwrap();
    assert_eq!(
        conversion.json(),
        serde_json::to_string_pretty(&cli_json).unwrap()
    );
    assert!(conversion
        .json()
        .contains("\"filename\": \"images/aa01.png\""));
    assert!(conversion
        .json()
        .contains("\"filename\": \"images/aa02.svg\""));
    assert_eq!(
        conversion.file_names(),
        ["images/aa01.png", "images/aa02.svg", "meta.json"]
    );
    assert_eq!(conversion.image_count(), 2);
    assert_eq!(conversion.file_contents(0).as_deref(), Some(PNG));
    assert_eq!(conversion.file_contents(3), None);
    assert_eq!(conversion.raw_json(), None);
}

#[test]
fn zip_holds_the_json_and_every_file() {
    let options = Options {
        raw: true,
        ..Options::new()
    };
    let conversion = Conversion::new(&fig_container(), &options).unwrap();
    let raw_json = conversion.raw_json().unwrap();
    let raw: serde_json::Value = serde_json::from_str(&raw_json).unwrap();
    assert_eq!(raw["fileType"], "figma");

    let entries = unzip(&conversion.zip().unwrap());
    let names: Vec<_> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            JSON_NAME,
            RAW_JSON_NAME,
            "images/aa01.png",
            "images/aa02.svg",
            "meta.json"
        ]
    );
    assert_eq!(entries[0].1, conversion.json().into_bytes());
    assert_eq!(entries[1].1, raw_json.into_bytes());
    assert_eq!(entries[2].1, PNG);
    assert_eq!(entries[3].1, SVG);
}

#[test]
fn options_map_to_the_cli_flags() {
    let canvas = canvas_fig();
    for (legacy, keep_internal) in [(false, false), (true, false), (false, true), (true, true)] {
        let options = Options {
            compact: true,
            legacy,
            keep_internal,
            raw: false,
        };
        let conversion = Conversion::new(&canvas, &options).unwrap();
        let expected = figma2json::convert_with_options(
            &canvas,
            Some(std::path::Path::new("/nonexistent-figma2json-web-test-dir")),
            &ConvertOptions {
                legacy,
                keep_internal,
            },
        )
        .unwrap();
        assert_eq!(conversion.json(), serde_json::to_string(&expected).unwrap());
        assert!(!conversion.json().contains('\n'));
        assert!(conversion.file_names().is_empty());
        assert_eq!(unzip(&conversion.zip().unwrap()).len(), 1);
    }
}

#[test]
fn reports_errors_as_messages() {
    let error = Conversion::new(b"not a figma file", &Options::new()).unwrap_err();
    assert!(error.starts_with("Invalid magic header"), "{error}");

    let error = Conversion::new(&zip(&[("readme.txt", b"x")]), &Options::new()).unwrap_err();
    assert_eq!(error, "Canvas file not found in ZIP archive");
}

#[test]
fn reports_the_crate_version() {
    assert_eq!(figma2json_web::version(), env!("CARGO_PKG_VERSION"));
}
