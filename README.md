# bare-x-to-pdf

Convert office documents to PDF for [Bare](https://github.com/holepunchto/bare).
A native addon with a Rust engine that lays pages out with
[Typst](https://typst.app).

`x` is every format it reads into a PDF today:

| Family                    | Extensions                         |
| ------------------------- | ---------------------------------- |
| Word                      | `.docx`, `.docm`, `.dotx`, `.dotm` |
| Word 97–2003              | `.doc`, `.dot`                     |
| Excel                     | `.xlsx`, `.xlsm`, `.xltx`, `.xltm` |
| Excel 97–2003             | `.xls`, `.xlt`                     |
| PowerPoint                | `.pptx`                            |
| OpenDocument Text         | `.odt`, `.ott`, `.fodt`            |
| OpenDocument Spreadsheet  | `.ods`, `.ots`, `.fods`            |
| OpenDocument Presentation | `.odp`, `.otp`, `.fodp`            |
| Markdown                  | `.md`, `.markdown`                 |

The `from` name for each is the extension without its dot, and PDF is the only output.
Markdown is CommonMark plus GitHub tables, task lists and strikethrough. Legacy
PowerPoint (`.ppt`), `.pptm`, `.potx`, `.ppsx`, Rich Text and HTML are not read.

## Install

```sh
npm install bare-x-to-pdf
```

The native prebuild is an optional dependency per platform (`bare-x-to-pdf-<platform>-<arch>`),
so an install downloads only the package for its own platform, about 17 MB packed and
42 MB unpacked, plus the main package with the fonts and sources, about 4 MB packed.
Prebuilds exist for android (arm, arm64, x64), darwin (arm64, x64), ios (arm64 and the
arm64 and x64 simulators), linux (arm64, x64) and win32 (x64); other hosts build from a
checkout with the Bare toolchain, CMake 4 and Rust.

To bundle for a different platform, ask npm for that prebuild as well:

```sh
npm install --os=android --cpu=arm64
npm install --os=ios --cpu=arm64
```

## Usage

```js
const converter = require('bare-x-to-pdf')

const pdf = converter.convert(docxBytes, { to: 'pdf' })
```

Input is a `Uint8Array` or `ArrayBuffer` and the result is a `Buffer`. Calls are
synchronous and CPU bound; run them in a `Bare.Thread` when latency matters.

## API

### `convert(input, options)`

Convert `input` and return a `Buffer`.

- `options.to` — target format, `'pdf'`. Required.
- `options.from` — source format; sniffed from the bytes when omitted. Markdown has no
  signature to sniff, so pass `from: 'md'` for it. Markdown images load only from `data:`
  URIs, since the input is bytes with no directory to resolve relative paths against.
- `options.fontsDir` — a directory of fonts to use instead of the bundled set.

### `detect(input)`

Sniff the format from the bytes and return `'docx'`, `'pptx'`, `'xlsx'`, `'doc'`, `'xls'`,
`'odt'`, `'ods'`, `'odp'`, `'fodt'`, `'fods'`, `'fodp'` or `null`. Templates and
macro-enabled files report their base format (a `.dotx` is `'docx'`, an `.ots` is
`'ods'`), which converts the same way. Markdown is plain text and always returns `null`.

### `supports(from, to)`

Return whether the `from` → `to` conversion is supported.

### `conversions()`

Return every supported pair as `{ from, to }`.

### `FONTS_DIR`

Path to the bundled fonts directory.

## Development

The platform packages in `npm/` are npm workspaces, and each declares its own `os` and `cpu`
so that installs download only one. In a checkout, install with `npm install --force` so npm
links all of them, then build the host prebuild with `npm run prebuild` and run `npm test`.
The fidelity benchmark in `bench/` has its own dependencies: `cd bench && npm install`.

## Releases

`.github/workflows/publish.yml` runs when a `v*` tag is pushed. Tag the commit whose
`package.json` has that version, after running `npm run packages` to bring the platform
packages to it. The workflow builds every platform, publishes the platform packages and then
the main package through npm trusted publishing, and creates a GitHub release. A tag newer
than npm's current `latest` publishes as `latest`; an older one as `release-<major>.<minor>`.

## License

Apache-2.0. Typst, pulldown-cmark and the other Rust dependencies are Apache-2.0 or MIT.
Liberation, Carlito and Caladea are under the SIL Open Font License (see `fonts/`);
OpenSymbol comes from LibreOffice.
