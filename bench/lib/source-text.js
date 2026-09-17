const zip = require('./zip')

const entities = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" }

function unescape(text) {
  return text.replace(/&(#x[0-9a-fA-F]+|#\d+|\w+);/g, (m, code) => {
    if (code[0] === '#') return String.fromCodePoint(code[1] === 'x' ? parseInt(code.slice(2), 16) : parseInt(code.slice(1), 10))
    return entities[code] ?? m
  })
}

function stripTags(xml) {
  return unescape(
    xml
      .replace(/<text:s\b[^>]*text:c="(\d+)"[^>]*\/>/g, (m, n) => ' '.repeat(Number(n)))
      .replace(/<text:(s|tab|line-break)\b[^>]*\/>/g, ' ')
      .replace(/<\/(text:p|text:h|text:list-item|table:table-cell|w:p|a:p)>/g, '\n')
      .replace(/<w:(tab|br|cr)\b[^>]*\/>/g, ' ')
      .replace(/<[^>]+>/g, '')
  )
}

function collect(xml, tag) {
  const out = []
  const open = new RegExp(`<${tag}(?:\\s[^>]*)?>([\\s\\S]*?)</${tag}>`, 'g')
  let m
  while ((m = open.exec(xml))) out.push(unescape(m[1]))
  return out
}

function paragraphs(xml, textTag, breakTags) {
  const clean = xml.replace(/<mc:Fallback>[\s\S]*?<\/mc:Fallback>/g, '')
  const out = []
  for (const para of clean.split(/<\/(?:w:p|a:p)>/)) {
    const words = []
    const re = new RegExp(`<${textTag}(?:\\s[^>]*)?>([\\s\\S]*?)</${textTag}>|<(?:${breakTags})\\b[^>]*/>`, 'g')
    let m
    while ((m = re.exec(para))) words.push(m[1] === undefined ? ' ' : unescape(m[1]))
    const line = words.join('').trim()
    if (line) out.push(line)
  }
  return out.join('\n')
}

function docx(bytes) {
  const z = zip.open(bytes)
  const parts = z
    .names()
    .filter((n) => /^word\/(document|header\d*|footer\d*|footnotes|endnotes)\.xml$/.test(n))
    .sort((a, b) => (a === 'word/document.xml' ? -1 : b === 'word/document.xml' ? 1 : a.localeCompare(b)))
  return parts
    .map((name) =>
      paragraphs(
        z
          .text(name)
          .replace(/<w:instrText\b[^>]*>[\s\S]*?<\/w:instrText>/g, '')
          .replace(/<w:delText\b[^>]*>[\s\S]*?<\/w:delText>/g, ''),
        'w:t',
        'w:tab|w:br|w:cr'
      )
    )
    .join('\n')
}

function odtBody(xml) {
  const body = xml.match(/<office:text\b[\s\S]*?<\/office:text>/) || xml.match(/<office:body\b[\s\S]*?<\/office:body>/)
  const headers = collect(xml, 'style:header').concat(collect(xml, 'style:footer'))
  return [...headers.map(stripTags), body ? stripTags(body[0]) : ''].join('\n')
}

function odt(bytes) {
  const z = zip.open(bytes)
  return odtBody((z.text('styles.xml') || '') + (z.text('content.xml') || ''))
}

function fodt(bytes) {
  return odtBody(bytes.toString('utf8'))
}

function pptx(bytes) {
  const z = zip.open(bytes)
  const slides = z
    .names()
    .filter((n) => /^ppt\/slides\/slide\d+\.xml$/.test(n))
    .sort((a, b) => Number(a.match(/\d+/g).pop()) - Number(b.match(/\d+/g).pop()))
  return slides.map((name) => paragraphs(z.text(name), 'a:t', 'a:br')).join('\n')
}

function xlsx(bytes) {
  const z = zip.open(bytes)
  const shared = collect(z.text('xl/sharedStrings.xml') || '', 'si').map((si) => stripTags(si.replace(/<rPh\b[\s\S]*?<\/rPh>/g, '')))
  const sheets = z
    .names()
    .filter((n) => /^xl\/worksheets\/sheet\d+\.xml$/.test(n))
    .sort((a, b) => Number(a.match(/\d+/g).pop()) - Number(b.match(/\d+/g).pop()))
  const lines = []
  for (const name of sheets) {
    const xml = z.text(name).replace(/<c\b[^>]*\/>/g, '')
    const cells = xml.matchAll(/<c\b([^>]*)>([\s\S]*?)<\/c>/g)
    for (const cell of cells) {
      const attrs = cell[1]
      const type = (attrs.match(/\bt="(\w+)"/) || [])[1]
      const inner = cell[2]
      if (type === 's') {
        const idx = Number((inner.match(/<v>(\d+)<\/v>/) || [])[1])
        if (shared[idx] !== undefined) lines.push(shared[idx])
      } else if (type === 'inlineStr') {
        lines.push(stripTags(inner))
      } else if (type === 'str' || type === 'b' || type === 'n' || !type) {
        const v = (inner.match(/<v>([\s\S]*?)<\/v>/) || [])[1]
        if (v !== undefined) lines.push(type === 'b' ? (v === '1' ? 'TRUE' : 'FALSE') : unescape(v))
      }
    }
  }
  for (const name of z.names().filter((n) => /^xl\/drawings\/drawing\d+\.xml$/.test(n)).sort()) {
    const text = paragraphs(z.text(name), 'a:t', 'a:br')
    if (text) lines.push(text)
  }
  return lines.join('\n')
}

// Text a reader can expect to find in the converted PDF, or null when the
// source format has no cheap independent extractor.
function sourceText(bytes, extension) {
  switch (extension) {
    case 'docx':
    case 'docm':
    case 'dotx':
    case 'dotm':
      return docx(bytes)
    case 'odt':
    case 'ott':
      return odt(bytes)
    case 'fodt':
      return fodt(bytes)
    case 'pptx':
      return pptx(bytes)
    case 'xlsx':
    case 'xlsm':
      return xlsx(bytes)
    default:
      return null
  }
}

module.exports = { sourceText }
