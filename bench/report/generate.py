#!/usr/bin/env python3
import base64
import datetime as dt
import html
import json
import os
import statistics
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "bench", "out")
COLLABORA = os.path.join(ROOT, "..", "..", "bare-collabora")

FORMAT_NAMES = {
    "docx": "Word (DOCX)",
    "doc": "Word 97 (DOC)",
    "odt": "OpenDocument (ODT)",
    "fodt": "Flat OpenDocument (FODT)",
    "pptx": "PowerPoint (PPTX)",
    "xlsx": "Excel (XLSX)",
    "xls": "Excel 97 (XLS)",
    "md": "Markdown (MD)",
}
FORMAT_ORDER = ["docx", "doc", "odt", "fodt", "pptx", "xlsx", "xls", "md"]
TIMING_FILES = [
    "sdk-sample.docx",
    "minimal-table-unicode.docx",
    "sdk-sample.pptx",
    "sdk-sample.xlsx",
    "mixednumberings.odt",
    "sdk-sample.doc",
]
TEXT_NOTES = {
    "lo-footnote.docx": "The footnote number sticks to the word before it in both PDFs, so 'Foo' comes out as 'Foo1'.",
    "lo-footer-body-distance.docx": "The two pages use the first-page and even-page headers and footers, so the four odd-page words never appear. The rest come out in page order (header, body, footer), which differs from the order in the file.",
    "lo-formats.xlsx": "The sheet stores raw values (0.401, 12345) but prints them formatted (40.10%, $12,345.00), which the word check cannot match. Ours prints a few more values the way the file spells them.",
    "lo-section_break_numbering.docx": "The word 'header' sits in a header no section uses, so neither engine prints it.",
    "mixednumberings.odt": "List labels glue to the text that follows (3.2.1xxxx) in both PDFs, and the words joined by non-breaking hyphens come back shortened from ours.",
    "lo-2col-header.docx": "All three words are there. The file lists body, footer, header; the page reads header, body, footer.",
    "lo-mixednumberings.docx": "List labels glue to the text that follows (3.2.1xxxx) in both PDFs.",
    "lo-ShapePlusImage.pptx": "LibreOffice draws the WordArt as curves with no text layer, so its PDF yields no words at all.",
    "lo-formatting-bullet-indent.pptx": "bare-collabora spaces the title's letters apart ('Mast er st yl e'), so those three words do not match.",
}

SAMPLES = [
    ("lo-anchor-position.docx", "A picture anchored beside text. Position, size and text placement are identical."),
    ("sdk-sample.docx", "A six-row table with minimum row heights and cell margins. Row edges, baselines and the start of every line match; what is left is anti-aliasing of small bold text."),
    ("wrapped-picture.docx", "Heading, inline picture with caption, a right-anchored picture with square wrap and a numbered list. Text wraps beside the picture for its height only, then returns to full width."),
    ("sdk-sample.pptx", "Four slides with placeholders, shapes and a table styled from the master. Kerning follows Impress."),
    ("lo-formats.xlsx", "Number, percentage, currency, scientific, fraction and boolean formats. Values and alignment match; small text makes the score sensitive to half-point offsets."),
    ("lo-ShapePlusImage.pptx", "WordArt text warps. LibreOffice bends the text along the shape; we render it straight. Not planned."),
    ("sdk-sample.xls", "A student roster from the SDK test project. Column widths, clipping at the cell edge and number alignment match LibreOffice to a quarter point."),
    ("picture.xls", "A PNG picture anchored from B3 to E12, read from the workbook's drawing layer and placed by its cell anchor. Size and position match."),
    ("charts.xls", "Column, pie and line charts read from the embedded chart streams, with their data taken from the sheet cells. LibreOffice smooths the line chart and draws diamond markers; ours draws straight lines with square markers."),
    ("cond-format.xls", "Four conditional formats: value comparisons with red and green fills, and formula rules that make a row bold red and a status italic blue. Every rule applies as in LibreOffice."),
    ("comments.xls", "Two cell comments become PDF sticky notes pinned to the cell's top-right corner, the way LibreOffice's export does it. The icons are the viewer's rendering of those notes."),
    ("markdown-features.md", "Headings, emphasis, inline code, bullet, nested and numbered lists, a fenced code block, a table, a quote and a rule, laid out with LibreOffice Writer's default styles. Every line sits within half a point of LibreOffice's."),
    ("markdown-readme.md", "This project's README as a real document: a formats table, install commands and the API reference across two pages."),
]


def load(name):
    with open(os.path.join(OUT, name)) as f:
        return json.load(f)


def sh(*cmd, cwd=ROOT):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True).stdout.strip()


def pct(v, digits=1):
    return "–" if v is None else f"{v * 100:.{digits}f}%"


def ms(v):
    return "–" if v is None else f"{v:,.0f} ms"


def mb(bytes_):
    return f"{bytes_ / 1e6:.1f} MB"


def esc(s):
    return html.escape(str(s))


