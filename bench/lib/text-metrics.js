// Words are compared case-insensitively after Unicode normalisation; the
// hyphen variants and PDFium's U+FFFE placeholder for a non-breaking hyphen all
// count as a plain hyphen so both engines are judged on the same footing.
function tokens(text) {
  return text
    .normalize('NFKC')
    .replace(/[￾�‐‑­]/g, '-')
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean)
}

function counts(list) {
  const map = new Map()
  for (const t of list) map.set(t, (map.get(t) || 0) + 1)
  return map
}

function overlap(a, b) {
  const cb = counts(b)
  let hit = 0
  for (const [word, n] of counts(a)) hit += Math.min(n, cb.get(word) || 0)
  return hit
}

function lcs(a, b) {
  if (!a.length || !b.length) return 0
  const ids = new Map()
  const enc = (list) =>
    list.map((w) => {
      if (!ids.has(w)) ids.set(w, ids.size + 1)
      return ids.get(w)
    })
  const x = enc(a)
  const y = enc(b)
  let prev = new Uint32Array(y.length + 1)
  let cur = new Uint32Array(y.length + 1)
  for (let i = 1; i <= x.length; i++) {
    for (let j = 1; j <= y.length; j++) {
      cur[j] = x[i - 1] === y[j - 1] ? prev[j - 1] + 1 : Math.max(prev[j], cur[j - 1])
    }
    ;[prev, cur] = [cur, prev]
  }
  return prev[y.length]
}

// recall: share of source words found; precision: share of extracted words
// that exist in the source; order: longest common subsequence over the source,
// which drops when text comes out in a different reading order.
function compare(source, extracted) {
  const s = tokens(source)
  const e = tokens(extracted)
  const hit = overlap(s, e)
  return {
    sourceWords: s.length,
    words: e.length,
    recall: s.length ? hit / s.length : 1,
    precision: e.length ? hit / e.length : 1,
    order: s.length ? lcs(s, e) / s.length : 1
  }
}

function agreement(a, b) {
  const x = tokens(a)
  const y = tokens(b)
  const total = x.length + y.length
  return total ? (2 * lcs(x, y)) / total : 1
}

function unmappable(text) {
  return (text.match(/[￾�]/g) || []).length
}

module.exports = { tokens, compare, agreement, unmappable }
