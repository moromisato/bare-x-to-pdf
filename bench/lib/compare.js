const { rasterize } = require('./raster')

const INK = 160

function comparePdfs(referenceBytes, candidateBytes, { scale = 1 } = {}) {
  const reference = rasterize(referenceBytes, { scale })
  const candidate = rasterize(candidateBytes, { scale })

  const pages = []
  const count = Math.max(reference.length, candidate.length)

  for (let i = 0; i < count; i++) {
    const ref = reference[i]
    const ours = candidate[i]
    if (!ref || !ours) {
      pages.push({ index: i, missing: ref ? 'candidate' : 'reference', similarity: 0, inkIou: 0 })
      continue
    }
    const fitted = fit(ours, ref.width, ref.height)
    pages.push({
      index: i,
      reference: { width: ref.width, height: ref.height, points: ref.points },
      candidate: { width: ours.width, height: ours.height, points: ours.points },
      sameSize: sameSize(ref.points, ours.points),
      ...diff(ref.gray, fitted, ref.width, ref.height)
    })
  }

  return {
    referencePages: reference.length,
    candidatePages: candidate.length,
    similarity: mean(pages.map((p) => p.similarity)),
    inkIou: mean(pages.map((p) => p.inkIou)),
    pages,
    rasters: { reference, candidate }
  }
}

function diff(a, b, width, height) {
  let total = 0
  let inter = 0
  let union = 0
  const image = new Uint8Array(width * height * 4)

  for (let i = 0, j = 0; i < a.length; i++, j += 4) {
    const da = a[i]
    const db = b[i]
    total += Math.abs(da - db)

    const inkA = da < INK
    const inkB = db < INK
    if (inkA && inkB) inter++
    if (inkA || inkB) union++

    if (inkA && inkB) {
      image[j] = 40
      image[j + 1] = 40
      image[j + 2] = 40
    } else if (inkA) {
      image[j] = 30
      image[j + 1] = 90
      image[j + 2] = 220
    } else if (inkB) {
      image[j] = 220
      image[j + 1] = 50
      image[j + 2] = 40
    } else {
      image[j] = 255
      image[j + 1] = 255
      image[j + 2] = 255
    }
    image[j + 3] = 255
  }

  return {
    similarity: 1 - total / (a.length * 255),
    inkIou: union === 0 ? 1 : inter / union,
    diffImage: { width, height, data: image }
  }
}

function fit(page, width, height) {
  if (page.width === width && page.height === height) return page.gray
  const out = new Uint8Array(width * height)
  for (let y = 0; y < height; y++) {
    const sy = Math.min(page.height - 1, Math.floor((y * page.height) / height))
    for (let x = 0; x < width; x++) {
      const sx = Math.min(page.width - 1, Math.floor((x * page.width) / width))
      out[y * width + x] = page.gray[sy * page.width + sx]
    }
  }
  return out
}

function sameSize(a, b) {
  return Math.abs(a.width - b.width) < 1 && Math.abs(a.height - b.height) < 1
}

function mean(values) {
  if (values.length === 0) return 0
  return values.reduce((s, v) => s + v, 0) / values.length
}

module.exports = { comparePdfs }
