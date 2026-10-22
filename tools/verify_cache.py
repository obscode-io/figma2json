#!/usr/bin/env python3
"""Check a Figma cache folder built by build_figma_cache.py.

    python3 tools/verify_cache.py            # from the cache root

Checks: all JSON parses; every file referenced by an index or a .md exists; every instance resolves to an
exported component; every token, bound variable and style name used in node files exists in TOKENS.json;
no internal Figma leftovers. Exits 1 on any problem.
"""
import collections, glob, json, os, re, sys

problems, warnings, stats = [], [], collections.Counter()


def load(path):
    try:
        return json.load(open(path))
    except Exception as e:  # noqa: BLE001
        problems.append(f"invalid JSON: {path}: {e}")
        return None


REF_KEYS = ("componentId", "overriddenSymbolID", "symbolIdValue")


def check_refs(folder, node_files, component_ids, known_missing=()):
    tok = load(f"{folder}/TOKENS.json") or {}
    var_by_id = {v["id"]: v["name"] for c in tok.get("collections", []) for v in c["variables"]}
    style_names = {s["name"] for s in tok.get("styles", [])}

    def check_binding(b, f):
        if not isinstance(b, dict):
            return
        if "expression" in b:
            for arg in b.get("args", []):
                check_binding(arg, f)
            return
        if "variableId" in b:
            stats[f"{folder}: token refs"] += 1
            vid, name = b["variableId"], b.get("variable")
            if vid not in var_by_id:
                problems.append(f"{f}: token {name!r} ({vid}) not in TOKENS.json")
            elif name and var_by_id[vid] != name:
                problems.append(f"{f}: token {vid} is {var_by_id[vid]!r} in TOKENS.json, not {name!r}")

    def walk(x, f):
        if isinstance(x, dict):
            if x.get("type") == "INSTANCE":
                stats[f"{folder}: instances"] += 1
            for k in REF_KEYS:
                if isinstance(x.get(k), str):
                    stats[f"{folder}: component refs (instances + swaps)"] += 1
                    if x[k] in known_missing:
                        warnings.append(f"{f}: {k} {x[k]} is not defined anywhere in the Figma file (listed as notFound)")
                    elif x[k] not in component_ids:
                        problems.append(f"{f}: {k} {x[k]} (in {x.get('id', '?')}) not exported")
            if "variableId" in x:
                check_binding(x, f)
            for b in (x.get("boundVariables") or {}).values():
                check_binding(b, f)
            for k in ("textStyle", "fillStyle", "effectStyle", "strokeStyle", "gridStyle"):
                if isinstance(x.get(k), str):
                    stats[f"{folder}: style refs"] += 1
                    if x[k] not in style_names:
                        problems.append(f"{f}: style {x[k]!r} not in TOKENS.json")
            for k, v in x.items():
                if k != "boundVariables":
                    walk(v, f)
        elif isinstance(x, list):
            for v in x:
                walk(v, f)

    for f in node_files:
        data = load(f)
        if data is None:
            continue
        raw = json.dumps(data)
        for leftover in ("__enum__", '"colorVar"', '"parameterConsumptionMap"', '"variableConsumptionMap"',
                         '"derivedSymbolData"', '"guid"', '"styleIdFor', '"assetRef": {}'):
            if leftover in raw:
                problems.append(f"{f}: contains {leftover}")
        walk(data, f)


def component_ids_in(files):
    ids = set()
    def collect(x):
        if isinstance(x, dict):
            if x.get("type") == "COMPONENT":
                ids.add(x.get("id"))
            for v in x.values():
                collect(v)
        elif isinstance(x, list):
            for v in x:
                collect(v)
    for f in files:
        collect(load(f))
    return ids


for f in glob.glob("**/*.json", recursive=True):
    load(f)
    stats["json files"] += 1

for md in glob.glob("**/*.md", recursive=True):
    base = os.path.dirname(md)
    for p in re.findall(r"`((?:flows|components|sections|library-components)/[^`]+\.json)`", open(md).read()):
        if not os.path.exists(os.path.join(base, p)):
            problems.append(f"{md}: missing {p}")

if os.path.isdir("design-system"):
    dc = load("design-system/components.json")
    files = [f"design-system/{c['file']}" for c in dc["componentSets"] + dc["components"]]
    for f in files:
        if not os.path.exists(f):
            problems.append(f"missing {f}")
    lib = glob.glob("design-system/library-components/*-*.json")
    lib_index = load("design-system/library-components/index.json") if os.path.exists("design-system/library-components/index.json") else {}
    check_refs("design-system", files + lib, component_ids_in(files + lib), set((lib_index or {}).get("notFound", [])))

if os.path.isdir("admin"):
    idx = load("admin/index.json")
    screens = [f"admin/{s['file']}" for s in idx["screens"]]
    for f in screens:
        if not os.path.exists(f):
            problems.append(f"missing {f}")
    comps = glob.glob("admin/components/*-*.json")
    comp_index = load("admin/components/index.json") or {}
    check_refs("admin", screens + comps, component_ids_in(comps), set(comp_index.get("notFound", [])))

import xml.etree.ElementTree as ET
for index in glob.glob("**/icons/index.json", recursive=True) + glob.glob("**/*-icons/index.json", recursive=True):
    base = os.path.dirname(os.path.dirname(index))
    for e in load(index) or []:
        f = os.path.join(base, e["file"])
        if not os.path.exists(f):
            problems.append(f"{index}: missing {e['file']}")
            continue
        try:
            ET.parse(f)
            stats["svg icons/graphics"] += 1
        except ET.ParseError as err:
            problems.append(f"{f}: invalid SVG: {err}")

for k, v in sorted(stats.items()):
    print(f"{k:<34}{v}")
for w in sorted(set(warnings)):
    print("  warning:", w)
print(f"\n{len(problems)} problem(s)")
for p in problems[:50]:
    print("  -", p)
sys.exit(1 if problems else 0)
