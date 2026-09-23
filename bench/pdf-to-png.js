const fs = require('bare-fs')
const path = require('bare-path')
const { rasterize } = require('./lib/raster')
const { encodeGray } = require('./lib/png')

const [, , input, outDir = '.', scaleArg = '1'] = Bare.argv
if (!input) {
  console.error('usage: bare pdf-to-png.js <file.pdf> [out-dir] [scale]')
  Bare.exit(1)
}

fs.mkdirSync(outDir, { recursive: true })
const stem = path.basename(input).replace(/\.pdf$/i, '')
const pages = rasterize(fs.readFileSync(input), { scale: Number(scaleArg) })
pages.forEach((page, i) => {
  const file = path.join(outDir, `${stem}-${i + 1}.png`)
  fs.writeFileSync(file, encodeGray(page.gray, page.width, page.height))
  console.log(`${file} ${page.width}x${page.height}`)
})
