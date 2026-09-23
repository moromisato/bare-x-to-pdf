const fs = require('bare-fs')
const path = require('bare-path')

const root = path.join(__dirname, '..')
const target = `${Bare.platform}-${Bare.arch}`
const name = `${require('../package.json').name}-${target}`
const link = path.join(root, 'node_modules', name)

try {
  fs.rmSync(link, { recursive: true })
} catch {}

fs.symlinkSync(path.join('..', 'npm', target), link, 'dir')
console.log(`${link} -> npm/${target}`)
