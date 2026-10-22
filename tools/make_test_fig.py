#!/usr/bin/env python3
"""Write a small synthetic `.fig` file for smoke tests and before/after output comparisons.

    python3 -I tools/make_test_fig.py OUT.fig          # bare canvas.fig (fig-kiwi header + chunks)
    python3 -I tools/make_test_fig.py OUT.fig --zip    # ZIP container, as Figma's "Save local copy" writes

The file is built from scratch with the standard library only: a hand-written Kiwi binary schema and a
Kiwi-encoded message, both compressed with raw DEFLATE, behind a `fig-kiwi` header with version 48.

The document is DOCUMENT 0:0 > CANVAS "Page 1" 0:1 > FRAME "Frame" 1:2 > VECTOR "Vector" 1:3. Every node
has `visible: true` and `opacity: 1.0` (defaults that the pipeline removes). The frame has one solid fill
(#ff8000). The vector references blob 0 (`commandsBlob`, path M 0 0 L 10 0 L 10 10 Z) and blob 1
(`vectorNetworkBlob`, 2 vertices, 1 segment, 1 region).

With `--zip` the frame also gets an IMAGE paint whose `image.hash` points at a 1x1 PNG stored as
`images/<hex hash>` in the archive, next to `canvas.fig`, so ZIP extraction and image renaming are
exercised too. The output is deterministic: the same arguments always produce the same bytes.
"""
import argparse
import struct
import sys
import zipfile
import zlib

TYPE_BOOL, TYPE_BYTE, TYPE_INT, TYPE_UINT, TYPE_FLOAT, TYPE_STRING = -1, -2, -3, -4, -5, -6
DEF_ENUM, DEF_STRUCT, DEF_MESSAGE = 0, 1, 2

IMAGE_HASH = bytes(range(0xA0, 0xB4))


def var_uint(value):
    """Kiwi variable-length unsigned 32-bit integer."""
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value == 0:
            out.append(byte)
            return bytes(out)
        out.append(byte | 0x80)


def var_int(value):
    """Kiwi variable-length signed 32-bit integer (zigzag encoded)."""
    return var_uint(((value << 1) ^ (value >> 31)) & 0xFFFFFFFF)


def var_float(value):
    """Kiwi variable-length float: the f32 bits rotated so the exponent comes first; zero is one byte."""
    bits = struct.unpack("<I", struct.pack("<f", value))[0]
    bits = ((bits >> 23) | (bits << 9)) & 0xFFFFFFFF
    if bits & 0xFF == 0:
        return b"\x00"
    return struct.pack("<I", bits)


def string(value):
    """Kiwi string: UTF-8 bytes followed by a NUL byte."""
    return value.encode("utf-8") + b"\x00"


SCHEMA = [
    ("NodeType", DEF_ENUM, [("DOCUMENT", 0, False, 1), ("CANVAS", 0, False, 2), ("FRAME", 0, False, 3),
                            ("VECTOR", 0, False, 4)]),
    ("GUID", DEF_STRUCT, [("sessionID", TYPE_UINT, False, 1), ("localID", TYPE_UINT, False, 2)]),
    ("ParentIndex", DEF_MESSAGE, [("guid", 1, False, 1), ("position", TYPE_STRING, False, 2)]),
    ("Color", DEF_STRUCT, [("r", TYPE_FLOAT, False, 1), ("g", TYPE_FLOAT, False, 2),
                           ("b", TYPE_FLOAT, False, 3), ("a", TYPE_FLOAT, False, 4)]),
    ("Paint", DEF_MESSAGE, [("color", 3, False, 1), ("opacity", TYPE_FLOAT, False, 2),
                            ("visible", TYPE_BOOL, False, 3), ("type", 8, False, 4), ("image", 9, False, 5)]),
    ("NodeChange", DEF_MESSAGE, [("guid", 1, False, 1), ("parentIndex", 2, False, 2), ("type", 0, False, 3),
                                 ("name", TYPE_STRING, False, 4), ("visible", TYPE_BOOL, False, 5),
                                 ("opacity", TYPE_FLOAT, False, 6), ("fillPaints", 4, True, 7),
                                 ("commandsBlob", TYPE_UINT, False, 8), ("vectorNetworkBlob", TYPE_UINT, False, 9)]),
    ("Blob", DEF_MESSAGE, [("bytes", TYPE_BYTE, True, 1)]),
    ("Message", DEF_MESSAGE, [("nodeChanges", 5, True, 1), ("blobs", 6, True, 2)]),
    ("PaintType", DEF_ENUM, [("SOLID", 0, False, 0), ("IMAGE", 0, False, 1)]),
    ("Image", DEF_MESSAGE, [("hash", TYPE_BYTE, True, 1), ("name", TYPE_STRING, False, 2)]),
]
DEF_INDEX = {name: i for i, (name, _, _) in enumerate(SCHEMA)}


def encode_schema():
    """Encode SCHEMA in Kiwi's binary schema format."""
    out = bytearray(var_uint(len(SCHEMA)))
    for name, kind, fields in SCHEMA:
        out += string(name) + bytes([kind]) + var_uint(len(fields))
        for field_name, type_id, is_array, value in fields:
            out += string(field_name) + var_int(type_id) + bytes([int(is_array)]) + var_uint(value)
    return bytes(out)


