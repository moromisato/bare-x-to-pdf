const test = require('brittle')
const fs = require('bare-fs')
const path = require('bare-path')
const converter = require('..')

const fixture = (name) => fs.readFileSync(path.join(__dirname, 'fixtures', name))

function inspect(pdf) {
  const source = pdf.toString('latin1')
  const pages = source.match(/\/Type\s*\/Page\b/g) || []
  const box = source.match(/\/MediaBox\s*\[\s*([\d.-]+)\s+([\d.-]+)\s+([\d.-]+)\s+([\d.-]+)\s*\]/)
  return {
    header: source.slice(0, 5),
    pageCount: pages.length,
    width: box ? Number(box[3]) - Number(box[1]) : 0,
    height: box ? Number(box[4]) - Number(box[2]) : 0,
    hasFont: /\/Font\b/.test(source),
    hasImage: /\/Subtype\s*\/Image\b/.test(source)
  }
}

test('detects the input format from magic bytes', (t) => {
  t.is(converter.detect(fixture('minimal-table-unicode.docx')), 'docx')
  t.is(converter.detect(Buffer.from('%PDF-1.4\n')), null)
  t.is(converter.detect(Buffer.from([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])), 'doc')
  t.is(converter.detect(Buffer.from('plain text')), null)
})

test('lists supported conversions', (t) => {
  t.ok(converter.supports('docx', 'pdf'))
  t.ok(converter.supports('.DOCX', 'PDF'))
  t.absent(converter.supports('pdf', 'pdf'))
  t.alike(
    converter.conversions().map((c) => `${c.from}>${c.to}`),
    [
      'docx>pdf',
      'docm>pdf',
      'dotx>pdf',
      'dotm>pdf',
      'doc>pdf',
      'dot>pdf',
      'odt>pdf',
      'ott>pdf',
      'fodt>pdf',
      'ods>pdf',
      'ots>pdf',
      'fods>pdf',
      'odp>pdf',
      'otp>pdf',
      'fodp>pdf',
      'pptx>pdf',
      'xlsx>pdf',
      'xlsm>pdf',
      'xltx>pdf',
      'xltm>pdf',
      'xls>pdf',
      'xlt>pdf',
      'md>pdf',
      'markdown>pdf'
    ]
  )
})

test('converts a minimal docx with a table and unicode text to pdf', (t) => {
  const pdf = inspect(converter.convert(fixture('minimal-table-unicode.docx'), { to: 'pdf' }))
  t.is(pdf.header, '%PDF-')
  t.ok(pdf.pageCount >= 1)
  t.ok(pdf.hasFont)
  t.absent(pdf.hasImage)
})

test('converts a styled docx to a letter sized pdf', (t) => {
  const pdf = inspect(converter.convert(fixture('sdk-sample.docx'), { from: 'docx', to: 'pdf' }))
  t.ok(Math.abs(pdf.width - 612) < 1)
  t.ok(Math.abs(pdf.height - 792) < 1)
})

test('converts a pptx deck to one pdf page per slide', (t) => {
  const pptx = fs.readFileSync(path.join(__dirname, '..', 'bench', 'corpus', 'sdk-sample.pptx'))
  t.is(converter.detect(pptx), 'pptx')
  const pdf = inspect(converter.convert(pptx, { to: 'pdf' }))
  t.is(pdf.pageCount, 4)
  t.ok(Math.abs(pdf.width - 720) < 1)
  t.ok(Math.abs(pdf.height - 405) < 1)
  t.ok(pdf.hasFont)
})

test('converts an xls workbook to pdf', (t) => {
  const xls = fixture('lo-formats.xls')
  t.is(converter.detect(xls), 'xls')
  const pdf = inspect(converter.convert(xls, { to: 'pdf' }))
  t.ok(pdf.pageCount >= 1)
  t.ok(pdf.hasFont)
})

test('converts markdown to pdf when the format is given', (t) => {
  const markdown = Buffer.from('# Title\n\nSome **bold** text.\n\n- one\n- two\n')
  t.is(converter.detect(markdown), null)
  const pdf = inspect(converter.convert(markdown, { from: 'md', to: 'pdf' }))
  t.is(pdf.pageCount, 1)
  t.ok(pdf.hasFont)
})

test('converts OpenDocument spreadsheets and presentations to pdf', (t) => {
  const ods = fs.readFileSync(path.join(__dirname, '..', 'bench', 'corpus', 'odf-cond-format.ods'))
  t.is(converter.detect(ods), 'ods')
  const sheet = inspect(converter.convert(ods, { to: 'pdf' }))
  t.is(sheet.pageCount, 1)
  t.ok(sheet.hasFont)
  const odp = fs.readFileSync(path.join(__dirname, '..', 'bench', 'corpus', 'odf-sdk-sample.odp'))
  t.is(converter.detect(odp), 'odp')
  const slides = inspect(converter.convert(odp, { to: 'pdf' }))
  t.is(slides.pageCount, 4)
  t.ok(Math.abs(slides.width - 720) < 1)
})

test('rejects unsupported and undetectable input', (t) => {
  t.exception(() => converter.convert(Buffer.from('nope'), { to: 'pdf' }), /could not detect/)
  t.exception(() => converter.convert(fixture('sdk-sample.docx'), { to: 'html' }), /not supported/)
  t.exception(() => converter.convert(Buffer.from('PK\x03\x04garbage'), { to: 'pdf' }))
})