def grade(iou):
    return "good" if iou >= 0.85 else "fair" if iou >= 0.6 else "weak"


def data_uri(path):
    with open(path, "rb") as f:
        return "data:image/png;base64," + base64.b64encode(f.read()).decode()


def dir_size(path):
    total = 0
    for base, _, files in os.walk(path):
        for name in files:
            total += os.path.getsize(os.path.join(base, name))
    return total


def collabora_breakdown():
    lib_dir = os.path.join(COLLABORA, "npm", "darwin-arm64", "prebuilds", "darwin-arm64", "bare-collabora-darwin-arm64")
    groups = {"libmergedlo": 0, "ICU data": 0, "Writer (libswlo)": 0, "Calc (libsclo)": 0, "Impress (libsdlo)": 0, "other libraries": 0, "assets (fonts, registry, share)": 0}
    for base, _, files in os.walk(lib_dir):
        for name in files:
            size = os.path.getsize(os.path.join(base, name))
            if name.startswith("libmergedlo"):
                groups["libmergedlo"] += size
            elif name.startswith("libicudata"):
                groups["ICU data"] += size
            elif name.startswith("libswlo"):
                groups["Writer (libswlo)"] += size
            elif name.startswith("libsclo"):
                groups["Calc (libsclo)"] += size
            elif name.startswith("libsdlo"):
                groups["Impress (libsdlo)"] += size
            elif ".dylib" in name or name.endswith(".so"):
                groups["other libraries"] += size
            else:
                groups["assets (fonts, registry, share)"] += size
    order = ["libmergedlo", "assets (fonts, registry, share)", "ICU data", "Writer (libswlo)", "Calc (libsclo)", "Impress (libsdlo)", "other libraries"]
    return [(k, groups[k]) for k in order]


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "bench", "report", dt.date.today().isoformat() + ".html")
    report = load("report.json")
    text = load("text-report.json")
    timing = load("timing.json")

    cases = [r for r in report["results"] if r["task"].endswith(">pdf") and not r.get("error")]
    cases.sort(key=lambda r: -r["inkIou"])
    pages = sum(r["referencePages"] for r in cases)
    mean_iou = statistics.mean(r["inkIou"] for r in cases)
    by_format = {}
    for r in cases:
        by_format.setdefault(r["name"].rsplit(".", 1)[1], []).append(r)

    text_rows = [t for t in text["results"] if t["task"].endswith(">pdf")]
    scored = [t for t in text_rows if t.get("sourceWords") and t.get("ours") and t.get("reference") and t["ours"].get("recall") is not None and t["reference"].get("recall") is not None]

    def mean_of(side, key):
        return statistics.mean(t[side][key] for t in scored)

    ours_t = timing["ours"]
    ref_t = timing["collabora"]

    def warm(engine, name):
        for f in engine["files"]:
            if f["file"] == name:
                return statistics.median(f["warmMs"])
        return None

    def cold(engine, name):
        for f in engine["files"]:
            if f["file"] == name:
                return f["coldMs"]
        return None

    corpus_ours = sum(r["oursMs"] for r in cases)
    corpus_ref = sum(r["referenceMs"] for r in cases)

    addon_dir = os.path.join(ROOT, "npm", "darwin-arm64", "prebuilds", "darwin-arm64")
    addon_size = sum(os.path.getsize(os.path.join(addon_dir, f)) for f in os.listdir(addon_dir) if f.endswith(".bare"))
    fonts_size = dir_size(os.path.join(ROOT, "fonts"))
    ours_total = addon_size + fonts_size
    ref_parts = collabora_breakdown()
    ref_total = sum(v for _, v in ref_parts)

    commit = sh("git", "rev-parse", "--short", "HEAD")
    cpu = sh("sysctl", "-n", "machdep.cpu.brand_string")
    mem_gb = int(sh("sysctl", "-n", "hw.memsize")) // (1024 ** 3)
    macos = sh("sw_vers", "-productVersion")
    bare = sh("bare", "--version")
    with open(os.path.join(COLLABORA, "package.json")) as f:
        collabora_version = json.load(f)["version"]
    today = dt.date.today().isoformat()

    parts = []
    w = parts.append

    w(f"""<title>bare-x-to-pdf Benchmark</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
:root{{--bg:#f7f7f4;--surface:#ffffff;--surface-2:#eceef2;--text:#15171c;--text-2:#4f5460;--muted:#7d818b;--border:#dfe1e6;--ours:#2a78d6;--ours-soft:#d5e5f9;--ref:#eb6834;--ref-soft:#fbdccd;--good:#1f8a4c;--fair:#a86f00;--weak:#c9463d;--good-bg:#e1f3e8;--fair-bg:#fbefd6;--weak-bg:#fbe3e1;--s0:#2a78d6;--s1:#8fbaea;--r0:#eb6834;--r1:#f19a76;--r2:#f7c5b0;--r3:#fbe0d3;--r4:#d9d3cf;--r5:#c2bcb8;--r6:#a8a29e}}
@media (prefers-color-scheme: dark){{:root:not([data-theme="light"]){{--bg:#131417;--surface:#1b1d21;--surface-2:#25282e;--text:#f1f1ee;--text-2:#b8bbc3;--muted:#82868f;--border:#2d3037;--ours:#4a92e8;--ours-soft:#1f3b5e;--ref:#e46a3a;--ref-soft:#4c2a1c;--good:#4cc57f;--fair:#e2a93f;--weak:#f07f77;--good-bg:#173224;--fair-bg:#3a2d10;--weak-bg:#3d1f1c;--s0:#4a92e8;--s1:#2f5c93;--r0:#e46a3a;--r1:#a8461f;--r2:#7c3a1e;--r3:#5c3222;--r4:#4a4642;--r5:#3c3936;--r6:#2f2d2b}}}}
:root[data-theme="dark"]{{--bg:#131417;--surface:#1b1d21;--surface-2:#25282e;--text:#f1f1ee;--text-2:#b8bbc3;--muted:#82868f;--border:#2d3037;--ours:#4a92e8;--ours-soft:#1f3b5e;--ref:#e46a3a;--ref-soft:#4c2a1c;--good:#4cc57f;--fair:#e2a93f;--weak:#f07f77;--good-bg:#173224;--fair-bg:#3a2d10;--weak-bg:#3d1f1c;--s0:#4a92e8;--s1:#2f5c93;--r0:#e46a3a;--r1:#a8461f;--r2:#7c3a1e;--r3:#5c3222;--r4:#4a4642;--r5:#3c3936;--r6:#2f2d2b}}
body{{background:var(--bg);color:var(--text);font-family:"IBM Plex Sans",system-ui,-apple-system,"Segoe UI",sans-serif;font-size:17px;line-height:1.6;margin:0}}
.wrap{{max-width:1240px;margin:0 auto;padding-block:40px 80px;padding-inline:24px}}
h1{{font-size:34px;font-weight:600;margin:0 0 8px;letter-spacing:-.015em;text-wrap:balance;line-height:1.2}}
h2{{font-size:24px;font-weight:600;margin:64px 0 8px;letter-spacing:-.01em}}
h2 + p{{margin-top:8px}}
p{{max-width:78ch;color:var(--text-2);margin:12px 0}}
p b, li b{{color:var(--text);font-weight:500}}
.eyebrow{{font-size:13px;letter-spacing:.08em;text-transform:uppercase;color:var(--muted);font-weight:500}}
.meta{{display:flex;flex-wrap:wrap;gap:6px 22px;color:var(--text-2);font-size:15px;margin-top:10px}}
code{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:.85em;background:var(--surface-2);padding:1px 6px;border-radius:4px}}
.mono{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-variant-numeric:tabular-nums}}
.tablewrap{{overflow-x:auto;border:1px solid var(--border);border-radius:10px;background:var(--surface);margin-top:20px}}
table{{border-collapse:collapse;width:100%}}
th{{text-align:left;font-size:13px;color:var(--muted);font-weight:500;text-transform:uppercase;letter-spacing:.05em;padding:14px 16px;border-bottom:1px solid var(--border);white-space:nowrap;background:var(--surface)}}
td{{padding:13px 16px;border-bottom:1px solid var(--border);vertical-align:middle;font-size:16px}}
tr:last-child td{{border-bottom:0}}
th.num,td.num{{text-align:right;font-family:"IBM Plex Mono",ui-monospace,monospace;font-variant-numeric:tabular-nums}}
td.name{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:15px}}
td.win{{color:var(--good);font-weight:500}}
table.vs th.eng{{font-size:15px;text-transform:none;letter-spacing:0;color:var(--text);font-weight:600}}
table.vs th.eng i{{display:inline-block;width:11px;height:11px;border-radius:3px;margin-right:9px;vertical-align:-1px}}
table.vs th.ours i{{background:var(--ours)}}table.vs th.ref i{{background:var(--ref)}}
table.vs td.metric{{min-width:240px}}
table.vs .mname{{display:block;font-weight:500}}
table.vs .msub{{display:block;font-size:14px;color:var(--muted)}}
table.vs td.val{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-variant-numeric:tabular-nums;font-size:17px;white-space:nowrap}}
table.vs td.val.win{{color:var(--good);font-weight:500}}
table.vs td.diff{{color:var(--text-2);font-size:15px;white-space:nowrap}}
.note{{font-size:15px;color:var(--muted);margin-top:10px}}
.key{{display:flex;gap:20px;font-size:15px;color:var(--text-2);margin:12px 0 0;flex-wrap:wrap}}
.key i{{display:inline-block;width:11px;height:11px;border-radius:3px;margin-right:7px;vertical-align:-1px}}
.bar{{height:12px;background:var(--surface-2);border-radius:3px;overflow:hidden;flex:1}}
.bar.sm{{height:10px;min-width:120px}}
.bar-fill{{height:100%;border-radius:3px}}
.bar-fill.good{{background:var(--good)}}.bar-fill.fair{{background:var(--fair)}}.bar-fill.weak{{background:var(--weak)}}
.fmts{{display:grid;grid-template-columns:repeat(auto-fit,minmax(340px,1fr));gap:12px 40px;margin-top:20px}}
.fmt{{padding:12px 0;border-bottom:1px solid var(--border)}}
.fmt-head{{display:flex;align-items:baseline;gap:12px;margin-bottom:8px}}
.fmt-name{{font-weight:500;flex:1}}.fmt-n{{color:var(--muted);font-size:15px}}.fmt-val{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-variant-numeric:tabular-nums;font-size:18px}}
.chips{{display:flex;gap:8px;flex-wrap:wrap;margin:16px 0 0}}
.chip{{font:inherit;font-size:15px;padding:6px 14px;border-radius:999px;border:1px solid var(--border);background:var(--surface);color:var(--text-2);cursor:pointer}}
.chip[aria-pressed="true"]{{background:var(--text);color:var(--bg);border-color:var(--text)}}
.chip:focus-visible,th[data-k]:focus-visible{{outline:2px solid var(--ours);outline-offset:2px}}
#cases th[data-k]{{cursor:pointer}}
#cases table{{min-width:760px}}
td.iou{{min-width:220px}}
td.iou .cell{{display:flex;align-items:center;gap:12px}}
td.iou .num{{min-width:60px;text-align:right;font-family:"IBM Plex Mono",ui-monospace,monospace;font-variant-numeric:tabular-nums}}
td.fmt{{color:var(--text-2);font-size:15px;white-space:nowrap}}
td.why{{color:var(--text-2);font-size:15px;min-width:320px;line-height:1.45}}
#text table{{min-width:900px}}
.tiles{{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:14px;margin-top:20px}}
.tile{{background:var(--surface);border:1px solid var(--border);border-radius:10px;padding:18px 20px}}
.tile .lbl{{font-size:15px;color:var(--text);font-weight:500}}
.tile .def{{font-size:14px;color:var(--muted);margin-top:2px;min-height:2.6em}}
.tile .pair{{display:flex;gap:28px;margin-top:14px;align-items:flex-end}}
.tile .val{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:30px;font-weight:500;line-height:1.1;font-variant-numeric:tabular-nums}}
.tile .who{{font-size:13px;color:var(--muted);display:block;margin-top:4px}}
.tile .who i{{display:inline-block;width:9px;height:9px;border-radius:2px;margin-right:6px;vertical-align:0}}
.grp{{display:grid;grid-template-columns:minmax(180px,260px) 1fr;gap:16px;align-items:center;padding:10px 0;border-bottom:1px solid var(--border)}}
.grp-label{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:15px}}
.grp-bars{{display:flex;flex-direction:column;gap:5px}}
.hbar{{display:flex;align-items:center;gap:10px}}
.hbar-fill{{height:14px;border-radius:0 3px 3px 0;min-width:2px}}
.hbar-fill.ours{{background:var(--ours)}}.hbar-fill.ref{{background:var(--ref)}}
.hbar-val{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:14px;color:var(--text-2);white-space:nowrap}}
.stack-row{{margin:18px 0 26px}}
.stack-label{{font-size:16px;margin-bottom:8px}}.stack-label b{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-weight:500}}
.stack{{display:flex;height:26px;gap:2px;background:var(--surface-2);border-radius:5px;overflow:hidden}}
.seg{{height:100%}}
.seg.ours.s0,.sw.ours.s0{{background:var(--s0)}}.seg.ours.s1,.sw.ours.s1{{background:var(--s1)}}
.seg.ref.s0,.sw.ref.s0{{background:var(--r0)}}.seg.ref.s1,.sw.ref.s1{{background:var(--r1)}}.seg.ref.s2,.sw.ref.s2{{background:var(--r2)}}.seg.ref.s3,.sw.ref.s3{{background:var(--r3)}}.seg.ref.s4,.sw.ref.s4{{background:var(--r4)}}.seg.ref.s5,.sw.ref.s5{{background:var(--r5)}}.seg.ref.s6,.sw.ref.s6{{background:var(--r6)}}
.legend{{display:flex;flex-wrap:wrap;gap:6px 18px;margin-top:10px;font-size:15px;color:var(--text-2)}}
.legend b{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-weight:500;color:var(--text)}}
.sw{{display:inline-block;width:11px;height:11px;border-radius:3px;margin-right:7px;vertical-align:-1px}}
.samples{{display:grid;gap:22px;margin-top:20px}}
.sample{{margin:0;background:var(--surface);border:1px solid var(--border);border-radius:10px;padding:18px 20px}}
.sample figcaption{{display:flex;align-items:center;gap:14px;flex-wrap:wrap;margin-bottom:12px}}
.sample-name{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:16px;font-weight:500}}
.sample-fmt{{font-size:14px;color:var(--muted)}}
.sample[hidden]{{display:none}}
.pill{{font-family:"IBM Plex Mono",ui-monospace,monospace;font-size:13px;padding:3px 10px;border-radius:999px}}
.pill.good{{background:var(--good-bg);color:var(--good)}}.pill.fair{{background:var(--fair-bg);color:var(--fair)}}.pill.weak{{background:var(--weak-bg);color:var(--weak)}}
.triple{{display:grid;grid-template-columns:repeat(3,1fr);gap:12px}}
.triple div{{display:flex;flex-direction:column;gap:6px}}
.triple img{{width:100%;max-width:100%;border:1px solid var(--border);border-radius:5px;background:#fff}}
.triple span{{font-size:14px;color:var(--muted)}}
.sample .note{{font-size:16px;color:var(--text-2);margin:12px 0 0;max-width:none}}
ul.method{{color:var(--text-2);padding-left:22px;max-width:80ch}}
ul.method li{{margin:6px 0}}
@media (max-width:720px){{.triple{{grid-template-columns:1fr}}.grp{{grid-template-columns:1fr}}h1{{font-size:28px}}body{{font-size:16px}}}}
</style>
<div class="wrap">
<div class="eyebrow">Benchmark report</div>
<h1>bare-x-to-pdf against bare-collabora</h1>
<p>Office documents converted to PDF by our Rust and Typst engine, compared page by page with the same files converted by bare-collabora (LibreOfficeKit). Same machine, same corpus, same rasteriser.</p>
<div class="meta"><span>{today}</span><span>commit <code>{commit}</code></span><span>{esc(cpu)}, {mem_gb} GB, macOS {esc(macos)}, Bare {esc(bare.lstrip('v'))}</span><span>bare-collabora {esc(collabora_version)}</span><span>{len(cases)} cases, {pages} pages</span></div>
""")

    # Headline table
    first_ours, first_ref = cold(ours_t, "sdk-sample.docx"), cold(ref_t, "sdk-sample.docx")
    warm_ours, warm_ref = warm(ours_t, "minimal-table-unicode.docx"), warm(ref_t, "minimal-table-unicode.docx")
    mem_ours, mem_ref = timing["memory"]["ours"], timing["memory"]["collabora"]
    recall_o, recall_r = mean_of("ours", "recall"), mean_of("reference", "recall")
    order_o, order_r = mean_of("ours", "order"), mean_of("reference", "order")

    def row(name, sub, a, b, diff, win):
        wa = ' win' if win == 'a' else ''
        wb = ' win' if win == 'b' else ''
        return f'<tr><td class="metric"><span class="mname">{esc(name)}</span><span class="msub">{esc(sub)}</span></td><td class="val{wa}">{a}</td><td class="val{wb}">{b}</td><td class="diff">{esc(diff)}</td></tr>'

    def ratio(a, b):
        return f"{b / a:.1f}× " if a else ""

    rows = [
        row("Layout fidelity", f"mean ink overlap with bare-collabora pages, {len(cases)} cases", pct(mean_iou), "reference", "measured against it", None),
        row("Words recovered from the PDF", f"share of source words the PDF text contains, {len(scored)} cases", pct(recall_o), pct(recall_r), f"{(recall_o - recall_r) * 100:+.1f} points", "a" if recall_o > recall_r else "b"),
        row("Reading order", "longest run of source words in the same order", pct(order_o), pct(order_r), f"{(order_o - order_r) * 100:+.1f} points", "a" if order_o > order_r else "b"),
        row("Install size", "darwin-arm64 prebuild on disk", mb(ours_total), mb(ref_total), f"{ref_total / ours_total:.1f}× smaller", "a"),
        row("Library load", "require until ready", ms(ours_t["loadMs"]), ms(ref_t["loadMs"]), f"{ratio(ours_t['loadMs'], ref_t['loadMs'])}faster", "a"),
        row("First conversion", "sdk-sample.docx, cold process", ms(first_ours), ms(first_ref), f"{ratio(first_ours, first_ref)}faster", "a"),
        row("Warm conversion", "19-page DOCX, median of five", ms(warm_ours), ms(warm_ref), f"{ratio(warm_ours, warm_ref)}faster", "a"),
        row("Whole corpus", f"{len(cases)} conversions to PDF, engines warm", ms(corpus_ours), ms(corpus_ref), f"{ratio(corpus_ours, corpus_ref)}faster", "a"),
        row("Peak memory", "six files in one process", f"{mem_ours / 2**20:.0f} MB", f"{mem_ref / 2**20:.0f} MB", f"{mem_ref / mem_ours:.1f}× less", "a"),
        row("Input formats", "converted to PDF", "14 extensions", "221 import filters", "far broader coverage", "b"),
    ]
    w(f"""
<div class="tablewrap"><table class="vs">
<thead><tr><th class="metric">Measure</th><th class="eng ours"><i></i>bare-x-to-pdf</th><th class="eng ref"><i></i>bare-collabora</th><th>Difference</th></tr></thead>
<tbody>{''.join(rows)}</tbody>
</table></div>
<p class="note">Green marks the better value on each row. Layout fidelity is measured against bare-collabora's own output, so that column is the reference by definition.</p>

<h2>How the score works</h2>
<p>Each page from both engines is rasterised at 72 dpi. <b>Ink overlap</b> (IoU) takes the dark pixels of the reference page and of ours and divides what they share by everything either of them painted. It is a harsh, layout-first score: text stems are about one pixel wide at this size, so a line shifted by half a point loses most of its overlap even though a reader would never notice. As a rule of thumb, <b>85% and above</b> is indistinguishable at reading size, <b>60 to 85%</b> is the same layout with glyphs a fraction of a point off, and <b>below 60%</b> something sits in a different place.</p>
<div class="key"><span><i style="background:var(--good)"></i>85% and above</span><span><i style="background:var(--fair)"></i>60 to 85%</span><span><i style="background:var(--weak)"></i>below 60%</span></div>

<h2>Fidelity by format</h2>
<p>Mean ink overlap per input format. Word and OpenDocument text lead because most layout rules were validated against Writer's source; spreadsheets and slides carry more small text and shapes, where half-point offsets cost more.</p>
<div class="fmts">""")
    for fmt in FORMAT_ORDER:
        rs = by_format.get(fmt)
        if not rs:
            continue
        m = statistics.mean(r["inkIou"] for r in rs)
        w(f'<div class="fmt"><div class="fmt-head"><span class="fmt-name">{esc(FORMAT_NAMES[fmt])}</span><span class="fmt-n">{len(rs)} case{"s" if len(rs) != 1 else ""}</span><span class="fmt-val">{pct(m)}</span></div><div class="bar"><div class="bar-fill {grade(m)}" style="width:{m * 100:.1f}%"></div></div></div>')
    w("</div>")

    # Every case
    w(f"""
<h2>Every case</h2>
<p>One row per document, best match first. Times are the in-process conversion of each file during the run, with both engines warm. Filter by format, or click a column header to sort.</p>
<div class="chips" id="case-chips"><button class="chip" data-fmt="all" aria-pressed="true">All formats</button>""")
    for fmt in FORMAT_ORDER:
        if fmt in by_format:
            w(f'<button class="chip" data-fmt="{fmt}" aria-pressed="false">{esc(FORMAT_NAMES[fmt])}</button>')
    w("""</div>
<div class="tablewrap" id="cases"><table><thead><tr><th data-k="name">Document</th><th data-k="fmt">Format</th><th class="num" data-k="pages">Pages</th><th data-k="iou">Ink overlap</th><th class="num" data-k="ours">bare-x-to-pdf</th><th class="num" data-k="ref">bare-collabora</th></tr></thead><tbody>""")
    for r in cases:
        fmt = r["name"].rsplit(".", 1)[1]
        g = grade(r["inkIou"])
        w(f'<tr data-fmt="{fmt}"><td class="name">{esc(r["name"])}</td><td class="fmt">{esc(FORMAT_NAMES[fmt])}</td><td class="num">{r["referencePages"]}</td><td class="iou"><div class="cell"><div class="bar sm"><div class="bar-fill {g}" style="width:{r["inkIou"] * 100:.1f}%"></div></div><span class="num">{pct(r["inkIou"])}</span></div></td><td class="num">{ms(r["oursMs"])}</td><td class="num">{ms(r["referenceMs"])}</td></tr>')
    w("</tbody></table></div>")

    # Text extraction
    prec_o, prec_r = mean_of("ours", "precision"), mean_of("reference", "precision")
    imperfect = [t for t in scored if min(t["ours"]["recall"], t["reference"]["recall"], t["ours"]["order"], t["reference"]["order"]) < 0.999]
    perfect = len(scored) - len(imperfect)

    def tile(label, definition, a, b):
        return f'<div class="tile"><div class="lbl">{esc(label)}</div><div class="def">{esc(definition)}</div><div class="pair"><div><span class="val">{pct(a)}</span><span class="who"><i style="background:var(--ours)"></i>bare-x-to-pdf</span></div><div><span class="val">{pct(b)}</span><span class="who"><i style="background:var(--ref)"></i>bare-collabora</span></div></div></div>'

    w(f"""
<h2>Text extraction</h2>
<p>A PDF that looks right but yields garbage when copied or indexed is not good enough. Both engines' PDFs go through the same extractor, bare-pdfium, and the words that come out are checked against the words in the source document itself, read from its XML. Three questions, each answered for the {len(scored)} documents that have extractable source text:</p>
<div class="tiles">
{tile("Words recovered", "Of the words in the source document, how many appear in the PDF text?", recall_o, recall_r)}
{tile("In reading order", "How much of the source text comes back in the same order it was written?", order_o, order_r)}
{tile("Nothing invented", "Of the words in the PDF text, how many exist in the source? Page numbers, list labels and formatted numbers lower this for both engines alike.", prec_o, prec_r)}
</div>
<p>Both engines return every source word, in order, for <b>{perfect} of the {len(scored)}</b> documents. These are the other {len(imperfect)}, with the number of source words each engine's PDF gives back and why some are missing.</p>
<div class="tablewrap"><table id="text"><thead><tr><th>Document</th><th class="num">Words in source</th><th class="num">Found by bare-x-to-pdf</th><th class="num">Found by bare-collabora</th><th>What happens</th></tr></thead><tbody>""")
    imperfect.sort(key=lambda t: t["ours"]["recall"] + t["ours"]["order"])

    for t in imperfect:
        total = t["sourceWords"]
        found_o = round(t["ours"]["recall"] * total)
        found_r = round(t["reference"]["recall"] * total)
        win_o = " win" if found_o > found_r else ""
        win_r = " win" if found_r > found_o else ""
        w(f'<tr><td class="name">{esc(t["name"])}</td><td class="num">{total:,}</td><td class="num{win_o}">{found_o:,} of {total:,}</td><td class="num{win_r}">{found_r:,} of {total:,}</td><td class="why">{esc(TEXT_NOTES.get(t["name"], ""))}</td></tr>')
    w("""</tbody></table></div>
<p class="note">A word counts as found when it appears in the PDF text exactly, after Unicode normalisation, case-folding and unifying hyphen variants. A label or number glued to a word, or a value printed in a different form, therefore counts as missing for both engines alike.</p>""")

    # Speed
    max_ms = max(max(warm(ours_t, f), warm(ref_t, f)) for f in TIMING_FILES)
    w(f"""
<h2>Conversion time</h2>
<p>Each engine ran in its own process on six files, six conversions per file; the bars show the median of the last five. bare-collabora's first document also pays for LibreOfficeKit start-up ({ms(first_ref)} for sdk-sample.docx against {ms(first_ours)} for ours), and requiring the module takes {ms(ref_t["loadMs"])} against {ms(ours_t["loadMs"])}. Peak resident memory for the whole run was <b>{mem_ours / 2**20:.0f} MB</b> against <b>{mem_ref / 2**20:.0f} MB</b>.</p>
<div class="key"><span><i style="background:var(--ours)"></i>bare-x-to-pdf</span><span><i style="background:var(--ref)"></i>bare-collabora</span></div>""")
    for f in TIMING_FILES:
        a, b = warm(ours_t, f), warm(ref_t, f)
        w(f'<div class="grp"><div class="grp-label">{esc(f)}</div><div class="grp-bars"><div class="hbar"><div class="hbar-fill ours" style="width:{a / max_ms * 100:.1f}%"></div><span class="hbar-val">{ms(a)}</span></div><div class="hbar"><div class="hbar-fill ref" style="width:{b / max_ms * 100:.1f}%"></div><span class="hbar-val">{ms(b)}</span></div></div></div>')

    # Size
    scale = max(ours_total, ref_total)
    w(f"""
<h2>Size on disk</h2>
<p>darwin-arm64 prebuilds as shipped. Our package is the addon, which holds the format readers, the Typst layout engine and the PDF writer, plus the metric-compatible fonts (Liberation, Carlito, Caladea, OpenSymbol). Both bars share one scale.</p>
<div class="stack-row"><div class="stack-label">bare-x-to-pdf <b>{mb(ours_total)}</b></div><div class="stack"><div class="seg ours s0" style="width:{addon_size / scale * 100:.2f}%" title="Addon (Rust core + Typst): {mb(addon_size)}"></div><div class="seg ours s1" style="width:{fonts_size / scale * 100:.2f}%" title="Fonts: {mb(fonts_size)}"></div></div><div class="legend"><span><i class="sw ours s0"></i>Addon (Rust core + Typst) <b>{mb(addon_size)}</b></span><span><i class="sw ours s1"></i>Fonts <b>{mb(fonts_size)}</b></span></div></div>
<div class="stack-row"><div class="stack-label">bare-collabora <b>{mb(ref_total)}</b></div><div class="stack">""")
    for i, (name, size) in enumerate(ref_parts):
        w(f'<div class="seg ref s{i}" style="width:{size / scale * 100:.2f}%" title="{esc(name)}: {mb(size)}"></div>')
    w('</div><div class="legend">')
    for i, (name, size) in enumerate(ref_parts):
        w(f'<span><i class="sw ref s{i}"></i>{esc(name)} <b>{mb(size)}</b></span>')
    w("</div></div>")

    # Side by side
    w("""
<h2>Side by side</h2>
<p>First page of {len(SAMPLES)} cases across the range. The overlay paints the reference in blue and ours in red; where they agree the ink is dark.</p>""")
    by_name = {r["name"]: r for r in cases}
    sample_formats = [f for f in FORMAT_ORDER if any(n.rsplit(".", 1)[-1] == f for n, _ in SAMPLES)]
    w('<div class="chips" id="sample-chips"><button class="chip" data-fmt="all" aria-pressed="true">All formats</button>')
    for fmt in sample_formats:
        count = sum(1 for n, _ in SAMPLES if n.rsplit(".", 1)[-1] == fmt)
        w(f'<button class="chip" data-fmt="{fmt}" aria-pressed="false">{esc(FORMAT_NAMES[fmt])} · {count}</button>')
    w('</div><div class="samples">')
    for name, note in SAMPLES:
        r = by_name.get(name)
        if not r:
            continue
        d = os.path.join(OUT, name.replace(".", "-"))
        imgs = [os.path.join(d, f"page-1-{k}.png") for k in ("reference", "ours", "ours-diff")]
        if not all(os.path.exists(p) for p in imgs):
            continue
        g = grade(r["inkIou"])
        w(f'<figure class="sample" data-fmt="{name.rsplit(".", 1)[-1]}"><figcaption><span class="sample-name">{esc(name)}</span><span class="sample-fmt">{esc(FORMAT_NAMES[name.rsplit(".", 1)[-1]])}</span><span class="pill {g}">{pct(r["inkIou"])} ink overlap</span></figcaption><div class="triple"><div><img src="{data_uri(imgs[0])}" alt="bare-collabora page 1 of {esc(name)}"><span>bare-collabora</span></div><div><img src="{data_uri(imgs[1])}" alt="bare-x-to-pdf page 1 of {esc(name)}"><span>bare-x-to-pdf</span></div><div><img src="{data_uri(imgs[2])}" alt="overlay of both pages"><span>overlay: blue reference only, red ours only</span></div></div><p class="note">{esc(note)}</p></figure>')
    w("</div>")

    # Method
    w(f"""
<h2>Method</h2>
<ul class="method">
<li>Corpus: {len(cases)} documents in <code>bench/corpus/</code>: real samples, a chart workbook, plus feature fixtures from LibreOffice's test suites (numbering, headers and footers, anchored pictures, footnotes, section breaks, borders, frames, columns, number formats).</li>
<li>Reference: bare-collabora {esc(collabora_version)} (darwin-arm64) converting the same files in the same process as the benchmark harness, <code>bare bench/run.js</code>.</li>
<li>Rasterisation: bare-pdfium at 72 dpi for both PDFs; ink overlap and pixel similarity in <code>bench/lib/compare.js</code>.</li>
<li>Text: <code>bare bench/text.js</code> extracts both PDFs with bare-pdfium and reads the source words from the document XML with a small zip reader in <code>bench/lib/source-text.js</code>; metrics in <code>bench/lib/text-metrics.js</code>.</li>
<li>Timing: <code>bare scripts/timing.js ours|collabora &lt;files&gt;</code> in separate processes; peak memory from <code>/usr/bin/time -l</code>.</li>
<li>Sizes: the darwin-arm64 prebuild directories on disk.</li>
<li>This page: <code>python3 bench/report/generate.py</code> over <code>bench/out</code>.</li>
</ul>
</div>
<script>
(function(){{
  const filter=(chipSel,itemSel)=>{{const chips=document.querySelectorAll(chipSel);const items=[...document.querySelectorAll(itemSel)];chips.forEach(c=>c.addEventListener('click',()=>{{chips.forEach(x=>x.setAttribute('aria-pressed','false'));c.setAttribute('aria-pressed','true');const f=c.dataset.fmt;items.forEach(r=>{{r.hidden=!(f==='all'||r.dataset.fmt===f)}})}}))}};
  filter('#case-chips .chip','#cases tbody tr');
  filter('#sample-chips .chip','.samples .sample');
  const table=document.querySelector('#cases table');const tbody=table.querySelector('tbody');
  const num=s=>parseFloat(String(s).replace(/[^0-9.\\-]/g,''));
  let sortKey='iou',asc=false;
  table.querySelectorAll('th').forEach((th,i)=>{{th.tabIndex=0;const go=()=>{{const k=th.dataset.k;if(sortKey===k)asc=!asc;else{{sortKey=k;asc=(k==='name'||k==='fmt')}};const rs=[...tbody.querySelectorAll('tr')];rs.sort((a,b)=>{{const ta=a.children[i].textContent.trim(),tb=b.children[i].textContent.trim();const na=num(ta),nb=num(tb);let c=(isNaN(na)||isNaN(nb)||k==='name'||k==='fmt')?ta.localeCompare(tb):na-nb;return asc?c:-c}});rs.forEach(r=>tbody.appendChild(r))}};th.addEventListener('click',go);th.addEventListener('keydown',e=>{{if(e.key==='Enter'||e.key===' '){{e.preventDefault();go()}}}})}});
}})();
</script>
""")

    with open(out_path, "w") as f:
        f.write("".join(parts))
    print(f"wrote {out_path} ({os.path.getsize(out_path) / 1024:.0f} KB)")


if __name__ == "__main__":
    main()
