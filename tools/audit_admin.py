#!/usr/bin/env python3
"""Audit the Admin UI page (and every component it uses) for anything an implementer would trip over."""
import collections, json, re, sys

sys.path.insert(0, sys.argv[3] if len(sys.argv) > 3 else ".")
import build_figma_cache as B  # noqa: E402

doc = json.load(open(sys.argv[1]))
cache = sys.argv[2]
root = doc["document"]
B.resolve_overrides(root)
ui = next(p for p in root["children"] if p["name"] == "UI")
idx = json.load(open(f"{cache}/admin/index.json"))
comp_idx = json.load(open(f"{cache}/admin/components/index.json"))
icons = json.load(open(f"{cache}/admin/icons/index.json"))
tok = json.load(open(f"{cache}/admin/TOKENS.json"))
nodes = {n["id"]: n for n, _ in B.walk(root) if n.get("id")}
used_comp = {c["id"] for c in comp_idx["components"]}
screen_of = {}
for s in idx["screens"]:
    for n, _ in B.walk(nodes[s["id"]]):
        screen_of[n.get("id")] = (s["flow"], s["name"], s["id"])

scope = [nodes[s["id"]] for s in idx["screens"]] + [nodes[c] for c in used_comp if c in nodes]
R = collections.defaultdict(collections.Counter)
EX = collections.defaultdict(list)


def ex(key, item, limit=8):
    if len(EX[key]) < limit and item not in EX[key]:
        EX[key].append(item)


def where(n):
    s = screen_of.get(n.get("id"))
    return f"{s[0]} / {s[1]} ({s[2]})" if s else "component"


