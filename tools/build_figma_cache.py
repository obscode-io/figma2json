#!/usr/bin/env python3
"""Turn figma2json (0.4+) output into an agent-friendly cache.

    python3 build_figma_cache.py canvas.json OUT_DIR --kind design-system
    python3 build_figma_cache.py canvas.json OUT_DIR --kind screens --page UI

design-system: tokens, styles, component sets (one file each, big sets split per variant), sections.
screens:       one file per screen, grouped by flow (the Sections on the chosen page), prototype links,
               tokens/styles/components the screens actually use.

Every node file is pruned for reading by an agent:
  - derived/cached data removed (derivedSymbolData, derivedTextData, authored colors, versions ...)
  - variable bindings simplified: a paint bound to a variable gets "variable": "Text/primary";
    numeric bindings become "boundVariables": {"STACK_SPACING": "Space/8", ...}
"""
import argparse, collections, json, os, re, sys

DROP = {
    "derivedSymbolData", "derivedSymbolDataLayoutVersion", "derivedTextData", "authoredColor",
    "authoredStops", "publishedVersion", "sharedSymbolVersion", "overrideKey", "pluginRelaunchData",
    "ancestorPathBeforeDeletion", "sortPosition", "interpolationColorSpace", "interpolationHueMethod",
    "toggledOffOTFeatures", "toggledOnOTFeatures", "version", "userFacingVersion",
    "sourceLibraryKey", "isSymbolPublishable", "isPublishable", "stackLayoutVersion",
    "sectionStatusInfo", "annotationCategories", "editInfo", "stopsVar",
}


def slug(s, n=60):
    return (re.sub(r"[^a-z0-9]+", "-", str(s).lower()).strip("-") or "untitled")[:n]


# defId -> component property name, filled in by main() from every componentPropDefs in the file
PROP_NAMES = {}


def resolve_overrides(root):
    """Give every instance override the id of the child it changes.

    `symbolOverrides[].guidPath` names its target by the child's `overrideKey` (library components) or `id`
    (local ones), one segment per nesting level. This adds `targetId` ("id" or "id/id/..." through nested
    instances). Overrides kept by Figma for children of *other* variants of the set (so they survive a variant
    switch) get `"otherVariant": true` instead: they don't apply to the variant shown."""
    nodes = {}
    for n, _ in walk(root):
        if n.get("id"):
            nodes[n["id"]] = n
    keymaps = {}

    def keymap(comp_id):
        if comp_id not in keymaps:
            m = {}
            comp = nodes.get(comp_id)
            for c, _ in walk(comp) if comp else []:
                if c is comp:
                    continue
                m.setdefault(c.get("id"), c)
                if c.get("overrideKey"):
                    m.setdefault(c["overrideKey"], c)
            keymaps[comp_id] = m
        return keymaps[comp_id]

    def swapped(inst, ovs, prefix_key, child):
        """Effective component of nested `child`: an override swap at this path, or an instance-swap
        property bound to it (propRefs OVERRIDDEN_SYMBOL_ID -> the instance's componentPropAssignments)."""
        for x in ovs:
            if x.get("guidPath") == prefix_key and isinstance(x.get("overriddenSymbolID"), str):
                return x["overriddenSymbolID"]
        for e in (child.get("parameterConsumptionMap") or {}).get("entries", []) or []:
            data = e.get("variableData") or {}
            if e.get("variableField") == "OVERRIDDEN_SYMBOL_ID" and data.get("dataType") == "PROP_REF":
                def_id = ((data.get("value") or {}).get("propRefValue") or {}).get("defId")
                for a in inst.get("componentPropAssignments") or []:
                    val = ((a.get("varValue") or {}).get("value") or {}).get("symbolIdValue")
                    if a.get("defID") == def_id and isinstance(val, str):
                        return val
        return child.get("componentId")

    stats = collections.Counter()
    for n in list(nodes.values()):
        if n.get("type") != "INSTANCE":
            continue
        ovs = (n.get("symbolData") or {}).get("symbolOverrides") or []
        for o in ovs:
            gp = o.get("guidPath")
            if not isinstance(gp, str):
                continue
            comp_id, ids, parts = n.get("componentId"), [], gp.split("/")
            for k, part in enumerate(parts):
                child = keymap(comp_id).get(part) if comp_id else None
                if child is None:
                    ids = None
                    break
                ids.append(child["id"])
                comp_id = swapped(n, ovs, "/".join(parts[:k + 1]), child)
            if ids:
                o["targetId"] = "/".join(ids)
                stats["resolved"] += 1
            else:
                o["otherVariant"] = True
                stats["otherVariant"] += 1
    return stats


def binding(var):
    """{"variable": name, "variableId": id} for an ALIAS binding (also inside EXPRESSION /
    FONT_STYLE wrappers, as {"expression": ..., "args": [...]}), else None."""
    if not isinstance(var, dict):
        return None
    if var.get("dataType") == "ALIAS":
        alias = (var.get("value") or {}).get("alias")
        out = {"variable": var.get("variable"), "variableId": var.get("variableId") or alias}
        return {k: v for k, v in out.items() if v}
    value = var.get("value") or {}
    if var.get("dataType") == "EXPRESSION":
        ex = value.get("expressionValue") or {}
        args = []
        for a in ex.get("expressionArguments", []):
            b = binding(a)
            args.append(b if b else (a.get("value") or {}))
        return {"expression": ex.get("expressionFunction"), "args": args}
    if var.get("dataType") == "FONT_STYLE":
        return binding((value.get("fontStyleValue") or {}).get("asFloat"))
    return None


