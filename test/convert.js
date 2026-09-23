const test = require('brittle')
const fs = require('bare-fs')
const path = require('bare-path')
const pdfium = require('bare-pdfium')
const converter = require('..')

const fixture = (name) => fs.readFileSync(path.join(__dirname, 'fixtures', name))

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
  const pdf = converter.convert(fixture('minimal-table-unicode.docx'), { to: 'pdf' })
  t.is(pdf.subarray(0, 5).toString('latin1'), '%PDF-')
  const doc = pdfium.open(pdf)
  t.ok(doc.pageCount() >= 1)
  t.alike(doc.pageFlags(0), { hasImage: false, hasText: true })
  doc.close()
})

test('converts a styled docx to a letter sized pdf', (t) => {
  const pdf = converter.convert(fixture('sdk-sample.docx'), { from: 'docx', to: 'pdf' })
  const doc = pdfium.open(pdf)
  const size = doc.pageSize(0)
  t.ok(Math.abs(size.width - 612) < 1)
  t.ok(Math.abs(size.height - 792) < 1)
  doc.close()
})

test('converts a pptx deck to one pdf page per slide', (t) => {
  const pptx = fs.readFileSync(path.join(__dirname, '..', 'bench', 'corpus', 'sdk-sample.pptx'))
  t.is(converter.detect(pptx), 'pptx')
  const pdf = converter.convert(pptx, { to: 'pdf' })
  const doc = pdfium.open(pdf)
  t.is(doc.pageCount(), 4)
  const size = doc.pageSize(0)
  t.ok(Math.abs(size.width - 720) < 1)
  t.ok(Math.abs(size.height - 405) < 1)
  t.ok(doc.pageFlags(0).hasText)
  doc.close()
})

test('converts an xls workbook to pdf', (t) => {
  const xls = fixture('lo-formats.xls')
  t.is(converter.detect(xls), 'xls')
  const pdf = converter.convert(xls, { to: 'pdf' })
  const doc = pdfium.open(pdf)
  t.ok(doc.pageCount() >= 1)
  t.ok(doc.pageFlags(0).hasText)
  doc.close()
})

test('converts markdown to pdf when the format is given', (t) => {
  const markdown = Buffer.from('# Title\n\nSome **bold** text.\n\n- one\n- two\n')
  t.is(converter.detect(markdown), null)
  const pdf = converter.convert(markdown, { from: 'md', to: 'pdf' })
  const doc = pdfium.open(pdf)
  t.is(doc.pageCount(), 1)
  t.ok(doc.extractText(0).includes('Title'))
  doc.close()
})

test('converts OpenDocument spreadsheets and presentations to pdf', (t) => {
  const ods = fs.readFileSync(path.join(__dirname, '..', 'bench', 'corpus', 'odf-cond-format.ods'))
  t.is(converter.detect(ods), 'ods')
  const sheet = pdfium.open(converter.convert(ods, { to: 'pdf' }))
  t.is(sheet.pageCount(), 1)
  t.ok(sheet.extractText(0).includes('Epsilon'))
  sheet.close()
  const odp = fs.readFileSync(path.join(__dirname, '..', 'bench', 'corpus', 'odf-sdk-sample.odp'))
  t.is(converter.detect(odp), 'odp')
  const slides = pdfium.open(converter.convert(odp, { to: 'pdf' }))
  t.is(slides.pageCount(), 4)
  t.ok(Math.abs(slides.pageSize(0).width - 720) < 1)
  slides.close()
})

test('rejects unsupported and undetectable input', (t) => {
  t.exception(() => converter.convert(Buffer.from('nope'), { to: 'pdf' }), /could not detect/)
  t.exception(() => converter.convert(fixture('sdk-sample.docx'), { to: 'html' }), /not supported/)
  t.exception(() => converter.convert(Buffer.from('PK\x03\x04garbage'), { to: 'pdf' }))
})