def encode_value(type_id, value):
    """Encode one value of a Kiwi type; messages are dicts, enums are variant names."""
    if type_id == TYPE_BOOL:
        return bytes([int(value)])
    if type_id == TYPE_BYTE:
        return bytes([value])
    if type_id == TYPE_INT:
        return var_int(value)
    if type_id == TYPE_UINT:
        return var_uint(value)
    if type_id == TYPE_FLOAT:
        return var_float(value)
    if type_id == TYPE_STRING:
        return string(value)
    _, kind, fields = SCHEMA[type_id]
    if kind == DEF_ENUM:
        return var_uint(next(v for name, _, _, v in fields if name == value))
    out = bytearray()
    for field_name, field_type, is_array, field_value in fields:
        if field_name not in value:
            if kind == DEF_STRUCT:
                raise ValueError(f"struct field {field_name} missing")
            continue
        if kind == DEF_MESSAGE:
            out += var_uint(field_value)
        item = value[field_name]
        if is_array:
            out += var_uint(len(item))
            for element in item:
                out += encode_value(field_type, element)
        else:
            out += encode_value(field_type, item)
    if kind == DEF_MESSAGE:
        out.append(0)
    return bytes(out)


def guid(session, local):
    return {"sessionID": session, "localID": local}


def node(node_guid, parent, node_type, name, **extra):
    """A NodeChange with the default `visible`/`opacity` the pipeline is expected to remove."""
    change = {"guid": node_guid, "type": node_type, "name": name, "visible": True, "opacity": 1.0}
    if parent is not None:
        change["parentIndex"] = {"guid": parent, "position": "!"}
    change.update(extra)
    return change


def commands_blob():
    """Path commands M 0 0 L 10 0 L 10 10 Z."""
    out = bytearray([1]) + struct.pack("<ff", 0, 0)
    for x, y in ((10, 0), (10, 10)):
        out += bytes([2]) + struct.pack("<ff", x, y)
    out.append(0)
    return bytes(out)


def vector_network_blob():
    """Two vertices, one segment between them and one region with a single loop over that segment."""
    out = bytearray(struct.pack("<III", 2, 1, 1))
    for x, y in ((0, 0), (5, 5)):
        out += struct.pack("<Iff", 0, x, y)
    out += struct.pack("<IIffIff", 0, 0, 0, 0, 1, 0, 0)
    out += struct.pack("<IIII", 3, 1, 1, 0)
    return bytes(out)


def message(with_image):
    """The Kiwi message: four nodes and two blobs."""
    paints = [{"color": {"r": 1.0, "g": 0.5, "b": 0.0, "a": 1.0}, "opacity": 1.0, "visible": True}]
    if with_image:
        paints.append({"type": "IMAGE", "opacity": 1.0, "visible": True,
                       "image": {"hash": list(IMAGE_HASH), "name": "pixel"}})
    return {
        "nodeChanges": [
            node(guid(0, 0), None, "DOCUMENT", "Document"),
            node(guid(0, 1), guid(0, 0), "CANVAS", "Page 1"),
            node(guid(1, 2), guid(0, 1), "FRAME", "Frame", fillPaints=paints),
            node(guid(1, 3), guid(1, 2), "VECTOR", "Vector", commandsBlob=0, vectorNetworkBlob=1),
        ],
        "blobs": [{"bytes": list(commands_blob())}, {"bytes": list(vector_network_blob())}],
    }


def png_1x1():
    """A valid 1x1 transparent RGBA PNG."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    header = struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)
    pixels = zlib.compress(b"\x00\x00\x00\x00\x00")
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", pixels) + chunk(b"IEND", b"")


def deflate_raw(data):
    """Raw DEFLATE (no zlib header), the older of the two chunk compressions Figma uses."""
    compressor = zlib.compressobj(level=6, wbits=-15)
    return compressor.compress(data) + compressor.flush()


def canvas_fig(with_image):
    """A complete bare .fig: header, version, then the length-prefixed schema and data chunks."""
    chunks = [deflate_raw(encode_schema()),
              deflate_raw(encode_value(DEF_INDEX["Message"], message(with_image)))]
    out = bytearray(b"fig-kiwi" + struct.pack("<I", 48))
    for chunk in chunks:
        out += struct.pack("<I", len(chunk)) + chunk
    return bytes(out)


def main():
    parser = argparse.ArgumentParser(description="Write a small synthetic .fig file.")
    parser.add_argument("output", help="path of the .fig file to write")
    parser.add_argument("--zip", action="store_true",
                        help="write a ZIP container with canvas.fig and an image, like Figma's local copy")
    args = parser.parse_args()
    if args.zip:
        fixed_time = (2020, 1, 1, 0, 0, 0)
        with zipfile.ZipFile(args.output, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr(zipfile.ZipInfo("canvas.fig", fixed_time), canvas_fig(True))
            archive.writestr(zipfile.ZipInfo(f"images/{IMAGE_HASH.hex()}", fixed_time), png_1x1())
    else:
        with open(args.output, "wb") as out:
            out.write(canvas_fig(False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
