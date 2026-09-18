const path = require('bare-path')
const { binding, pdfiumPath } = require('./binding')

const FONTS_DIR = path.join(__dirname, 'fonts')

function defaultPdfiumPath() {
  return pdfiumPath
}

const CONVERSIONS = {
  docx: ['pdf'],
  docm: ['pdf'],
  dotx: ['pdf'],
  dotm: ['pdf'],
  doc: ['pdf'],
  dot: ['pdf'],
  odt: ['pdf'],
  ott: ['pdf'],
  fodt: ['pdf'],
  pptx: ['pdf'],
  xlsx: ['pdf'],
  xlsm: ['pdf'],
  xltx: ['pdf'],
  xltm: ['pdf'],
  pdf: ['docx']
}

const ZIP_MARKERS = [
  { format: 'odt', marker: 'application/vnd.oasis.opendocument.text' },
  { format: 'docx', marker: 'word/' },
  { format: 'pptx', marker: 'ppt/' },
  { format: 'xlsx', marker: 'xl/' }
]

const SIGNATURES = [
  { format: 'pdf', bytes: [0x25, 0x50, 0x44, 0x46] },
  { format: 'docx', bytes: [0x50, 0x4b, 0x03, 0x04] },
  { format: 'doc', bytes: [0xd0, 0xcf, 0x11, 0xe0] }
]

function detect(input) {
  const bytes = toBytes(input)
  for (const { format, bytes: magic } of SIGNATURES) {
    if (bytes.length < magic.length) continue
    if (!magic.every((b, i) => bytes[i] === b)) continue
    if (format === 'docx') return detectZip(bytes) || 'docx'
    return format
  }
  const head = Buffer.from(
    bytes.buffer,
    bytes.byteOffset,
    Math.min(bytes.byteLength, 4096)
  ).toString('latin1')
  if (head.trimStart().startsWith('<?xml') && head.includes('office:document')) return 'fodt'
  return null
}

function detectZip(bytes) {
  const names = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString('latin1')
  for (const { format, marker } of ZIP_MARKERS) {
    if (names.includes(marker)) return format
  }
  return null
}

function convert(input, opts = {}) {
  const bytes = toBytes(input)
  const to = normalize(opts.to)
  if (!to) throw new TypeError('convert: opts.to is required')

  const from = normalize(opts.from) || detect(bytes)
  if (!from) throw new Error('convert: could not detect the input format, pass opts.from')

  if (!supports(from, to)) {
    throw new Error(`convert: ${from} to ${to} is not supported`)
  }

  const fontsDir = opts.fontsDir || FONTS_DIR
  const pdfiumPath = opts.pdfiumPath || defaultPdfiumPath()
  return Buffer.from(binding.convert(bytes, from, to, fontsDir, pdfiumPath))
}

function supports(from, to) {
  const targets = CONVERSIONS[normalize(from)]
  return Array.isArray(targets) && targets.includes(normalize(to))
}

function conversions() {
  return Object.entries(CONVERSIONS).flatMap(([from, tos]) => tos.map((to) => ({ from, to })))
}

function normalize(format) {
  if (typeof format !== 'string') return null
  return format.toLowerCase().replace(/^\./, '')
}

function toBytes(input) {
  if (input instanceof Uint8Array) return input
  if (input instanceof ArrayBuffer) return new Uint8Array(input)
  throw new TypeError('expected a Uint8Array or ArrayBuffer')
}

exports.convert = convert
exports.detect = detect
exports.supports = supports
exports.conversions = conversions
exports.FONTS_DIR = FONTS_DIR
exports.defaultPdfiumPath = defaultPdfiumPath
