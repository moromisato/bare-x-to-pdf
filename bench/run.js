const fs = require('bare-fs')
const path = require('bare-path')
const converter = require('..')
const reference = require('./lib/reference')
const { comparePdfs } = require('./lib/compare')
const { encodeGray, encodeRgba } = require('./lib/png')

const args = parseArgs(Bare.argv)
const corpusDir = path.resolve(args.corpus || path.join(__dirname, 'corpus'))
const outDir = path.resolve(args.out || path.join(__dirname, 'out'))
const scale = Number(args.scale || 1)
const filter = args.filter || ''
const writeImages = !args['no-images']

const previous = readReport(path.join(outDir, 'report.json'))
fs.mkdirSync(outDir, { recursive: true })

const engine = reference.load(args.reference ? path.resolve(args.reference) : undefined)

const results = []
const files = fs
  .readdirSync(corpusDir)
  .filter((f) => !f.startsWith('.') && f.includes(filter))
  .sort()

for (const file of files) {
  const ext = path.extname(file).toLowerCase()
  if (ext === '.docx') results.push(...runDocx(file))
  else if (ext === '.pdf') results.push(...runPdf(file))
}

printTable(results, previous)

fs.writeFileSync(
  path.join(outDir, 'report.json'),
  JSON.stringify(
    { date: new Date().toISOString(), scale, results: results.map(stripImages) },
    null,
    2
  )
)

function runDocx(file) {
  const src = path.join(corpusDir, file)
  const dir = caseDir(file)
  const bytes = fs.readFileSync(src)

  const referencePdf = path.join(dir, 'reference.pdf')
  const refTime = time(() => engine.convert(src, referencePdf, 'pdf'))

  const result = { name: file, task: 'docx>pdf', referenceMs: refTime }
  try {
    let ours
    result.oursMs = time(() => {
      ours = converter.convert(bytes, { from: 'docx', to: 'pdf' })
    })
    fs.writeFileSync(path.join(dir, 'ours.pdf'), ours)
    Object.assign(result, score(fs.readFileSync(referencePdf), ours, dir, 'ours'))
  } catch (error) {
    result.error = error.message
  }
  return [result]
}

function runPdf(file) {
  const src = path.join(corpusDir, file)
  const dir = caseDir(file)
  const original = fs.readFileSync(src)

  const result = { name: file, task: 'pdf>docx' }
  const oursDocx = path.join(dir, 'ours.docx')
  const oursPdf = path.join(dir, 'ours-rendered.pdf')
  try {
    let docx
    result.oursMs = time(() => {
      docx = converter.convert(original, { from: 'pdf', to: 'docx' })
    })
    fs.writeFileSync(oursDocx, docx)
    engine.convert(oursDocx, oursPdf, 'pdf')
    Object.assign(result, score(original, fs.readFileSync(oursPdf), dir, 'ours'))
  } catch (error) {
    result.error = error.message
  }
  return [result]
}

function score(referencePdf, candidatePdf, dir, label) {
  const comparison = comparePdfs(referencePdf, candidatePdf, { scale })
  if (writeImages) {
    comparison.pages.forEach((page, i) => {
      if (page.missing) return
      const ref = comparison.rasters.reference[i]
      const ours = comparison.rasters.candidate[i]
      fs.writeFileSync(
        path.join(dir, `page-${i + 1}-reference.png`),
        encodeGray(ref.gray, ref.width, ref.height)
      )
      fs.writeFileSync(
        path.join(dir, `page-${i + 1}-${label}.png`),
        encodeGray(ours.gray, ours.width, ours.height)
      )
      const d = page.diffImage
      fs.writeFileSync(
        path.join(dir, `page-${i + 1}-${label}-diff.png`),
        encodeRgba(d.data, d.width, d.height)
      )
    })
  }
  return {
    referencePages: comparison.referencePages,
    candidatePages: comparison.candidatePages,
    similarity: comparison.similarity,
    inkIou: comparison.inkIou,
    pages: comparison.pages.map(({ diffImage, ...rest }) => rest)
  }
}

function printTable(rows, prev) {
  const columns = [
    ['case', (r) => `${r.name} [${r.task}]`],
    ['pages', (r) => (r.error ? '-' : `${r.candidatePages}/${r.referencePages}`)],
    ['similarity', (r) => pct(r.similarity)],
    ['ink iou', (r) => pct(r.inkIou)],
    ['prev iou', (r) => pct(previousOf(prev, r)?.inkIou)],
    ['ref ms', (r) => ms(r.referenceMs)],
    ['ours ms', (r) => ms(r.oursMs)],
    ['status', (r) => (r.error ? `error: ${r.error}` : 'ok')]
  ]
  const cells = rows.map((r) => columns.map(([, f]) => f(r)))
  const widths = columns.map(([h], i) => Math.max(h.length, ...cells.map((c) => c[i].length)))
  const line = (parts) => parts.map((p, i) => p.padEnd(widths[i])).join('  ')
  console.log(line(columns.map(([h]) => h)))
  console.log(line(widths.map((w) => '-'.repeat(w))))
  for (const c of cells) console.log(line(c))
  const scored = rows.filter((r) => !r.error)
  if (scored.length) {
    const avg = scored.reduce((s, r) => s + r.inkIou, 0) / scored.length
    console.log(`\nmean ink iou over ${scored.length} cases: ${pct(avg)}`)
  }
}

function previousOf(prev, row) {
  return prev?.results?.find((r) => r.name === row.name && r.task === row.task)
}

function caseDir(file) {
  const dir = path.join(outDir, file.replace(/\.[^.]+$/, ''))
  fs.mkdirSync(dir, { recursive: true })
  return dir
}

function stripImages(result) {
  return result
}

function readReport(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch {
    return null
  }
}

function time(fn) {
  const start = Date.now()
  fn()
  return Date.now() - start
}

function pct(v) {
  return typeof v === 'number' ? `${(v * 100).toFixed(1)}%` : '-'
}

function ms(v) {
  return typeof v === 'number' ? `${v}` : '-'
}

function parseArgs(argv) {
  const out = {}
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    if (!arg.startsWith('--')) continue
    const key = arg.slice(2)
    const next = argv[i + 1]
    if (next && !next.startsWith('--')) {
      out[key] = next
      i++
    } else {
      out[key] = true
    }
  }
  return out
}
