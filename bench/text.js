const fs = require('bare-fs')
const path = require('bare-path')
const pdfium = require('bare-pdfium')
const { sourceText } = require('./lib/source-text')
const { compare, agreement, unmappable, tokens } = require('./lib/text-metrics')

const args = parseArgs(Bare.argv.slice(2))
const corpusDir = path.resolve(args.corpus || path.join(__dirname, 'corpus'))
const outDir = path.resolve(args.out || path.join(__dirname, 'out'))
const filter = args.filter ? new RegExp(args.filter) : null

const files = fs
  .readdirSync(corpusDir)
  .filter((f) => !f.startsWith('.') && (!filter || filter.test(f)))
  .sort()

const results = []
for (const file of files) {
  const ext = path.extname(file).slice(1).toLowerCase()
  const dir = path.join(outDir, file.replace(/\.([^.]+)$/, '-$1'))
  const row = { name: file, task: `${ext}>pdf` }
  try {
    const source = { text: sourceText(fs.readFileSync(path.join(corpusDir, file)), ext) }
    const oursPath = path.join(dir, 'ours.pdf')
    const referencePath = path.join(dir, 'reference.pdf')
    if (!fs.existsSync(oursPath)) throw new Error('run bench/run.js first')
    const ours = extract(fs.readFileSync(oursPath))
    const reference = fs.existsSync(referencePath) ? extract(fs.readFileSync(referencePath)) : null
    row.sourceWords = source.text === null ? null : tokens(source.text).length
    row.ours = engineRow(source.text, ours)
    row.reference = reference ? engineRow(source.text, reference) : null
    row.agreement = reference ? agreement(ours.text, reference.text) : null
    fs.writeFileSync(path.join(dir, 'text-ours.txt'), ours.text)
    if (reference) fs.writeFileSync(path.join(dir, 'text-reference.txt'), reference.text)
    if (source.text !== null) fs.writeFileSync(path.join(dir, 'text-source.txt'), source.text)
  } catch (error) {
    row.error = error.message
  }
  results.push(row)
}

fs.writeFileSync(path.join(outDir, 'text-report.json'), JSON.stringify({ date: new Date().toISOString(), results }, null, 2))
print(results)

function extract(bytes) {
  const start = Date.now()
  const doc = pdfium.open(bytes)
  try {
    const pages = []
    for (let i = 0; i < doc.pageCount(); i++) pages.push(doc.extractText(i))
    const text = pages.join('\n')
    return { text, ms: Date.now() - start, pages: pages.length }
  } finally {
    doc.close()
  }
}

function engineRow(source, extracted) {
  const row = { words: tokens(extracted.text).length, ms: extracted.ms, unmappable: unmappable(extracted.text) }
  if (source !== null) Object.assign(row, compare(source, extracted.text))
  return row
}

function print(rows) {
  const pct = (v) => (v === undefined || v === null ? '-' : `${(v * 100).toFixed(1)}%`)
  const cols = [
    ['case', (r) => r.name],
    ['src words', (r) => r.sourceWords ?? '-'],
    ['ours words', (r) => r.ours?.words ?? '-'],
    ['ref words', (r) => r.reference?.words ?? '-'],
    ['recall ours', (r) => pct(r.ours?.recall)],
    ['recall ref', (r) => pct(r.reference?.recall)],
    ['precision ours', (r) => pct(r.ours?.precision)],
    ['precision ref', (r) => pct(r.reference?.precision)],
    ['order ours', (r) => pct(r.ours?.order)],
    ['order ref', (r) => pct(r.reference?.order)],
    ['agreement', (r) => pct(r.agreement)],
    ['status', (r) => r.error || 'ok']
  ]
  const table = rows.map((r) => cols.map(([, f]) => String(f(r))))
  const widths = cols.map(([h], i) => Math.max(h.length, ...table.map((row) => row[i].length)))
  const line = (cells) => cells.map((c, i) => c.padEnd(widths[i])).join('  ')
  console.log(line(cols.map(([h]) => h)))
  console.log(line(widths.map((w) => '-'.repeat(w))))
  for (const row of table) console.log(line(row))
  const scored = rows.filter((r) => r.sourceWords > 0 && r.ours?.recall !== undefined && r.reference?.recall !== undefined)
  const mean = (f) => scored.reduce((a, r) => a + f(r), 0) / Math.max(1, scored.length)
  console.log(`\n${scored.length} cases with source text and both engines: recall ours ${pct(mean((r) => r.ours.recall))} / ref ${pct(mean((r) => r.reference.recall))}, precision ours ${pct(mean((r) => r.ours.precision))} / ref ${pct(mean((r) => r.reference.precision))}, order ours ${pct(mean((r) => r.ours.order))} / ref ${pct(mean((r) => r.reference.order))}`)
}

function parseArgs(argv) {
  const out = {}
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith('--')) {
      const key = argv[i].slice(2)
      const next = argv[i + 1]
      if (next && !next.startsWith('--')) {
        out[key] = next
        i++
      } else out[key] = true
    }
  }
  return out
}