def prune(x):
    if isinstance(x, list):
        return [prune(v) for v in x]
    if not isinstance(x, dict):
        return x
    is_node = "type" in x and "id" in x
    out = {}
    for k, v in x.items():
        if k in DROP or (k == "key" and is_node):
            continue
        if k == "colorVar":
            b = binding(v)
            if b:
                out.update(b)
            continue
        if k in ("parameterConsumptionMap", "variableConsumptionMap"):
            bound, props = {}, {}
            for e in (v or {}).get("entries", []):
                field, data = e.get("variableField"), e.get("variableData") or {}
                if not field:
                    continue
                if data.get("dataType") == "PROP_REF":
                    def_id = ((data.get("value") or {}).get("propRefValue") or {}).get("defId")
                    props[field] = PROP_NAMES.get(def_id, def_id)
                    continue
                b = binding(data)
                if b:
                    bound[field] = b
            if bound:
                out.setdefault("boundVariables", {}).update(bound)
            if props:
                out.setdefault("propRefs", {}).update(props)
            continue
        if k == "textData" and isinstance(v, dict):
            v = {kk: vv for kk, vv in v.items() if kk != "lines"}
        out[k] = prune(v)
    return out


def walk(n, path=()):
    yield n, path
    for c in n.get("children", []) or []:
        yield from walk(c, path + (n,))


