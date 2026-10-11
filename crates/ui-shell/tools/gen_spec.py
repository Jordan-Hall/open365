#!/usr/bin/env python3
"""Builds crates/ui-shell/spec/{writer,sheets}.json from the oracle reference
fixtures/reference/ui/{word,excel}-ribbon.json.

Run from the repository root:  python crates/ui-shell/tools/gen_spec.py

The spec keeps what was measured and drops what is not ours:
  * structure: tabs -> groups -> blocks (columns of rows) -> controls;
  * intrinsic sizes (width, height, split-arrow width) and gaps, in "u" = physical pixels at the
    reference scale (1.25), so the layout engine can reproduce the reference at that scale exactly
    and derive other scales;
  * labels with product names replaced (Open365 shows no third-party product names).
Positions are NOT stored (except where a control does not sit on the row grid): the layout engine
computes them.  Nothing here is copied artwork; icons are chosen by us (Tabler, MIT).
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
REF = ROOT / "fixtures" / "reference" / "ui"
OUT = ROOT / "crates" / "ui-shell" / "spec"
# Optional: a file with one Tabler icon name per line (from the pinned tag) to validate the icon map.
import os
NAMES = Path(os.environ["OPEN365_TABLER_NAMES"]) if "OPEN365_TABLER_NAMES" in os.environ else None

# Labels that carry a third-party product name or trademark are renamed; widths are kept as measured.
RENAME = {
    "Office Clipboard...": "Clipboard...",
    "SmartArt...": "Diagram...",
    "WordArt": "Styled Text",
    "Editor": "Proofread",
    "Immersive Reader": "Reader",
    "Flash Fill": "Quick Fill",
    "AutoSum": "Auto Sum",
    "PivotTable": "Pivot Table",
    "Recommended PivotTables": "Suggested Pivot Tables",
    "PivotChart": "Pivot Chart",
    "Get Word Mobile App": "Get Mobile App",
    "Get Excel Mobile App": "Get Mobile App",
    "Office Scripts": "Scripts",
    "Office Scripts Gallery": "Scripts Library",
    "SharePoint": "Server",
    "Microsoft search": "Search",
    "AutoSave": "Autosave",
    "Python (Preview)": "Python",
    "Insert Python": "Insert Python",
    "Dark Mode": "Appearance",
    "Switch Modes": "Switch Theme",
    "Copilot": "Assistant",
    "Show Training": "Show Training",
}
GENERIC_FIX = [(r"\bWord\b", "document"), (r"\bExcel\b", "workbook"), (r"Microsoft ", ""), (r"Office ", "")]


STATUS = [("Page Number", "page"), ("Word Count", "words"), ("Coauthoring", "coauthor"), ("Language", "language"),
          ("Text Predictions", "predictions"), ("Accessibility", "accessibility"), ("Focus", "focus"),
          ("Read Mode", "read"), ("Print Layout", "print-layout"), ("Web Layout", "web-layout"),
          ("Zoom Out", "zoom-out"), ("Zoom In", "zoom-in"), ("Zoom 100", "zoom-percent"), ("Zoom", "zoom"),
          ("Cell Mode", "mode"), ("Normal", "normal"), ("Page Layout", "page-layout"), ("Page Break", "page-break"),
          ("Size box", "grip")]
STATUS_LABEL = {"page": "Page", "words": "Words", "language": "Language", "accessibility": "Accessibility",
                "focus": "Focus", "read": "Read Mode", "print-layout": "Print Layout", "web-layout": "Web Layout",
                "zoom-out": "Zoom Out", "zoom-in": "Zoom In", "zoom-percent": "Zoom", "zoom": "Zoom", "mode": "Ready",
                "normal": "Normal", "page-layout": "Page Layout", "page-break": "Page Break Preview", "grip": "Resize"}


def clean(label: str) -> str:
    label = label.replace("‑", "-").replace("’", "'")
    if label in RENAME:
        return RENAME[label]
    for pat, rep in GENERIC_FIX:
        label = re.sub(pat, rep, label)
    return label.strip()


def slug(s: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")
    return s or "item"


# label (cleaned, lower-case) -> Tabler outline icon name. Unmapped controls get a generated glyph.
ICONS = {
    "paste": "clipboard", "cut": "scissors", "copy": "copy", "format painter": "paint",
    "clipboard...": "clipboard-list", "grow font": "text-increase", "shrink font": "text-decrease",
    "increase font size": "text-increase", "decrease font size": "text-decrease",
    "change case": "letter-case", "clear formatting": "clear-formatting", "bold": "bold",
    "italic": "italic", "underline": "underline", "strikethrough": "strikethrough",
    "subscript": "subscript", "superscript": "superscript", "text highlight colour": "highlight",
    "font colour": "typography", "bullets": "list", "numbering": "list-numbers",
    "multi-level list": "list-tree", "decrease indent": "indent-decrease",
    "increase indent": "indent-increase", "sort...": "sort-ascending-letters",
    "show all": "pilcrow", "align left": "align-left", "centre": "align-center",
    "align right": "align-right", "justify": "align-justified",
    "line and paragraph spacing": "line-height", "shading": "bucket-droplet", "borders": "border-all",
    "text effects and typography": "typography", "editing": "pencil", "dictate": "microphone",
    "proofread": "writing", "add-ins": "apps", "cover page": "file-description",
    "blank page": "file", "page break": "page-break", "table": "table", "pictures": "photo",
    "shapes": "shape", "icons": "icons", "3d models": "cube", "diagram...": "chart-dots-3",
    "chart...": "chart-bar", "screenshot": "screenshot", "online videos...": "brand-youtube",
    "link": "link", "bookmark...": "bookmark", "cross-reference...": "arrows-cross",
    "comment": "message", "header": "layout-navbar", "footer": "layout-bottombar",
    "page number": "number", "text box": "text-resize", "quick parts": "puzzle", "styled text": "typography",
    "drop cap": "letter-a", "signature line": "signature", "date & time...": "calendar-time",
    "object...": "box", "equation": "math-function", "symbol": "omega",
    "themes": "palette", "colours": "color-swatch", "fonts": "typography", "paragraph spacing": "line-height",
    "effects": "sparkles", "set as default": "check", "watermark": "droplet", "page colour": "color-picker",
    "page borders...": "border-outer", "margins": "layout-sidebar", "orientation": "rotate-clockwise",
    "size": "dimensions", "columns": "columns", "breaks": "separator", "line numbers": "list-numbers",
    "hyphenation": "abc", "position": "layout-grid", "wrap text": "text-wrap", "bring forward": "stack-front",
    "send backward": "stack-back", "selection pane...": "list-search", "align": "align-box-left-middle",
    "group": "stack", "rotate": "rotate-2", "table of contents": "list-details", "add text": "text-plus",
    "update table...": "refresh", "insert footnote": "note", "insert endnote": "notes",
    "next footnote": "arrow-down", "show notes": "eye", "insert citation": "quote", "manage sources...": "books",
    "bibliography": "book", "change provider": "arrows-exchange", "insert caption...": "photo-edit",
    "insert table of figures...": "list", "update table": "refresh", "mark entry...": "bookmark-plus",
    "insert index...": "list-search", "update index": "refresh", "mark citation...": "bookmark-plus",
    "insert table of authorities...": "list", "envelopes...": "mail", "labels...": "tag",
    "start mail merge": "mail-forward", "select recipients": "users", "edit recipient list...": "list-details",
    "highlight merge fields": "highlight", "address block...": "address-book", "greeting line...": "message-circle",
    "insert merge field": "braces", "rules": "ruler", "match fields...": "arrows-join", "update labels": "refresh",
    "preview results": "eye", "first": "player-skip-back", "previous": "player-track-prev", "next": "player-track-next",
    "last": "player-skip-forward", "find recipient...": "search", "check for errors...": "alert-triangle",
    "finish & merge": "mail-check", "spelling and grammar": "abc", "thesaurus...": "book-2",
    "word count": "hash", "read aloud": "volume", "check accessibility": "accessible", "language": "language",
    "new comment": "message-plus", "delete": "trash", "show comments": "messages", "filter all markup": "filter",
    "show markup": "eye", "reviewing pane": "layout-sidebar-right", "change tracking options...": "settings",
    "tracking": "git-compare", "compare": "git-pull-request", "protect": "lock", "hide ink": "eraser",
    "read mode": "book-2", "print layout": "file-text", "web layout": "world", "outline": "list-tree",
    "draft": "file-pencil", "focus": "focus-2", "reader": "headphones", "appearance": "contrast-2", "switch theme": "contrast-2",
    "vertical": "arrows-vertical", "side to side": "arrows-horizontal", "ruler": "ruler-2", "gridlines": "grid-dots",
    "navigation pane": "layout-sidebar", "zoom...": "zoom-in", "100%": "zoom-scan", "one page": "file",
    "multiple pages": "files", "page width": "arrows-horizontal", "new window": "app-window",
    "arrange all": "layout-board", "split": "layout-columns", "view side by side": "columns-2",
    "synchronous scrolling": "arrows-move-vertical", "reset window position": "arrow-back-up",
    "switch windows": "switch-horizontal", "macros": "player-play", "properties": "info-circle",
    "help": "help", "feedback": "message-report", "show training": "school", "what's new": "sparkles",
    "get mobile app": "device-mobile", "esignature fields": "signature", "undo": "arrow-back-up", "redo": "arrow-forward-up",
    "save": "device-floppy", "repeat": "arrow-forward-up",
    # Sheets
    "orientation ": "rotate-clockwise", "top align": "align-box-top-center", "middle align": "align-box-center-middle",
    "bottom align": "align-box-bottom-center", "merge & centre": "arrows-join-2", "number format": "numbers",
    "accounting number format": "currency-pound", "percent style": "percentage", "comma style": "point",
    "increase decimal": "decimal", "decrease decimal": "decimal", "fill colour": "bucket-droplet",
    "conditional formatting": "table-options", "format as table": "table", "cell styles": "palette",
    "insert": "row-insert-bottom", "format": "table-options", "auto sum": "sum", "fill": "arrow-down",
    "clear": "eraser", "sort & filter": "arrows-sort", "find & select": "search", "pivot table": "table-dashed",
    "suggested pivot tables": "table-dashed", "forms": "forms", "checkbox": "checkbox",
    "recommended charts": "chart-dots", "insert column or bar chart": "chart-bar",
    "insert line or area chart": "chart-line", "insert pie or doughnut chart": "chart-pie",
    "insert hierarchy chart": "chart-treemap", "insert statistic chart": "chart-histogram",
    "insert scatter (x, y) or bubble chart": "chart-scatter", "insert combo chart": "chart-bar-popular",
    "maps": "map", "pivot chart": "chart-infographic", "line": "chart-line", "column": "chart-bar",
    "win/loss": "chart-candle", "slicer...": "adjustments", "timeline": "timeline", "text": "text-size",
    "symbols": "omega", "ink to shape": "shape", "ink replay": "player-play", "print area": "printer",
    "background...": "photo", "print titles": "list", "page set-up": "page-break", "scale": "zoom-scan",
    "view": "eye", "print": "printer", "insert function...": "math-function", "recently used": "history",
    "financial": "currency-dollar", "logical": "binary-tree", "date & time": "calendar-time",
    "lookup & reference": "search", "maths & trig": "math-symbols", "more functions": "dots",
    "defined names": "tag", "trace precedents": "arrow-bar-to-left", "trace dependents": "arrow-bar-to-right",
    "remove arrows": "arrows-cross", "show formulas": "math-function", "error checking...": "alert-circle",
    "evaluate formula": "calculator", "watch window": "eye", "calculation options": "settings",
    "calculate now": "calculator", "calculate sheet": "calculator", "get data": "database-import",
    "from text/csv": "file-type-csv", "from web": "world", "from table/range": "table", "from picture": "photo",
    "recent sources": "history", "existing connections": "plug-connected", "refresh all": "refresh",
    "queries & connections": "plug-connected", "workbook links": "link", "sort a to z": "sort-ascending-letters",
    "sort z to a": "sort-descending-letters", "filter": "filter", "reapply": "refresh", "advanced...": "adjustments",
    "text to columns...": "columns-3", "clean data": "wash", "quick fill": "bolt", "remove duplicates": "copy-off",
    "data validation...": "checklist", "consolidate...": "arrows-join", "data model": "database",
    "analyse data": "chart-dots", "what-if analysis": "flask", "forecast sheet": "chart-line", "spelling...": "abc",
    "workbook statistics": "chart-bar", "check performance": "gauge", "translate": "language", "show changes": "history",
    "delete ": "trash", "previous comment": "player-track-prev", "next comment": "player-track-next",
    "notes": "note", "protect sheet...": "lock", "protect workbook...": "lock-square", "allow edit ranges": "lock-open",
    "unshare workbook": "users-minus", "normal": "layout", "page break preview": "file-broken", "page layout": "file-text",
    "custom views...": "eye-cog", "navigation": "layout-sidebar", "formula bar": "math-function", "headings": "heading",
    "freeze panes": "layout-sidebar", "hide": "eye-off", "unhide...": "eye", "zoom to selection": "zoom-in-area",
    "new script": "script-plus", "view scripts": "script", "keep": "pin", "exit": "logout", "new": "plus", "options": "settings",
    "drawing tools": "pencil", "add": "plus", "ink to maths": "math", "ink help": "help", "can't undo": "arrow-back-up",
    "can't repeat": "arrow-forward-up", "link ": "link", "data type icons": "photo", "focus cell": "focus-2",
    "orientation": "rotate-clockwise", "illustrations": "photo", "comments": "message", "sheet options": "settings",
    "calculation": "calculator", "queries": "plug-connected", "sort": "arrows-sort",
}


# Icon of a collapsed group's button, by group label (lower case); else the first control's icon.
GROUP_ICONS = {
    "font": "letter-case", "styles": "typography", "editing": "pencil", "voice": "microphone",
    "proofing": "abc", "tracking": "git-compare", "comments": "message", "illustrations": "photo",
    "tables": "table", "links": "link", "text": "text-size", "symbols": "omega", "pages": "file",
    "media": "player-play", "protect": "lock", "language": "language", "accessibility": "accessible",
    "show": "eye", "zoom": "zoom-in", "window": "app-window", "macros": "player-play",
    "arrange": "layout-grid", "themes": "palette", "page setup": "page-break", "page set-up": "page-break",
    "paragraph": "list", "clipboard": "clipboard", "alignment": "align-left", "number": "numbers",
    "cells": "table-options", "charts": "chart-bar", "sparklines": "chart-line", "filters": "filter",
    "defined names": "tag", "formula auditing": "math-function", "function library": "math-function",
    "get & transform data": "database-import", "queries & connections": "plug-connected",
    "data tools": "adjustments", "forecast": "chart-line", "outline": "list-tree", "sort & filter": "arrows-sort",
    "data types": "database", "sheet view": "eye", "workbook views": "layout", "sheet options": "settings",
    "scale to fit": "zoom-scan", "page layout": "file-text", "protect": "lock", "changes": "history",
    "views": "layout", "immersive": "book-2", "page movement": "arrows-horizontal", "document formatting": "palette",
    "page background": "color-picker", "table of contents": "list-details", "footnotes": "note",
    "citations & bibliography": "books", "captions": "photo-edit", "index": "list-search", "start mail merge": "mail-forward",
    "write & insert fields": "braces", "preview results": "eye", "create": "mail", "drawing tools": "pencil",
    "convert": "shape", "replay": "player-play", "performance": "gauge", "notes": "note", "ink": "pencil",
    "dark mode": "contrast-2", "header & footer": "layout-navbar", "table of authorities": "list",
    "python (preview)": "brand-python", "undo": "arrow-back-up", "help": "help", "mobile": "device-mobile",
}


# Icons the chrome itself uses (window controls, chevrons, backstage navigation...).
EXTRA_ICONS = ["chevron-down", "chevron-up", "chevron-right", "minus", "square", "squares", "x", "search", "dots-vertical",
               "check", "home", "folder-open", "file-plus", "file-export", "file-info", "share", "door-exit", "settings",
               "user-circle", "arrow-left", "plus", "math-function", "printer", "device-floppy", "arrow-back-up",
               "arrow-forward-up", "file-download", "info-circle", "layout-list", "pencil", "photo", "file-text",
               "layout-grid", "table", "book-2", "world", "zoom-in", "zoom-out"]


def leaf_items(g):
    out = []

    def walk(n):
        for c in n.get("children", []):
            if c["type"] == "Group":
                walk(c)
            else:
                out.append(c)

    walk(g)
    return out


def kind_of(c):
    t, cls = c["type"], c["cls"]
    if t == "SplitButton":
        return "split"
    if t == "ComboBox":
        return "combo"
    if t == "Spinner":
        return "spinner"
    if t == "Edit":
        return "edit"
    if t == "CheckBox":
        return "checkbox"
    if t == "MenuItem":
        return "dropdown"
    return "button"


def keys_of(node):
    """Key tip sequence after the tab key: "Alt, H, F P" -> "FP" (the tab key "H" is the tab's)."""
    ak = (node or {}).get("accessKey") or ""
    parts = [p.strip() for p in ak.split(",")][1:]
    return "".join(p.replace(" ", "") for p in parts[1:]) if len(parts) > 1 else ""


def make_item(c, tab_slug, group_slug, used, idmap=None, base=None, stats=None):
    """One control. With `idmap` (reference key -> ids of the widest state) a control of a narrower
    state keeps the id, icon and key tip of its widest-state twin, so one command has one identity."""
    x, y, w, h = c["rect"]
    label = clean(c["name"]) if c["name"] else ""
    k = kind_of(c)
    if k == "dropdown" and (w >= 200 or "Gallery" in c.get("id", "")) and h >= 60 and w >= 100:
        k = "gallery"
    stem = label or c.get("id") or k
    ident = f"{tab_slug}.{group_slug}.{slug(stem)}"
    twin = None
    if idmap is not None:
        for key in (c.get("id") or None, "name:" + c["name"]):
            ids = idmap.get(key) if key else None
            if ids:
                ident = ids.pop(0)
                twin = base.get(ident)
                break
        else:
            n = used.get(ident, 0)
            used[ident] = n + 1
            ident += f"-s{n + 1}"
            if stats is not None:
                stats.append(f"{ident} ({c['name']!r})")
    else:
        n = used.get(ident, 0)
        used[ident] = n + 1
        if n:
            ident += f"-{n + 1}"
    item = {"id": ident, "label": label, "kind": k, "w": w}
    if h not in (30, 96):
        item["h"] = h
    large = h >= 80 and k != "gallery"
    item["size"] = "large" if large else ("tall" if h >= 60 else "small")
    if k in ("split", "combo"):
        ch = c.get("children", [])
        if k == "split" and len(ch) >= 2:
            if large:
                item["arrow_h"] = ch[1]["rect"][3]
            else:
                item["arrow_w"] = ch[1]["rect"][2]
        if k == "combo" and len(ch) >= 2:
            item["arrow_w"] = ch[-1]["rect"][2]
            item["value"] = clean(ch[0]["name"])
    if not c["enabled"]:
        item["enabled"] = False
    mso = c.get("id") or ""
    if mso:
        item["ref"] = mso
    item["ref_name"] = c["name"]
    key = keys_of(c)
    if key:
        item["key"] = key
    icon = ICONS.get(label.lower()) or (twin or {}).get("icon")
    if icon:
        item["icon"] = icon
    if twin and not key and twin.get("key"):
        item["key"] = twin["key"]
    return item


def build_group(g, tab_slug, used, idmap=None, base=None, stats=None, collapsible=False):
    gx, gy, gw, gh = g["rect"]
    gslug = slug(clean(g["name"]))
    launcher = False
    launcher_key = ""
    items = []
    for c in leaf_items(g):
        r = c["rect"]
        if r[2] == 0:
            continue
        if r[2] == 20 and r[3] == 20 and r[0] + 20 == gx + gw:
            launcher = True
            launcher_key = keys_of(c)
            continue
        items.append(c)
    # Spinner arrow buttons (More / Less) belong to the spinner before them.
    spinners = [c for c in items if c["type"] == "Spinner"]
    arrows = [c for c in items if c["type"] == "Button" and c["name"] in ("More", "Less")]
    for a in arrows:
        items.remove(a)
        for s in spinners:
            if s["rect"][0] <= a["rect"][0] <= s["rect"][0] + s["rect"][2] + 1 and abs(a["rect"][1] - s["rect"][1]) < 20:
                s["_arrow_right"] = max(s.get("_arrow_right", 0), a["rect"][0] + a["rect"][2])
    for s in spinners:
        if "_arrow_right" in s:
            s["rect"] = [s["rect"][0], s["rect"][1], s["_arrow_right"] - s["rect"][0], s["rect"][3]]
    # A collapsed group is one dropdown button filling the whole group chunk.
    if collapsible and is_collapsed(g):
        items = []
    gj = {"id": f"{tab_slug}.{gslug}", "label": clean(g["name"]), "ref_name": g["name"], "launcher": launcher, "w": gw}
    if launcher_key:
        gj["launcher_key"] = launcher_key
    if not items:
        gj["blocks"] = []
        return gj
    items.sort(key=lambda c: (c["rect"][0], c["rect"][1]))
    # Blocks: runs of items between x cuts where no item spans.
    blocks, cur, cur_right = [], [], None
    for c in items:
        x = c["rect"][0]
        right = x + c["rect"][2]
        if cur and x >= cur_right:
            blocks.append((cur, cur_right))
            cur, cur_right = [], None
        cur.append(c)
        cur_right = right if cur_right is None else max(cur_right, right)
    blocks.append((cur, cur_right))
    out_blocks = []
    prev_right = gx
    first = True
    for bitems, bright in blocks:
        bx = min(c["rect"][0] for c in bitems)
        rows = {}
        for c in bitems:
            r = c["rect"]
            dy = (30 - r[3]) // 2 if r[3] < 30 else 0
            rows.setdefault(r[1] - gy - dy, []).append(c)
        jrows = []
        for ry in sorted(rows):
            ritems = sorted(rows[ry], key=lambda c: c["rect"][0])
            row = {}
            slot = {2: "t0", 34: "t1", 66: "t2", 12: "c0", 52: "c1"}.get(ry)
            if slot:
                row["slot"] = slot
            else:
                row["y"] = ry
            dx = ritems[0]["rect"][0] - bx
            if dx:
                row["dx"] = dx
            prev = None
            jit = []
            for c in ritems:
                it = make_item(c, tab_slug, gslug, used, idmap, base, stats)
                if prev is not None:
                    gap = c["rect"][0] - (prev["rect"][0] + prev["rect"][2])
                    if gap != 2:
                        it["gap"] = gap
                prev = c
                jit.append(it)
            row["items"] = jit
            jrows.append(row)
        blk = {"rows": jrows}
        gap = bx - prev_right
        if first:
            gj["pad_left"] = gap
            first = False
        elif gap != 2:
            blk["gap"] = gap
        # block width = max right of its items
        prev_right = max(c["rect"][0] + c["rect"][2] for c in bitems)
        out_blocks.append(blk)
    gj["pad_right"] = gx + gw - prev_right
    gj["blocks"] = out_blocks
    return gj


# ---- responsive ribbon -------------------------------------------------------------------------
# `*-responsive.json` holds the ribbon of every tab at window widths 1900..500 px.  From it we derive,
# per group, the list of layouts it passes through as the window narrows (state 0 is the widest;
# the last is usually the collapsed group button), and per tab the ordered shrink steps.  A group
# whose label is wider than its content squeezes by a few pixels before the next step (content
# centred): those variants merge into one state with a minimum width.

def group_nodes(tab_node):
    lr = tab_node.get("lowerRibbon")
    if not lr or not lr.get("children"):
        return []
    return lr["children"][0].get("children", [])


def split_items(g):
    """Leaf controls of group `g` with the dialog launcher separated: (items, launcher?)."""
    gx, gy, gw, gh = g["rect"]
    items, launcher = [], False
    for c in leaf_items(g):
        r = c["rect"]
        if r[2] == 0:
            continue
        if r[2] == 20 and r[3] == 20 and r[0] + 20 == gx + gw:
            launcher = True
            continue
        items.append(c)
    return items, launcher


def geometry(g):
    gx, gy, gw, gh = g["rect"]
    items, launcher = split_items(g)
    rows = sorted((c["type"], c["rect"][0] - gx, c["rect"][1] - gy, c["rect"][2], c["rect"][3]) for c in items)
    return gw, tuple(rows), launcher


def is_collapsed(g):
    gx, gy, gw, gh = g["rect"]
    items, _ = split_items(g)
    return len(items) == 1 and items[0]["rect"][2] >= gw - 1 and items[0]["rect"][3] >= gh - 6


def scale_of(g, first=False):
    """large (a tall button with a label under its icon), medium (labels beside icons), small, collapsed."""
    if is_collapsed(g) and not first:
        return "collapsed"
    items, _ = split_items(g)
    if any(c["rect"][3] >= 80 for c in items):
        return "large"
    if any(c["type"] in ("Button", "SplitButton", "MenuItem") and c["rect"][2] > 60 for c in items):
        return "medium"
    return "small"


def same_family(a, b):
    """Same layout up to a uniform sideways shift and a different group width (an elastic squeeze)."""
    (gwa, ia, la), (gwb, ib, lb) = a, b
    if la != lb or len(ia) != len(ib):
        return False
    if ia == ib and gwa == gwb:
        return True
    if not ia:
        return False
    for (ta, xa, ya, wa, ha), (tb, xb, yb, wb, hb) in zip(ia, ib):
        if (ta, ya, wa, ha) != (tb, yb, wb, hb):
            return False
    return len({xa - xb for (_, xa, *_), (_, xb, *_) in zip(ia, ib)}) == 1


def responsive_groups(series, tslug, used, report):
    """series: [(width, tab node)] widest first.  Returns (group dicts, steps)."""
    import copy
    names = [g["name"] for g in group_nodes(series[0][1])]
    cols = {gi: [] for gi in range(len(names))}  # gi -> [(width, node)]
    kept = []
    for width, tab in series:
        gs = group_nodes(tab)
        if [g["name"] for g in gs] != names:
            report.append(f"  width {width}: groups {[g['name'] for g in gs]} differ from {names}; skipped")
            continue
        kept.append(width)
        for gi, g in enumerate(gs):
            cols[gi].append((width, g))
    groups, seqs = [], []
    for gi, name in enumerate(names):
        fams = []  # [representative geometry, [(width, node)]]
        seq = []
        for width, node in cols[gi]:
            geo = geometry(node)
            for fi, (rep, members) in enumerate(fams):
                if same_family(rep, geo):
                    members.append((width, node))
                    seq.append(fi)
                    break
            else:
                fams.append((geo, [(width, node)]))
                seq.append(len(fams) - 1)
        if seq != sorted(seq):
            report.append(f"  group {name}: states are not monotone in width: {seq}")
        seqs.append(seq)
        state0 = None
        states = []
        base_items, idmap = {}, {}
        for fi, (rep, members) in enumerate(fams):
            natural = max(members, key=lambda m: m[1]["rect"][2])[1]
            gws = [m[1]["rect"][2] for m in members]
            if fi == 0:
                st = state0 = build_group(natural, tslug, used)
                for b in st["blocks"]:
                    for r in b["rows"]:
                        for it in r["items"]:
                            base_items[it["id"]] = it
                            idmap.setdefault(it.get("ref") or None, []).append(it["id"])
                            idmap.setdefault("name:" + it["ref_name"], []).append(it["id"])
            else:
                unmatched = []
                st = build_group(natural, tslug, used, copy.deepcopy(idmap), base_items, unmatched, True)
                if unmatched:
                    report.append(f"  group {name} state {fi}: new ids {unmatched}")
            st["scale"] = scale_of(natural, fi == 0)
            if min(gws) != max(gws):
                st["min_w"] = min(gws)
                ok_center = True
                for _, node in members:
                    items, _l = split_items(node)
                    if not items:
                        continue
                    x0 = min(c["rect"][0] for c in items)
                    cw = max(c["rect"][0] + c["rect"][2] for c in items) - x0
                    if x0 - node["rect"][0] != (node["rect"][2] - cw) // 2:
                        ok_center = False
                if ok_center:
                    st["center"] = True
                else:
                    report.append(f"  group {name} state {fi}: squeezed {min(gws)}..{max(gws)} but not centred")
            if fi > 0:
                for k in ("id", "label", "ref_name"):
                    st.pop(k, None)
                states.append(st)
        state0["states"] = states
        gk = [p.strip() for p in (group_nodes(series[0][1])[gi].get("accessKey") or "").split(",")][1:]
        pk = "".join(p.replace(" ", "") for p in gk[1:])
        if pk:
            state0["popup_key"] = pk
        icon = GROUP_ICONS.get(clean(name).lower()) or next(
            (it["icon"] for b in state0["blocks"] for r in b["rows"] for it in r["items"] if it.get("icon")), None)
        if icon:
            state0["icon"] = icon
        groups.append(state0)
    # Shrink steps: the groups whose state changed between two recorded widths, in width order.
    steps = []
    prev = [0] * len(names)
    for wi in range(len(kept)):
        cur = [seq[wi] for seq in seqs]
        chg = [[gi, cur[gi]] for gi in range(len(names)) if cur[gi] != prev[gi]]
        if chg:
            steps.append(chg)
        prev = cur
    return groups, steps


def chrome_item(c, pane, kind=None):
    x, y, w, h = c["rect"]
    px, py = pane[0], pane[1]
    return {
        "id": slug(c.get("id") or c["name"]),
        "label": clean(c["name"]),
        "kind": kind or kind_of(c),
        "x": x - px, "y": y - py, "w": w, "h": h,
        "right": px + pane[2] - (x + w),
        "ref_name": c["name"],
        **({"enabled": False} if not c["enabled"] else {}),
    }


def flatten(n):
    yield n
    for c in n.get("children", []):
        yield from flatten(c)


def build(app_key, path, ref_app, resp_path=None):
    d = json.loads(path.read_text(encoding="utf-8"))
    resp = sorted(json.loads(resp_path.read_text(encoding="utf-8"))["widths"], key=lambda w: -w["width"]) if resp_path else None
    pane = d["chrome"]["ribbonPane"]
    spec = {
        "app": app_key,
        "ref_scale": d["measuredScale"],
        "ref_window": d["window"][2:],
        "pane": pane,
        "tabs": [],
    }
    chrome = d["chrome"]["elements"]
    title, tabrow = [], []
    file_tab = None
    for e in chrome:
        if e["id"] == "FileTabButton":
            file_tab = chrome_item(e, pane, "file")
        elif e["type"] == "Tab":
            spec["tab_strip"] = {"x": e["rect"][0] - pane[0], "y": e["rect"][1] - pane[1], "w": e["rect"][2], "h": e["rect"][3]}
            for c in e.get("children", []):
                if c["type"] != "TabItem":
                    tabrow.append(chrome_item(c, pane))
        elif e["rect"][1] - pane[1] >= 55 and e["type"] != "Tab":
            tabrow.append(chrome_item(e, pane))
        elif e["id"] == "MeControlWidget":
            title.append(chrome_item(e, pane, "avatar"))
        elif e["type"] == "ToolBar":
            qat = {"x": e["rect"][0] - pane[0], "y": e["rect"][1] - pane[1], "w": e["rect"][2], "h": e["rect"][3], "items": []}
            for c in e["children"]:
                it = chrome_item(c, pane)
                it["id"] = "qat." + slug(c.get("id") or c["name"])
                if c.get("children") and c["type"] == "SplitButton":
                    it["arrow_w"] = c["children"][-1]["rect"][2]
                    it["kind"] = "split"
                it["label"] = clean(c["name"]).replace("Can't Undo", "Undo").replace("Repeat New...", "Repeat").replace("Can't Repeat", "Repeat")
                qat["items"].append(it)
            spec["qat"] = qat
        elif e["type"] == "TitleBar":
            spec["title_text"] = chrome_item(e, pane, "title")
        elif e["id"] == "TellMeControlAnchor":
            it = chrome_item(e, pane, "search")
            it["label"] = "Search"
            spec["search"] = it
        elif e["type"] == "Button" and e["cls"] == "NetUIAppFrameHelper":
            it = chrome_item(e, pane, "caption")
            it["id"] = "caption." + slug(e["name"])
            title.append(it)
    spec["file_tab"] = file_tab
    spec["title"] = title
    spec["tab_row_right"] = tabrow
    lower = d["tabs"][0]["lowerRibbon"]
    spec["lower"] = {"x": lower["rect"][0] - pane[0], "y": lower["rect"][1] - pane[1], "h": lower["rect"][3], "w": lower["rect"][2]}
    first_group = lower["children"][0]["children"][0]
    spec["groups_x"] = first_group["rect"][0] - pane[0]
    spec["group_y"] = first_group["rect"][1] - pane[1]
    spec["group_h"] = first_group["rect"][3]
    used = {}
    for ti, t in enumerate(d["tabs"]):
        tslug = slug(t["name"])
        tab = {"id": tslug, "label": clean(t["name"]), "w": t["rect"][2], "ref": t["id"], "groups": []}
        tk = [p.strip() for p in (t.get("accessKey") or "").split(",")][1:]
        if tk:
            tab["key"] = "".join(p.replace(" ", "") for p in tk)
        series = [(w["width"], w["tabs"][ti]) for w in resp] if resp else None
        if series and any(group_nodes(n) for _, n in series):
            report = []
            tab["groups"], tab["steps"] = responsive_groups([x for x in series if group_nodes(x[1])], tslug, used, report)
            if report:
                print(app_key, t["name"], file=sys.stderr)
                print("\n".join(report), file=sys.stderr)
        elif t["lowerRibbon"] and t["lowerRibbon"].get("children"):
            for g in t["lowerRibbon"]["children"][0]["children"]:
                tab["groups"].append(build_group(g, tslug, used))
        spec["tabs"].append(tab)
    # Status bar.
    status = None
    for p in d["panes"]:
        for n in flatten(p):
            if n["type"] == "StatusBar":
                status = n
    sr = status["rect"]
    spec["status"] = {"y_from_bottom": d["window"][3] - sr[1], "h": sr[3], "x": sr[0], "w": sr[2], "items": []}
    for c in status["children"]:
        sid = next((i for p, i in STATUS if c["name"].startswith(p)), None)
        if sid is None or sid in ("coauthor", "predictions"):
            continue  # product-specific indicators are not part of Open365
        it = chrome_item(c, [sr[0], sr[1], sr[2], sr[3]])
        it["id"] = "status." + sid
        it["label"] = STATUS_LABEL.get(sid, "")
        if c["type"] == "Slider":
            it["kind"] = "slider"
            for k in c.get("children", []):
                if k["type"] == "Thumb":
                    it["thumb"] = [k["rect"][0] - c["rect"][0], k["rect"][1] - sr[1], k["rect"][2], k["rect"][3]]
        spec["status"]["items"].append(it)
    # Excel only: name box, formula bar, sheet tabs.
    if ref_app == "Excel":
        for p in d["panes"]:
            if p["cls"] == "EXCEL;":
                spec["formula_left"] = {"x": p["rect"][0] - pane[0], "y": p["rect"][1] - pane[1], "w": p["rect"][2], "h": p["rect"][3]}
                nb = p["children"][0]
                spec["name_box"] = {"x": nb["rect"][0] - pane[0], "y": nb["rect"][1] - pane[1], "w": nb["rect"][2], "h": nb["rect"][3]}
            if p["id"] == "FormulaBar":
                spec["formula_bar"] = {"x": p["rect"][0] - pane[0], "y": p["rect"][1] - pane[1], "w": p["rect"][2], "h": p["rect"][3], "right": pane[0] + pane[2] - (p["rect"][0] + p["rect"][2])}
            if p["cls"] == "XLDESK":
                for n in flatten(p):
                    if n["cls"] == "ExcelBookTabControl":
                        r = n["rect"]
                        spec["sheet_tabs"] = {"y": r[1] - pane[1], "h": r[3], "x": r[0] - pane[0],
                                              "items": [dict(chrome_item(c, [r[0], r[1], r[2], r[3]]), id="sheettab." + slug(c["name"])) for c in n["children"]]}
                    if n["type"] == "DataItem" and n["name"] == "A":
                        spec["col_header_h"] = n["rect"][3]
                        spec["col_header_w"] = n["rect"][2]
                    if n["type"] == "HeaderItem" and n["name"] == "Select All":
                        spec["row_header_w"] = n["rect"][2]
                        spec["col_header_h"] = n["rect"][3]
        spec["name_box"]["label"] = "Name Box"
    # Backstage navigation (labels only: the content pane is not recorded).
    nav = [n for n in d["backstage"] if n["type"] == "ListItem"]
    spec["backstage"] = {
        "nav": [{"label": clean(n["name"]), "id": slug(n["name"]), "y": n["rect"][1], "h": n["rect"][3], "w": n["rect"][2]} for n in nav],
        "nav_x": nav[0]["rect"][0], "nav_w": nav[0]["rect"][2],
        "back": next(({"y": n["rect"][1], "h": n["rect"][3]} for n in nav if n["name"] == "Back"), None),
    }
    return spec


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    allowed = set(NAMES.read_text().split()) if NAMES and NAMES.exists() else None
    used_icons = set()
    for key, f, app in (("writer", "word-ribbon.json", "Word"), ("sheets", "excel-ribbon.json", "Excel")):
        spec = build(key, REF / f, app, REF / f.replace("-ribbon", "-responsive"))

        def walk_items(spec):
            for t in spec["tabs"]:
                for g in t["groups"]:
                    for b in g["blocks"]:
                        for r in b["rows"]:
                            yield from r["items"]

        for it in walk_items(spec):
            ic = it.get("icon")
            if ic and allowed is not None and ic not in allowed:
                print("missing tabler icon:", ic, "for", it["label"], file=sys.stderr)
                del it["icon"]
            elif ic:
                used_icons.add(ic)
        for t in spec["tabs"]:
            for g in t["groups"]:
                ic = g.get("icon")
                if ic and allowed is not None and ic not in allowed:
                    del g["icon"]
                elif ic:
                    used_icons.add(ic)
        for sec in ("qat",):
            for it in spec[sec]["items"]:
                ic = ICONS.get(it["label"].lower())
                if ic:
                    it["icon"] = ic
                    used_icons.add(ic)
        (OUT / f"{key}.json").write_text(json.dumps(spec, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
        n = sum(1 for _ in walk_items(spec))
        print(key, "tabs", len(spec["tabs"]), "items", n)
    used_icons |= set(EXTRA_ICONS)
    (OUT / "icons.txt").write_text("\n".join(sorted(used_icons)) + "\n", encoding="utf-8", newline="\n")
    # Embedded table: every icon is a vendored Tabler SVG (assets/icons/tabler, MIT).
    icon_dir = ROOT / "assets" / "icons" / "tabler"
    lines = ["//! Generated by tools/gen_spec.py. Do not edit.", "", "/// Tabler outline icons (MIT), by name.",
             "pub(crate) const ICONS: &[(&str, &str)] = &["]
    for n in sorted(used_icons):
        if (icon_dir / f"{n}.svg").exists():
            lines.append(f'    ("{n}", include_str!("../../../assets/icons/tabler/{n}.svg")),')
        else:
            print("icon file not vendored:", n, file=sys.stderr)
    lines.append("];")
    (ROOT / "crates" / "ui-shell" / "src" / "icon_table.rs").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print("icons", len(used_icons))


if __name__ == "__main__":
    main()
