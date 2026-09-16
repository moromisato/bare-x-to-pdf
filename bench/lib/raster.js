const pdfium = require('bare-pdfium')

function rasterize(bytes, { scale = 1 } = {}) {
  const doc = pdfium.open(bytes)
  try {
    const pages = []
    const count = doc.pageCount()
    for (let i = 0; i < count; i++) {
      const size = doc.pageSize(i)
      const { width, height, data } = doc.render(i, { scale })
      pages.push({ width, height, points: size, gray: toGray(data, width, height) })
    }
    return pages
  } finally {
    doc.close()
  }
}

function toGray(rgba, width, height) {
  const gray = new Uint8Array(width * height)
  for (let i = 0, j = 0; i < gray.length; i++, j += 4) {
    gray[i] = (rgba[j] * 299 + rgba[j + 1] * 587 + rgba[j + 2] * 114) / 1000
  }
  return gray
}

module.exports = { rasterize }