def dump(obj, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(obj, f, separators=(",", ":"), ensure_ascii=False)
    return os.path.getsize(path)


# ---------------------------------------------------------------- tokens

def tokens_files(doc, out, used_only=False, library_names=None, page=None):
    library_names = library_names or {}
    cols = json.loads(json.dumps(doc.get("variables", [])))
    styles = json.loads(json.dumps(doc.get("styles", [])))
    if page is not None:
        # Count uses on this page only (the file-wide counts include drafts and old screens)
        var_uses, style_uses = collections.Counter(), collections.Counter()
        style_keys = ("fillStyle", "strokeStyle", "textStyle", "effectStyle", "gridStyle")

        def count(x):
            if isinstance(x, dict):
                if x.get("dataType") == "ALIAS" and x.get("variableId"):
                    var_uses[x["variableId"]] += 1
                for k in style_keys:
                    if isinstance(x.get(k), str):
                        style_uses[x[k]] += 1
                for v in x.values():
                    count(v)
            elif isinstance(x, list):
                for v in x:
                    count(v)
        count(page)
        for c in cols:
            for v in c["variables"]:
                v["uses"] = var_uses[v["id"]]
            c["uses"] = sum(v["uses"] for v in c["variables"])
        # styles are matched by name; when a name exists in several libraries, credit the first
        credited = set()
        for st in sorted(styles, key=lambda st: -st.get("uses", 0)):
            st["uses"] = 0 if st["name"] in credited else style_uses[st["name"]]
            credited.add(st["name"])
    # How many variables alias each variable (primitives are used through semantic tokens)
    aliased = collections.Counter()
    for c in cols:
        for v in c["variables"]:
            for val in (v.get("valuesByMode") or {}).values():
                if isinstance(val, dict) and val.get("alias"):
                    aliased[(val.get("aliasCollection"), val["alias"])] += 1
    for c in cols:
        for v in c["variables"]:
            v["aliasedBy"] = aliased[(c["name"], v["name"])]
    if used_only:
        # Keep variables used directly, plus everything they alias (transitively, same library)
        lib_of = lambda c: c.get("sourceLibraryKey", "local")
        by_name = {(lib_of(c), c["name"], v["name"]): v["id"] for c in cols for v in c["variables"]}
        var_by_id = {v["id"]: (c, v) for c in cols for v in c["variables"]}
        keep = {v["id"] for c in cols for v in c["variables"] if v.get("uses")}
        todo = list(keep)
        while todo:
            c, v = var_by_id[todo.pop()]
            for val in (v.get("valuesByMode") or {}).values():
                if isinstance(val, dict) and val.get("alias"):
                    tid = by_name.get((lib_of(c), val.get("aliasCollection"), val["alias"]))
                    if tid and tid not in keep:
                        keep.add(tid); todo.append(tid)
        cols = [dict(c, variables=[v for v in c["variables"] if v["id"] in keep]) for c in cols]
        cols = [c for c in cols if c["variables"]]
        styles = [s for s in styles if s.get("uses")]

    def label(item):
        key = item.get("sourceLibraryKey")
        if not key:
            return library_names.get("local", "local")
        short = key[:11]  # "lk-" + 8 hex chars: stable across files
        return library_names.get(short, short)

    libs = collections.OrderedDict()
    for c in cols + styles:
        c["library"] = label(c)
        libs.setdefault(c["library"], c.get("sourceLibraryKey", "local"))
    json.dump({"collections": cols, "styles": styles, "libraries": libs},
              open(f"{out}/TOKENS.json", "w"), indent=1, ensure_ascii=False)

    L = ["# Tokens", "",
         "Variables (design tokens) and shared styles, with how often the design uses each (`uses`).",
         "Bindings in node files use these names: `\"variable\": \"Text/primary\"` on paints,",
         "`\"boundVariables\": {\"STACK_SPACING\": \"Space/8\"}` for numbers. Full data: `TOKENS.json`.", ""]
    L += ["## Libraries", "",
          "Libraries are labelled by the start of their library key, which is the same in every file.", "",
          "| Library | Collections | Variable uses | Styles | Style uses |", "|---|---|---|---|---|"]
    for lib in sorted(libs, key=lambda l: -sum(c.get("uses", 0) for c in cols if c["library"] == l)):
        cs = [c for c in cols if c["library"] == lib]
        ss = [x for x in styles if x["library"] == lib]
        L.append(f"| {lib} | {', '.join(sorted({c['name'] for c in cs})) or '–'} | {sum(c.get('uses', 0) for c in cs)} "
                 f"| {len(ss)} | {sum(x.get('uses', 0) for x in ss)} |")
    L.append("")
    for c in sorted(cols, key=lambda c: -c.get("uses", 0)):
        modes = [m["name"] for m in c.get("modes", [])]
        L += [f"## {c['name']} ({c['library']}, {c.get('uses', 0)} uses)", "",
              "| Variable | Type | " + " | ".join(modes) + " | Uses | Aliased by |",
              "|---|---|" + "---|" * len(modes) + "---|---|"]
        for v in sorted(c["variables"], key=lambda v: v["name"]):
            vals = []
            for m in modes:
                val = v.get("valuesByMode", {}).get(m)
                if isinstance(val, dict) and "alias" in val:
                    val = f"→ {val['alias']}"
                elif isinstance(val, float):
                    val = f"{val:g}"
                vals.append(str(val).replace("|", "\\|"))
            L.append(f"| `{v['name']}` | {v.get('type', '')} | " + " | ".join(vals) + f" | {v.get('uses', 0)} | {v.get('aliasedBy', 0)} |")
        L.append("")
    L += ["## Styles", "", "| Style | Library | Type | Value | Uses |", "|---|---|---|---|---|"]
    for s in sorted(styles, key=lambda s: (s.get("styleType", ""), s["name"])):
        if s.get("styleType") == "TEXT":
            fn = s.get("fontName", {})
            val = f"{fn.get('family')} {fn.get('style')} {s.get('fontSize', ''):g}/{s.get('lineHeight', 'auto')}, ls {s.get('letterSpacing', '0')}" if isinstance(s.get("fontSize"), (int, float)) else ""
        elif s.get("styleType") == "FILL":
            val = ", ".join(p.get("color", p.get("type", "")) for p in s.get("fillPaints", []))
        elif s.get("styleType") == "EFFECT":
            val = "; ".join(f"{e.get('type', '')} {e.get('offset', {}).get('x', 0):g} {e.get('offset', {}).get('y', 0):g} {e.get('radius', 0):g} {e.get('spread', 0):g} {e.get('color', '')}" for e in s.get("effects", []))
        else:
            val = ""
        L.append(f"| `{s['name']}` | {s['library']} | {s.get('styleType')} | {val} | {s.get('uses', 0)} |")
    open(f"{out}/TOKENS.md", "w").write("\n".join(L) + "\n")
    return len(cols), sum(len(c["variables"]) for c in cols), len(styles)


# ---------------------------------------------------------------- design system

def build_design_system(doc, out, library_names=None):
    root = doc["document"]
    os.makedirs(out, exist_ok=True)
    tokens_files(doc, out, library_names=library_names)
    sets, comps, sections = [], [], []
    seen = collections.Counter()
    for page in (p for p in root.get("children", []) if not p.get("internalOnly")):
        for sec in page.get("children", []) or []:
            sbase = f"sections/{slug(page['name'])}__{slug(sec['name'])}"
            seen[sbase] += 1
            fn = sbase + ("" if seen[sbase] == 1 else f"-{seen[sbase]}") + ".json"
            kb = dump(prune(sec), f"{out}/{fn}") // 1024
            sections.append({"page": page["name"], "section": sec["name"], "id": sec.get("id"), "file": fn, "kb": kb})
        for n, path in walk(page):
            t = n.get("type")
            parent = path[-1] if path else None
            if t == "COMPONENT_SET" or (t == "COMPONENT" and (parent or {}).get("type") != "COMPONENT_SET"):
                sec_name = path[1]["name"] if len(path) > 1 else page["name"]
                base = f"{slug(sec_name)}__{slug(n['name'])}"
                seen[base] += 1
                fn = f"components/{base}{'' if seen[base] == 1 else '-' + str(seen[base])}.json"
                data = prune(n)
                size = dump(data, f"{out}/{fn}")
                entry = {"name": n["name"], "id": n.get("id"), "section": sec_name, "file": fn, "kb": size // 1024}
                if t == "COMPONENT_SET":
                    props = []
                    orders = {o["property"]: o["values"] for o in n.get("stateGroupPropertyValueOrders", [])}
                    for d in n.get("componentPropDefs", []):
                        dt = (d.get("varValue") or {}).get("dataType") or d.get("type")
                        kind = "VARIANT" if d["name"] in orders else {"BOOLEAN": "BOOLEAN", "STRING": "TEXT", "SYMBOL_ID": "INSTANCE_SWAP", "SLOT_CONTENT_ID": "SLOT"}.get(dt, dt)
                        props.append({"name": d["name"], "kind": kind, **({"values": orders[d["name"]]} if d["name"] in orders else {})})
                    entry.update(properties=props, variants=[c["name"] for c in n.get("children", [])],
                                 description=n.get("symbolDescription", ""))
                    if size > 150_000:
                        vdir, vs = fn[:-5], collections.Counter()
                        for v in data.get("children", []):
                            b = slug(v["name"], 120); vs[b] += 1
                            dump(v, f"{out}/{vdir}/{b}{'' if vs[b] == 1 else '-' + str(vs[b])}.json")
                        entry["variantDir"] = vdir
                    sets.append(entry)
                else:
                    entry["size"] = n.get("size"); entry["description"] = n.get("symbolDescription", "")
                    comps.append(entry)
    json.dump({"componentSets": sets, "components": comps}, open(f"{out}/components.json", "w"), indent=1, ensure_ascii=False)
    all_defs = {n.get("id"): n for n, _ in walk(root) if n.get("type") == "COMPONENT"}
    own_ids = [n.get("id") for p in root.get("children", []) if not p.get("internalOnly")
               for n, _ in walk(p) if n.get("type") == "COMPONENT"]
    ds_icons = export_icons(all_defs, own_ids, out)
    visible = {"children": [p for p in root.get("children", []) if not p.get("internalOnly")]}
    library, _ = export_used_components(
        root, visible, out, subdir="library-components", md="LIBRARY-COMPONENTS.md", only_library=True,
        title="Library components used by the Design System",
        intro="Components from other libraries that the Design System's own components use")
    json.dump(sections, open(f"{out}/index.json", "w"), indent=1, ensure_ascii=False)

    L = ["# Components", "", f"{len(sets)} component sets and {len(comps)} standalone components.", ""]
    for sec in dict.fromkeys(s["section"] for s in sets):
        L += [f"## {sec}", "", "| Component | Id | Variants | Properties | Data |", "|---|---|---|---|---|"]
        for s in (s for s in sets if s["section"] == sec):
            props = "; ".join(f"**{p['name']}**: {' / '.join(p['values'])}" if p.get("values") else f"**{p['name']}** ({p['kind']})" for p in s["properties"]) or "–"
            data = f"`{s['file']}`" + (f" (per variant: `{s['variantDir']}/`)" if s.get("variantDir") else "")
            L.append(f"| {s['name']} | `{s['id']}` | {len(s['variants'])} | {props} | {data} |")
        L.append("")
    L += ["## Standalone components", "", "| Component | Id | Section | Data |", "|---|---|---|---|"]
    for c in comps:
        L.append(f"| {c['name']} | `{c['id']}` | {c['section']} | `{c['file']}` |")
    open(f"{out}/COMPONENTS.md", "w").write("\n".join(L) + "\n")
    return {"sections": len(sections), "componentSets": len(sets), "components": len(comps),
            "libraryComponents": library, "icons": ds_icons}


# ---------------------------------------------------------------- screens

def override_texts(n):
    """Texts set on an instance through overrides (instances have no children)."""
    out = []
    for o in (n.get("symbolData") or {}).get("symbolOverrides", []) or []:
        t = ((o.get("textData") or {}).get("characters") or "").strip()
        if t:
            out.append(t)
    return out


def node_texts(n, limit=12):
    out = []
    for x, _ in walk(n):
        t = ((x.get("textData") or {}).get("characters") or "").strip() if x.get("type") == "TEXT" else ""
        out += [t] if t else []
        out += override_texts(x) if x.get("type") == "INSTANCE" else []
        if len(out) >= limit:
            break
    return [t[:40] for t in out[:limit]]


def canvas_rows(children, gap=300):
    """Order children like a reader scans the canvas: rows top to bottom, then left to right."""
    pos = lambda c: ((c.get("transform") or {}).get("y", 0), (c.get("transform") or {}).get("x", 0))
    rows, current, row_y = [], [], None
    for c in sorted(children, key=pos):
        y = pos(c)[0]
        if row_y is not None and y - row_y > gap:
            rows.append(current); current = []
        if not current:
            row_y = y
        current.append(c)
    if current:
        rows.append(current)
    return [sorted(r, key=lambda c: pos(c)[1]) for r in rows]


def is_title_card(n):
    return n.get("type") == "INSTANCE" and n.get("name", "").strip().lower() == "title"


# ---------------------------------------------------------------- icons (SVG)

def _fmt(v):
    return f"{v:.3f}".rstrip("0").rstrip(".")


def vector_path(node, use_regions=True):
    """SVG path data for a VECTOR node's vectorNetwork, in the node's own coordinates.
    Fills use the regions (closed loops); strokes use the raw segments (open lines)."""
    vd = node.get("vectorData") or {}
    net = vd.get("vectorNetwork") or {}
    verts, segs = net.get("vertices") or [], net.get("segments") or []
    norm, size = vd.get("normalizedSize") or {}, node.get("size") or {}
    sx = (size.get("x") or 0) / norm["x"] if norm.get("x") else 1.0
    sy = (size.get("y") or 0) / norm["y"] if norm.get("y") else 1.0
    pt = lambda i: (verts[i]["x"] * sx, verts[i]["y"] * sy)

    def seg(i):
        s_, e_ = segs[i]["start"], segs[i]["end"]
        return (s_["vertex"], (s_.get("dx", 0), s_.get("dy", 0)), e_["vertex"], (e_.get("dx", 0), e_.get("dy", 0)))

    def chains(indices):
        """Split segments into connected, consistently oriented runs."""
        raw = [seg(i) for i in indices]
        runs, cur, last = [], [], None
        for k, (a, ta, b, tb) in enumerate(raw):
            if cur and a == last:
                cur.append((a, ta, b, tb)); last = b
            elif cur and b == last:
                cur.append((b, tb, a, ta)); last = a
            else:
                if cur:
                    runs.append(cur)
                nxt = raw[k + 1] if k + 1 < len(raw) else None
                if nxt and b not in (nxt[0], nxt[2]) and a in (nxt[0], nxt[2]):
                    a, ta, b, tb = b, tb, a, ta
                cur, last = [(a, ta, b, tb)], b
        if cur:
            runs.append(cur)
        return runs

    def draw(run, close):
        x0, y0 = pt(run[0][0])
        d = [f"M{_fmt(x0)} {_fmt(y0)}"]
        for a, ta, b, tb in run:
            (xa, ya), (xb, yb) = pt(a), pt(b)
            if ta == (0, 0) and tb == (0, 0):
                d.append(f"L{_fmt(xb)} {_fmt(yb)}")
            else:
                d.append(f"C{_fmt(xa + ta[0] * sx)} {_fmt(ya + ta[1] * sy)} {_fmt(xb + tb[0] * sx)} "
                         f"{_fmt(yb + tb[1] * sy)} {_fmt(xb)} {_fmt(yb)}")
        if close and run[-1][2] == run[0][0]:
            d.append("Z")
        return d

    out, rules = [], set()
    regions = net.get("regions") or []
    if use_regions and regions:
        for r in regions:
            rules.add(r.get("windingRule", "NONZERO"))
            for loop in r.get("loops", []):
                for run in chains(loop.get("segments", [])):
                    out += draw(run, True)
    else:
        for run in chains(list(range(len(segs)))):
            out += draw(run, True)
    return " ".join(out), ("evenodd" if rules & {"ODD", "EVENODD"} else "nonzero")


def _paint(paints):
    for p in paints or []:
        if p.get("visible", True) and p.get("type", "SOLID") == "SOLID" and p.get("color"):
            return p
    return None


def icon_svg(component, defs):
    """(svg, kind) for a component made only of vectors (and instances of such components), or None.
    kind is "icon" (vectors only) or "graphic" (vectors on backgrounds, e.g. badges, checkboxes)."""
    colors, elements = set(), []
    flags = {"vector": False, "background": False, "image": False}

    def radius(n):
        r = n.get("cornerRadius") or n.get("rectangleTopLeftCornerRadius") or 0
        return min(r, 9999)

    def box(n, out):
        """Background / border of a frame-like node, as a rect."""
        w, h = (n.get("size") or {}).get("x", 0), (n.get("size") or {}).get("y", 0)
        if any(p.get("type") == "IMAGE" and p.get("visible", True) for p in n.get("fillPaints") or []):
            flags["image"] = True
        f, st = _paint(n.get("fillPaints")), _paint(n.get("strokePaints"))
        if not (f or st):
            return
        flags["background"] = True
        r = min(radius(n), w / 2, h / 2)
        attrs = f'width="{_fmt(w)}" height="{_fmt(h)}"' + (f' rx="{_fmt(r)}"' if r else "")
        if f:
            colors.add(f["color"]); out.append(("rect", attrs, f, None, n))
        if st:
            colors.add(st["color"]); out.append(("rectstroke", attrs, st, n.get("strokeWeight", 1), n))

    def visit(n, out, ov=None, prefix=""):
        """ov: overrides from enclosing instances, keyed by target id path relative to them."""
        ov = ov or {}
        path = (prefix + "/" if prefix else "") + str(n.get("id"))
        o = ov.get(path, {})
        if o.get("visible") is False or (n.get("visible") is False and o.get("visible") is not True):
            return True
        if "fillPaints" in o:
            n = dict(n, fillPaints=o["fillPaints"])
        if "strokePaints" in o:
            n = dict(n, strokePaints=o["strokePaints"])
        ty = n.get("type")
        t = n.get("transform") or {}
        x, y = t.get("x") or 0, t.get("y") or 0
        if ty == "VECTOR":
            flags["vector"] = True
            if any(p.get("type") == "IMAGE" for p in n.get("fillPaints") or []):
                flags["image"] = True
            f, st = _paint(n.get("fillPaints")), _paint(n.get("strokePaints"))
            items = []
            if f:
                d, rule = vector_path(n, use_regions=True)
                colors.add(f["color"]); items.append(("fill", d, f, rule, n))
            if st:
                d, _ = vector_path(n, use_regions=False)
                colors.add(st["color"]); items.append(("stroke", d, st, n.get("strokeWeight", 1), n))
            out.append(("g", f"translate({_fmt(x)} {_fmt(y)})", items))
            return True
        if ty == "INSTANCE":
            target = defs.get(n.get("componentId"))
            if not target:
                return False
            tw, th = (target.get("size") or {}).get("x") or 1, (target.get("size") or {}).get("y") or 1
            iw, ih = (n.get("size") or {}).get("x") or tw, (n.get("size") or {}).get("y") or th
            inner = []
            box(target, inner)
            # this instance's own overrides, plus the enclosing ones that reach inside it
            sub = {k[len(path) + 1:]: v for k, v in ov.items() if k.startswith(path + "/")}
            for o2 in (n.get("symbolData") or {}).get("symbolOverrides") or []:
                if o2.get("targetId"):
                    sub.setdefault(o2["targetId"], o2)
            ok = all(visit(c, inner, sub, "") for c in target.get("children", []) or [])
            scale = "" if (abs(iw - tw) < 0.01 and abs(ih - th) < 0.01) else f" scale({_fmt(iw / tw)} {_fmt(ih / th)})"
            out.append(("g", f"translate({_fmt(x)} {_fmt(y)}){scale}", inner))
            return ok
        if ty in ("FRAME", "GROUP"):
            inner = []
            box(n, inner)
            ok = all(visit(c, inner, ov, prefix) for c in n.get("children", []) or [])
            out.append(("g", f"translate({_fmt(x)} {_fmt(y)})", inner))
            return ok
        return False  # text, images, ellipses, boolean operations: not a plain icon

    box(component, elements)
    if not all(visit(c, elements) for c in component.get("children", []) or []):
        return None
    if not flags["vector"] or flags["image"]:
        return None  # plain boxes (skeletons, overlays) and image placeholders are not icons
    single = len(colors) == 1
    tokens = set()

    def render(items, ind="  "):
        lines = []
        for it in items:
            kind = it[0]
            if kind == "g":
                body = render(it[2], ind + "  ")
                if body:
                    tr = it[1].replace("translate(0 0)", "").strip()
                    lines += ([f'{ind}<g transform="{tr}">'] + body + [f"{ind}</g>"]) if tr else body
                continue
            paint = it[2]
            if paint.get("variable"):
                tokens.add(paint["variable"])
            color = "currentColor" if single else paint["color"]
            op = paint.get("opacity")
            extra = f' opacity="{_fmt(op)}"' if op not in (None, 1, 1.0) else ""
            node = it[-1]
            if kind == "fill":
                lines.append(f'{ind}<path d="{it[1]}" fill="{color}" fill-rule="{it[3]}"{extra}/>')
            elif kind == "stroke":
                cap = {"ROUND": "round", "SQUARE": "square"}.get(node.get("strokeCap"), "butt")
                join = {"ROUND": "round", "BEVEL": "bevel"}.get(node.get("strokeJoin"), "miter")
                lines.append(f'{ind}<path d="{it[1]}" fill="none" stroke="{color}" stroke-width="{_fmt(it[3])}" '
                             f'stroke-linecap="{cap}" stroke-linejoin="{join}"{extra}/>')
            elif kind == "rect":
                lines.append(f'{ind}<rect {it[1]} fill="{color}"{extra}/>')
            elif kind == "rectstroke":
                lines.append(f'{ind}<rect {it[1]} fill="none" stroke="{color}" stroke-width="{_fmt(it[3])}"{extra}/>')
        return lines

    body = render(elements)
    if not body:
        return None
    w, h = component.get("size", {}).get("x", 24), component.get("size", {}).get("y", 24)
    head = f'<svg xmlns="http://www.w3.org/2000/svg" width="{_fmt(w)}" height="{_fmt(h)}" viewBox="0 0 {_fmt(w)} {_fmt(h)}" fill="none">'
    note = (f"<!-- {component.get('name')} ({component.get('id')})"
            + (f"; color tokens: {', '.join(sorted(tokens))}" if tokens else "")
            + ("; single color, so fill uses currentColor" if single else "") + " -->")
    return "\n".join([head, note] + body + ["</svg>"]) + "\n", ("graphic" if flags["background"] else "icon")


def export_icons(defs, component_ids, out, subdir="icons"):
    """SVG for every exported component that is a vector icon (icons/) or a vector graphic on a background
    (icons/graphics/, reference renderings of badges, checkboxes, icon buttons...)."""
    entries, seen = [], collections.Counter()
    for cid in sorted(component_ids):
        node = defs.get(cid)
        if not node:
            continue
        res = icon_svg(node, defs)
        if not res:
            continue
        svg, kind = res
        folder = subdir if kind == "icon" else f"{subdir}/graphics"
        base = slug(node.get("name", cid).replace("/", "-"), 80)
        seen[(folder, base)] += 1
        n = seen[(folder, base)]
        fn = f"{folder}/{base}{'' if n == 1 else '-' + str(n)}.svg"
        os.makedirs(f"{out}/{folder}", exist_ok=True)
        open(f"{out}/{fn}", "w").write(svg)
        entries.append({"id": cid, "name": node.get("name"), "kind": kind, "file": fn})
    if entries:
        json.dump(entries, open(f"{out}/{subdir}/index.json", "w"), indent=1, ensure_ascii=False)
    return {"icons": sum(e["kind"] == "icon" for e in entries), "graphics": sum(e["kind"] == "graphic" for e in entries)}


REF_KEYS = ("componentId", "overriddenSymbolID", "symbolIdValue")


def component_refs(node):
    """Component ids a subtree points at: instances, swapped-in components (symbol overrides, instance-swap
    property values) and instance-swap property defaults."""
    refs = set()
    def scan(x):
        if isinstance(x, dict):
            for k, v in x.items():
                if k in REF_KEYS and isinstance(v, str):
                    refs.add(v)
                else:
                    scan(v)
        elif isinstance(x, list):
            for v in x:
                scan(v)
    scan(node)
    return refs


def export_used_components(root, page, out, subdir="components", md="COMPONENTS.md", only_library=False,
                           title="Components used by these screens",
                           intro="Every component an instance on this page uses"):
    """Export every component the page uses (and the components those use), from wherever it is defined:
    the page itself, another page, or the hidden internal page of library-component copies (present when
    figma2json ran with --keep-internal). Writes components/<id>.json and components/index.json."""
    defs = {}
    for n, path in walk(root):
        if n.get("type") == "COMPONENT":
            parent = path[-1] if path else {}
            internal = any(p.get("internalOnly") for p in path)
            defs[n.get("id")] = (n, parent.get("name") if parent.get("type") == "COMPONENT_SET" else None, internal)
    referenced = component_refs(page)
    used, todo = set(), list(referenced)
    while todo:
        cid = todo.pop()
        if not cid or cid in used or cid not in defs:
            continue
        used.add(cid)
        for ref in component_refs(defs[cid][0]):
            referenced.add(ref)
            todo.append(ref)
    missing = sorted(referenced - set(defs))
    entries = []
    for cid in sorted(used):
        node, set_name, internal = defs[cid]
        if only_library and not internal:
            continue
        fn = f"{subdir}/{cid.replace(':', '-')}.json"
        kb = dump(prune(node), f"{out}/{fn}") // 1024
        entries.append({"id": cid, "componentSetName": set_name, "componentName": node.get("name"),
                        "source": "library copy" if internal else "this file", "file": fn, "kb": kb})
    if entries or missing:
        os.makedirs(f"{out}/{subdir}", exist_ok=True)
        json.dump({"components": entries, "notFound": missing}, open(f"{out}/{subdir}/index.json", "w"),
                  indent=1, ensure_ascii=False)
    if entries:
        L = [f"# {title}", "",
             f"{intro}, looked up by the instance's `componentId` (or by a swapped-in component's",
             "`overriddenSymbolID` / `symbolIdValue`).",
             "\"library copy\" = Figma's local copy of a library component (exported with `--keep-internal`).", "",
             "| Component set | Variant / component | Id | Source | Data |", "|---|---|---|---|---|"]
        for e in sorted(entries, key=lambda e: ((e["componentSetName"] or e["componentName"]).lower(), e["componentName"])):
            L.append(f"| {e['componentSetName'] or '–'} | {e['componentName']} | `{e['id']}` | {e['source']} | `{e['file']}` ({e['kb']} KB) |")
        if missing:
            L += ["", f"{len(missing)} component ids not found in this export (convert with `--keep-internal`)."]
        open(f"{out}/{md}", "w").write("\n".join(L) + "\n")
    icon_count = export_icons({k: v[0] for k, v in defs.items()},
                              [e["id"] for e in entries], out, subdir=f"{subdir}-icons" if subdir != "components" else "icons")
    return {"exported": len(entries), "notFound": len(missing), "icons": icon_count}, [defs[c][0] for c in sorted(used)]


def build_screens(doc, out, page_name, library_names=None):
    root = doc["document"]
    os.makedirs(out, exist_ok=True)
    pages = {p["name"]: p for p in root.get("children", [])}
    if page_name not in pages:
        sys.exit(f"Page {page_name!r} not found. Pages: {list(pages)}")
    page = pages[page_name]

    # Collect screens per flow (Section), ordered on the canvas, with Title cards as group headings
    owner, screens, connectors, notes, sections = {}, [], [], [], {}
    for g in page.get("children", []):
        flow = g["name"] if g.get("type") == "SECTION" else "(no section)"
        if g.get("type") == "SECTION":
            # a connector may point at a whole section (= the flow) instead of a screen
            owner[g.get("id")] = (flow, g.get("id"))
            sections[g.get("id")] = flow
        children = g.get("children", []) if g.get("type") == "SECTION" else [g]
        group = ""
        for row in canvas_rows(children):
            for s in row:
                if s.get("type") == "CONNECTOR":
                    connectors.append((flow, s)); continue
                if is_title_card(s):
                    group = " — ".join(override_texts(s)[:2]) or group; continue
                if s.get("type") not in ("FRAME", "COMPONENT", "INSTANCE", "SECTION", "COMPONENT_SET"):
                    continue
                if "<->" in s.get("name", ""):
                    # designer annotation such as "button <-> Settings / Team & Roles / Invite member modal"
                    notes.append({"flow": flow, "group": group, "id": s["id"], "note": s["name"]})
                    continue
                screens.append((flow, group, s))
                for n, _ in walk(s):
                    owner[n.get("id")] = (flow, s["id"])

    # Flow arrows drawn with connectors. The arrowhead marks the destination.
    links = []
    for flow, c in connectors:
        a, b = c.get("connectorStart") or {}, c.get("connectorEnd") or {}
        sa, sb = owner.get(a.get("endpointNodeID")), owner.get(b.get("endpointNodeID"))
        if not sa or not sb:
            notes.append({"flow": flow, "group": "", "id": c.get("id"),
                          "note": f"connector between {a.get('endpointNodeID')} and {b.get('endpointNodeID')} (endpoint not on this page)"})
            continue
        if sa[1] == sb[1]:
            continue
        arrow = lambda cap: "ARROW" in str(cap or "")
        if arrow(c.get("connectorStartCap")) and not arrow(c.get("connectorEndCap")):
            src, dst, via = sb, sa, b.get("endpointNodeID")
        else:
            src, dst, via = sa, sb, a.get("endpointNodeID")
        links.append({"source": "connector", "flow": flow, "from": src[1], "fromNode": via, "to": dst[1], "toFlow": dst[0]})
    # Prototype links between screens (if any)
    for flow, group, s in screens:
        for n, _ in walk(s):
            for inter in n.get("prototypeInteractions", []) or []:
                for act in inter.get("actions", []) or []:
                    tgt = act.get("transitionNodeID")
                    if tgt in owner and owner[tgt][1] != s["id"]:
                        links.append({"source": "prototype", "flow": flow, "from": s["id"], "fromNode": n.get("id"),
                                      "event": (inter.get("event") or {}).get("interactionType", ""),
                                      "to": owner[tgt][1], "toFlow": owner[tgt][0]})

    nodes_by_id = {n.get("id"): n for n, _ in walk(root) if n.get("id")}
    index, comp_use = [], collections.defaultdict(collections.Counter)
    for flow, group, s in screens:
        # the node id in the name keeps file names unique
        fn = f"flows/{slug(flow)}/{slug(s['name'])}__{s['id'].replace(':', '-')}.json"
        kb = dump(prune(s), f"{out}/{fn}") // 1024
        for n, _ in walk(s):
            if n.get("type") == "INSTANCE":
                comp_use[flow][n.get("componentSetName") or n.get("componentName") or n.get("name")] += 1
        index.append({"flow": flow, "group": group, "id": s["id"], "name": s["name"], "type": s.get("type"),
                      "size": [round(s.get("size", {}).get("x", 0)), round(s.get("size", {}).get("y", 0))],
                      "file": fn, "kb": kb, "sampleText": node_texts(s),
                      "linksTo": sorted({l["to"] for l in links if l["from"] == s["id"]}),
                      "linkedFrom": sorted({l["from"] for l in links if l["to"] == s["id"]})})
    library, component_nodes = export_used_components(root, page, out)
    # Active navigation on each screen: instances of tab/sidebar-like components in their Active variant
    for (flow, group, s), entry in zip(screens, index):
        active = []
        for x, _ in walk(s):
            if x.get("type") != "INSTANCE":
                continue
            cname = (x.get("componentName") or "")
            if not re.search(r"(State|state)=Active", cname):
                continue
            label = next(iter(override_texts(x)), "")
            if not label:
                comp = nodes_by_id.get(x.get("componentId"))
                label = next((((c.get("textData") or {}).get("characters") or "").strip()
                              for c, _ in walk(comp) if c.get("type") == "TEXT"), "") if comp else ""
            if label:
                active.append(f"{x.get('componentSetName') or x.get('name')}: {label[:30]}")
        entry["active"] = list(dict.fromkeys(active))[:4]

    # Screens that share a name (e.g. nine "Clients"): show the texts only that version has
    def all_texts(node):
        out = set()
        for x, _ in walk(node):
            if x.get("type") == "TEXT":
                t = ((x.get("textData") or {}).get("characters") or "").strip()
                if t:
                    out.add(t[:40])
            if x.get("type") == "INSTANCE":
                out.update(t[:40] for t in override_texts(x))
        return out
    by_name = collections.defaultdict(list)
    for (flow, group, s), entry in zip(screens, index):
        by_name[(flow, s["name"])].append((s, entry))
    for (flow, name), items in by_name.items():
        if len(items) < 2:
            continue
        texts = {s["id"]: all_texts(s) for s, _ in items}
        for s, entry in items:
            others = set().union(*(texts[o["id"]] for o, _ in items if o["id"] != s["id"]))
            entry["differsBy"] = sorted(texts[s["id"]] - others, key=len)[:4]
    json.dump({"page": page_name, "screens": index, "links": links, "notes": notes},
              open(f"{out}/index.json", "w"), indent=1, ensure_ascii=False)
    # Count token use on the page and inside the components it uses (agents build both)
    tokens_files(doc, out, used_only=True, library_names=library_names,
                 page={"children": [page] + component_nodes})

    name_of = {i["id"]: i["name"] for i in index}
    name_of.update({sid: f"{flow} flow" for sid, flow in sections.items()})
    proto = any(l["source"] == "prototype" for l in links)
    L = [f"# Flows on page \"{page_name}\"", "",
         "One Design Flow = one Section on this page. Screens are in reading order on the canvas (rows top to",
         "bottom, left to right), grouped under the designer's Title cards.", "",
         "\"Comes from\" and \"Leads to\" come from the connector arrows the designer drew between screens"
         + (" and from prototype links." if proto else " (there are no prototype links between screens on this page)."),
         "An arrow that starts at a whole Section is shown as \"<Flow> flow\": that screen opens from anywhere in",
         "the flow. Screens with no arrows still need their navigation confirmed with design.", "",
         "Node data per screen is in `flows/<flow>/`; all links with node ids are in `index.json`.", ""]
    other = [p["name"] for p in root.get("children", [])
             if p["name"] != page_name and p.get("children") and not p.get("internalOnly")]
    if other:
        L += [f"Not included (other pages): {', '.join(other)}. Ask before using them.", ""]
    for flow in dict.fromkeys(i["flow"] for i in index):
        items = [i for i in index if i["flow"] == flow]
        L += [f"## {flow} ({len(items)} screens)", ""]
        group = None
        for k, i in enumerate(items, 1):
            if i["group"] != group:
                group = i["group"]
                L += ["", f"**{group}**" if group else "", "",
                      "| # | Screen | Active nav | Differs by (text only in this version) | Id | Size | Comes from | Leads to | Data |",
                      "|---|---|---|---|---|---|---|---|---|"]
            to = ", ".join(f"{name_of.get(t, t)} (`{t}`)" for t in i["linksTo"]) or "–"
            frm = ", ".join(f"{name_of.get(t, t)} (`{t}`)" for t in i["linkedFrom"]) or "–"
            diff = "; ".join(f"“{t}”" for t in i.get("differsBy", [])).replace("|", "\\|") or "–"
            act = "; ".join(i.get("active", [])).replace("|", "\\|") or "–"
            L.append(f"| {k} | {i['name']} | {act} | {diff} | `{i['id']}` | {i['size'][0]}×{i['size'][1]} | {frm} | {to} | `{i['file']}` ({i['kb']} KB) |")
        top = ", ".join(f"{c} ({n})" for c, n in comp_use[flow].most_common(12))
        L += ["", f"Components used most: {top}", ""]
        fnotes = [n for n in notes if n["flow"] == flow]
        if fnotes:
            L += ["Designer notes on the canvas (which control opens what):", ""]
            L += [f"- {n['note']} (`{n['id']}`)" for n in fnotes] + [""]
    open(f"{out}/FLOWS.md", "w").write("\n".join(L) + "\n")
    return {"flows": len(dict.fromkeys(i["flow"] for i in index)), "screens": len(index), "links": len(links),
            "notes": len(notes), "componentsExported": library,
            "connectorLinks": sum(l["source"] == "connector" for l in links)}


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("canvas"); ap.add_argument("out")
    ap.add_argument("--kind", choices=["design-system", "screens"], required=True)
    ap.add_argument("--page", default="UI")
    ap.add_argument("--library-name", action="append", default=[], metavar="KEY=NAME",
                    help='name a library, e.g. lk-1b2b26f2="ProTrade Design System" (use "local" for the file itself)')
    a = ap.parse_args()
    names = dict(x.split("=", 1) for x in a.library_name)
    if os.path.isdir(a.out) and os.listdir(a.out):
        sys.exit(f"{a.out} is not empty. Build into a new, empty folder so no stale files are left behind.")
    doc = json.load(open(a.canvas))
    override_stats = resolve_overrides(doc["document"])
    parents = {}
    for n, _ in walk(doc["document"]):
        for d in n.get("componentPropDefs", []) or []:
            if d.get("id") and d.get("name"):
                PROP_NAMES[d["id"]] = d["name"]
            elif d.get("id") and d.get("parentPropDefId"):
                parents[d["id"]] = d["parentPropDefId"]  # variant-level def -> the set's named def
    for def_id, parent in parents.items():
        seen_ids = set()
        while parent in parents and parent not in seen_ids:
            seen_ids.add(parent)
            parent = parents[parent]
        if parent in PROP_NAMES:
            PROP_NAMES[def_id] = PROP_NAMES[parent]
    if "variables" not in doc:
        sys.exit("This canvas.json has no `variables`: convert with figma2json 0.4+ (without --legacy).")
    res = (build_design_system(doc, a.out, names) if a.kind == "design-system"
           else build_screens(doc, a.out, a.page, names))
    res["overrides"] = dict(override_stats)
    print(json.dumps(res))
