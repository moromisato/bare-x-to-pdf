# simple-converter

Office documents to PDF for [Bare](https://github.com/holepunchto/bare), in a
native addon roughly ten times smaller than `bare-collabora`. The engine is
written in Rust and lays pages out with [Typst](https://typst.app); PDFs are
read with [PDFium](https://pdfium.googlesource.com/pdfium/).

## Direction

The product is one pipeline: every input format is read into a shared document
model, Typst lays the pages out, and the PDF comes out the other end. Readers
are the only per-format work; layout, fonts, tables, images, headers, lists and
footnotes are shared. The formats that matter are the Office ones, in this
order:

| Input                  | Status              | Notes                                                                                                                                 |
| ---------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| DOCX, DOCM, DOTX, DOTM | done                | Flow layout through the shared model.                                                                                                 |
| DOC, DOT               | done, first version | Word 97-2003 binary reader: text, styles, lists, sections, headers, footnotes, tables, inline pictures. Word 6/95 files are rejected. |
| ODT, OTT, FODT         | done, first version | ODF text reader: styles, page masters with headers and footers, lists, tables, frames, notes.                                         |
| PPTX                   | working extra       | Slides as fixed-layout pages; not a focus.                                                                                            |

[FORMATS.md](FORMATS.md) lists every import filter bare-collabora ships and
what it would take to render each one here. PDF to DOCX also exists
(fixed-layout output) and stays, but it is no longer the focus. Plain text and Markdown are deliberately out of scope.

The goal is faithful output, not pixel identity with Office. Fidelity is
measured, not guessed: `bench/` converts a corpus with this library and with
`bare-collabora` side by side and scores the rendered pages against each other.

## Usage

```js
const converter = require('simple-converter')

const pdf = converter.convert(docxBytes, { to: 'pdf' })
const docx = converter.convert(pdfBytes, { to: 'docx' })
```

- `convert(bytes, { to, from?, fontsDir?, pdfiumPath? })` returns a `Buffer`.
  `from` is sniffed from the magic bytes when omitted.
- `detect(bytes)` returns `'docx'`, `'pdf'`, `'doc'` or `null`.
- `supports(from, to)` and `conversions()` describe the conversion matrix.

Calls are synchronous and CPU bound. Run them in a `Bare.Thread` when latency
matters; the engine holds no global state that prevents that.

## What is supported

DOCX to PDF reads the document into a flow model and hands it to Typst.

- Sections: page size and margins per section, multiple columns, page number
  start and format.
- Headers and footers: default, first page and even page variants, positioned
  at the Word header and footer distances, with PAGE and NUMPAGES fields.
- Paragraphs: alignment, indents (first line and hanging), space before and
  after with Word's HTML auto spacing rule (larger of the two unless the compat
  flag makes them additive), contextual spacing, line spacing (multiple, exact,
  at least), page breaks, tab stops (explicit, the implicit stop of a hanging
  indent, and default stops for tabs at the start of a line).
- Numbered and bulleted lists from `numbering.xml`: multi-level labels,
  restarts, letter and roman formats, bullet glyph mapping for Symbol and
  Wingdings.
- Runs: font, size, bold, italic, underline, strike, colour, highlight,
  superscript and subscript, caps and small caps, hidden text, symbols, line
  breaks, footnotes and endnotes.
- Styles: document defaults, paragraph and character style chains, theme
  fonts, the default paragraph style, numbering carried by styles.
- Tables: grid widths, column and row spans, per-cell borders, margins,
  shading, vertical alignment, minimum and exact row heights, table indent.
- Images and drawings: inline pictures, anchored pictures and text boxes
  positioned relative to page, margin, column or paragraph, square wrap beside
  a picture, top-and-bottom wrap, behind-text ordering, and the legacy VML
  `w:pict` forms of the same. PNG, JPEG, GIF and SVG media are placed; other
  formats reserve their space.
- Fonts: Word families are mapped onto the bundled metric-compatible set in
  `fonts/` (Liberation for Arial, Times New Roman and Courier New, Carlito for
  Calibri, Caladea for Cambria, OpenSymbol for Symbol and Wingdings bullets). Line heights use the same hhea metrics that
  Word and LibreOffice use, so line pitch and baselines match. The Carlito and
  Caladea files are the versions LibreOffice bundles, kept in the repository
  because newer Google Fonts releases changed their vertical metrics;
  `scripts/fetch-fonts.sh` only refreshes the Liberation family.

PDF to DOCX produces a fixed-layout document: every word becomes a text box
anchored to its page position with the original font name, size, weight, style
and colour, and everything that is not text (vector graphics, images, fills) is
rendered by PDFium into one background picture per page. Word and LibreOffice
open the result with the original layout intact; it is meant for reading and
light editing, not for reflowing.

Not yet handled: tabs in the middle of a line (approximated with the default
tab width), text wrapping tightly around pictures wider than half the text
area, tracked changes, comments, charts and SmartArt, embedded fonts in either
direction, paragraph borders, and floating shapes and hidden text in DOC input.

## Building

Requires the Bare toolchain (`npm i -g bare-make`), CMake 4 and Rust. The Rust
toolchain is pinned by `rust-toolchain.toml` and rustup installs it on first
use.

```sh
npm install
bare-make generate   # fetches Corrosion and a prebuilt PDFium for the host
bare-make build
bare-make install    # writes prebuilds/<host>/
npm test
```

`prebuilds/<host>/` ends up with `simple-converter.bare` and a
`simple-converter/` folder holding the PDFium shared library, which is loaded
at runtime from beside the addon. Fonts ship in `fonts/` and are read at
runtime, so extra font packs can be added without a rebuild.

For quick iteration on the engine without the addon:

```sh
cd core
cargo test --release
```

Set `SIMPLE_CONVERTER_DUMP_TYPST=/tmp/out.typ` to write the generated Typst
source of the last DOCX conversion, and `SIMPLE_CONVERTER_DUMP_FIXED=/tmp/out.txt`
to dump the geometry extracted from the last PDF.

## Benchmark

```sh
bare bench/run.js [--corpus bench/corpus] [--out bench/out] [--reference ../../bare-collabora] [--scale 1] [--filter name] [--no-images]
```

The corpus in `bench/corpus/` mixes real documents with feature fixtures taken
from LibreOffice's DOCX test suite (numbering, headers and footers, anchored
pictures, footnotes, section breaks). For every `.docx` in the corpus the
reference engine and this library both produce a PDF. For every `.pdf` this library produces a DOCX, which the
reference engine renders back to PDF so it can be compared with the original.
Pages are rasterised with `bare-pdfium` and scored two ways: `similarity` is
the mean pixel difference, `ink iou` is the overlap of dark pixels and is the
number to watch, since it punishes shifted text. `bench/out/<case>/` receives
both PDFs plus per-page PNGs of reference, ours and a diff (blue is reference
only, red is ours only). The previous run's scores are shown for comparison and
written to `bench/out/report.json`.

`scripts/pdf-to-png.js <file.pdf> [out-dir] [scale]` rasterises any PDF for a
quick look.

## Size

On darwin-arm64 the stripped addon is about 26 MB, the PDFium library 7 MB
and the fonts 7 MB, against 337 MB for `bare-collabora`.

## Layout

- `index.js`, `binding.c`: the Bare addon and its JS API.
- `core/`: the Rust engine. `docx/`, `doc/`, `odt/` and `pptx/` read their
  formats into `model.rs`; `docx/writer.rs` writes fixed-layout DOCX;
  `typst_backend/` emits Typst and renders PDF; `pdf/` extracts text and
  backgrounds with PDFium.
- `cmake/`: Corrosion for the Rust build and the PDFium download.
- `bench/`: the comparison harness. `bench/corpus/` holds the documents.
- `fonts/`: bundled fonts with their licences.

## Licensing

Apache-2.0 for this package. PDFium is BSD. Typst and its dependencies are
Apache-2.0 or MIT. The fonts are under the SIL Open Font License; see the
`LICENSE.*` files in `fonts/`.
