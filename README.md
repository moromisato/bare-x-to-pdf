# bare-x-to-pdf

Convert office documents to PDF for [Bare](https://github.com/holepunchto/bare).
A native addon with a Rust engine that lays pages out with
[Typst](https://typst.app).

`x` is every format it reads into a PDF today:

| Family            | Extensions                         |
| ----------------- | ---------------------------------- |
| Word (OOXML)      | `.docx`, `.docm`, `.dotx`, `.dotm` |
| Word 97–2003      | `.doc`, `.dot`                     |
| OpenDocument Text | `.odt`, `.ott`, `.fodt`            |
| PowerPoint        | `.pptx`                            |
| Excel             | `.xlsx`, `.xlsm`, `.xltx`, `.xltm` |

## Install

```sh
npm install bare-x-to-pdf@beta
```

The native prebuild is an optional dependency per platform (`bare-x-to-pdf-<platform>-<arch>`),
so an install downloads only the package for its own platform, about 20 MB, plus the
fonts and sources in the main package. darwin, linux, win32, android and ios are covered;
other hosts build from source with the Bare toolchain, CMake 4 and Rust.

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
- `options.from` — source format; sniffed from the bytes when omitted.
- `options.fontsDir` — a directory of fonts to use instead of the bundled set.

### `detect(input)`

Return the detected format (`'docx'`, `'doc'`, `'odt'`, …) or `null`.

### `supports(from, to)`

Return whether the `from` → `to` conversion is supported.

### `conversions()`

Return every supported pair as `{ from, to }`.

### `FONTS_DIR`

Path to the bundled fonts directory.

## License

Apache-2.0. Typst and its dependencies are Apache-2.0 or MIT; the
bundled fonts are under the SIL Open Font License (see `fonts/`).