for top in scope:
    for n, path in B.walk(top):
        t = n.get("type")
        R["node types"][t] += 1
        # --- unbound colors (no token)
        for key in ("fillPaints", "strokePaints"):
            for p in n.get(key) or []:
                if p.get("visible", True) is False:
                    continue
                pt = p.get("type", "SOLID")
                R["paint types"][pt] += 1
                if pt == "SOLID" and "colorVar" not in p and not n.get("fillStyle" if key == "fillPaints" else "strokeStyle"):
                    R["unbound colors"][p.get("color")] += 1
                    ex(f"unbound {p.get('color')}", f"{where(n)} → {t} '{n.get('name')}' ({n.get('id')})", 3)
                if pt.startswith("GRADIENT"):
                    ex("gradients", f"{where(n)} → '{n.get('name')}' ({n.get('id')})")
                if pt == "IMAGE":
                    ex("images", f"{where(n)} → '{n.get('name')}' {p.get('image', {}).get('filename')}")
        for e in n.get("effects") or []:
            if e.get("visible", True) is False:
                continue
            R["effects"][e.get("type")] += 1
            if not n.get("effectStyle") and "colorVar" not in json.dumps(e):
                R["unbound effects"][f"{e.get('type')} {e.get('color')} {(e.get('offset') or {}).get('x', 0)},{(e.get('offset') or {}).get('y', 0)} r{e.get('radius')} s{e.get('spread', 0)}"] += 1
        # --- text
        if t == "TEXT":
            fn = (n.get("fontName") or {})
            R["fonts"][f"{fn.get('family')} {fn.get('style')}"] += 1
            if not n.get("textStyle"):
                R["text without text style"][f"{fn.get('family')} {fn.get('style')} {n.get('fontSize')}/{n.get('lineHeight', 'auto')}"] += 1
                ex("text without style", f"{where(n)} → '{(n.get('textData') or {}).get('characters', '')[:30]}' ({n.get('id')})")
            if (n.get("textData") or {}).get("styleOverrideTable"):
                R["text"]["mixed-style text nodes"] += 1
            if n.get("textAutoResize"):
                R["text auto resize"][n["textAutoResize"]] += 1
            if n.get("textTruncation") == "ENDING":
                R["text"]["truncated (ellipsis)"] += 1
        # --- layout
        if n.get("stackMode") == "GRID":
            R["layout"]["CSS grid frames"] += 1
            ex("grid frames", f"{where(n)} → '{n.get('name')}' ({n.get('id')})")
        if n.get("stackMode") in ("HORIZONTAL", "VERTICAL"):
            R["layout"]["auto-layout frames"] += 1
        elif t in ("FRAME", "COMPONENT") and n.get("children") and n.get("stackMode") is None:
            R["layout"]["frames without auto-layout (absolute)"] += 1
            ex("absolute frames", f"{where(n)} → '{n.get('name')}' ({n.get('id')}) {len(n.get('children'))} children")
        if n.get("stackPositioning") == "ABSOLUTE":
            R["layout"]["absolutely positioned children"] += 1
        if (n.get("transform") or {}).get("rotation"):
            R["layout"]["rotated nodes"] += 1
            ex("rotated", f"{where(n)} → '{n.get('name')}' ({n.get('id')}) {n['transform'].get('rotation')}°")
        if n.get("scrollBehavior") in ("FIXED", "STICKY_SCROLLS"):
            R["layout"]["fixed/sticky"] += 1
            ex("fixed/sticky", f"{where(n)} → '{n.get('name')}' {n.get('scrollBehavior')}")
        if n.get("overflowDirection") or n.get("scrollDirection"):
            R["layout"]["scrolling frames"] += 1
        if n.get("layoutGrids"):
            R["layout"]["layout grids"] += 1
        if n.get("opacity") not in (None, 1, 1.0):
            R["misc"]["semi-transparent nodes"] += 1
        if n.get("blendMode") and n.get("blendMode") not in ("NORMAL", "PASS_THROUGH"):
            R["misc"]["blend modes"] += 1
            ex("blend", f"{where(n)} → '{n.get('name')}' {n.get('blendMode')}")
        if n.get("mask"):
            R["misc"]["masks"] += 1
            ex("masks", f"{where(n)} → '{n.get('name')}' ({n.get('id')})")
        if t in ("BOOLEAN_OPERATION", "ELLIPSE", "LINE", "STAR", "REGULAR_POLYGON"):
            R["shapes"][t] += 1
        if t == "VECTOR" and not any((n.get("componentId") or "") for _ in [0]):
            pass
        # --- instances
        if t == "INSTANCE":
            R["instances"]["total"] += 1
            if n.get("componentId") not in used_comp:
                R["instances"]["component not exported"] += 1
                ex("instance missing", f"{where(n)} → '{n.get('name')}' {n.get('componentId')}")
            for o in (n.get("symbolData") or {}).get("symbolOverrides") or []:
                if o.get("otherVariant"):
                    R["overrides"]["for other variants (ignore)"] += 1
                elif o.get("targetId"):
                    R["overrides"]["resolved"] += 1
                    for k in o:
                        if k not in ("guidPath", "targetId"):
                            R["override properties"][k] += 1
        # --- detached-looking frames (named like a component but not an instance)
        if t == "FRAME" and n.get("name") in {nodes[c].get("name") for c in []}:
            pass
        # --- prototype
        for i in n.get("prototypeInteractions") or []:
            R["interactions"][(i.get("event") or {}).get("interactionType")] += 1
        # --- variable modes
        vm = (n.get("variableModeBySetMap") or {}).get("entries")
        if vm:
            R["misc"]["frames with explicit variable mode (e.g. Dark)"] += 1
            ex("modes", f"{where(n)} → '{n.get('name')}' {json.dumps(vm)[:120]}")

# components used: are they from the Design System file?
ds = json.load(open(f"{cache}/design-system/components.json"))
ds_names = {s["name"].lower() for s in ds["componentSets"]} | {c["name"].lower() for c in ds["components"]}
for c in comp_idx["components"]:
    name = (c["componentSetName"] or c["componentName"]).lower()
    R["components by source"]["in Design System file" if name in ds_names else "NOT in Design System file"] += 1
    if name not in ds_names:
        ex("components not in DS", f"{c['componentSetName'] or '–'} / {c['componentName']} ({c['id']}, {c['source']})", 60)

# icons: every icon-looking component exported as svg?
icon_ids = {e["id"] for e in icons}
for c in comp_idx["components"]:
    nm = c["componentName"]
    if re.match(r"^(stroke|fill)/", nm) and c["id"] not in icon_ids:
        ex("icon without svg", f"{nm} ({c['id']})")

# states per flow from screen names
states = collections.defaultdict(list)
for s in idx["screens"]:
    states[s["flow"]].append(s["name"])

out = {"counts": {k: dict(v) for k, v in R.items()}, "examples": EX, "screens": states}
json.dump(out, open("/tmp/audit.json", "w"), indent=1, ensure_ascii=False)
for k, v in R.items():
    items = v.most_common(12)
    print(f"\n## {k}")
    for a, b in items:
        print(f"  {b:>6}  {a}")
for k, v in EX.items():
    if k.startswith("unbound #"):
        continue
    print(f"\n### {k}")
    for e in v:
        print("   -", e)
