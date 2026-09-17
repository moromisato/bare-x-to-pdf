const fs = require('bare-fs')
const path = require('bare-path')

const [engine, list, reference = '../../bare-collabora'] = Bare.argv.slice(2)
if (!engine || !list) {
  console.error('usage: bare scripts/timing.js <ours|collabora> <file,file,...> [reference-dir]')
  Bare.exit(1)
}
const files = list.split(',')
const tmp = '/tmp/simple-converter-timing.pdf'
const rounds = 6

const t0 = Date.now()
let run
if (engine === 'ours') {
  const { convert } = require('..')
  run = (file) => {
    const bytes = fs.readFileSync(file)
    return () => fs.writeFileSync(tmp, convert(bytes, { to: 'pdf' }))
  }
} else {
  const { Document } = require(path.resolve(reference))
  run = (file) => () => new Document(file).saveAs(tmp, 'pdf')
}
const result = { engine, loadMs: Date.now() - t0, files: [] }
for (const file of files) {
  const step = run(file)
  const runs = []
  for (let i = 0; i < rounds; i++) {
    const start = Date.now()
    step()
    runs.push(Date.now() - start)
  }
  result.files.push({ file: path.basename(file), coldMs: runs[0], warmMs: runs.slice(1) })
}
console.log(JSON.stringify(result))
