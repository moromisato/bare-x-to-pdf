const path = require('bare-path')
const fs = require('bare-fs')

const DEFAULT_DIR = path.resolve(__dirname, '..', '..', '..', '..', 'bare-collabora')

let loaded = null

function load(dir = DEFAULT_DIR) {
  if (loaded && loaded.dir === dir) return loaded
  if (!fs.existsSync(path.join(dir, 'package.json'))) {
    throw new Error(
      `reference engine not found at ${dir}, pass --reference <path to bare-collabora>`
    )
  }
  const { Document } = require(dir)
  loaded = {
    dir,
    convert(src, out, format) {
      new Document(src).saveAs(out, format)
    }
  }
  return loaded
}

module.exports = { load, DEFAULT_DIR }
