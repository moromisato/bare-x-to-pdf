const fs = require('bare-fs')
const path = require('bare-path')
const { convert } = require('..')

const [input, output] = Bare.argv.slice(2)
if (!input || !output) {
  console.error('usage: bare scripts/convert.js <input> <output>')
  Bare.exit(1)
}

const to = path.extname(output).slice(1)
const from = path.extname(input).slice(1) || undefined
fs.writeFileSync(output, convert(fs.readFileSync(input), { from, to }))
