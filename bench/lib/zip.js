const zlib = require('bare-zlib')

function entries(buffer) {
  let eocd = -1
  for (let i = buffer.length - 22; i >= Math.max(0, buffer.length - 66000); i--) {
    if (buffer.readUInt32LE(i) === 0x06054b50) {
      eocd = i
      break
    }
  }
  if (eocd < 0) throw new Error('not a zip archive')
  const count = buffer.readUInt16LE(eocd + 10)
  let offset = buffer.readUInt32LE(eocd + 16)
  const map = new Map()
  for (let i = 0; i < count; i++) {
    if (buffer.readUInt32LE(offset) !== 0x02014b50) break
    const method = buffer.readUInt16LE(offset + 10)
    const compressed = buffer.readUInt32LE(offset + 20)
    const nameLength = buffer.readUInt16LE(offset + 28)
    const extraLength = buffer.readUInt16LE(offset + 30)
    const commentLength = buffer.readUInt16LE(offset + 32)
    const local = buffer.readUInt32LE(offset + 42)
    const name = buffer.toString('utf8', offset + 46, offset + 46 + nameLength)
    map.set(name, { method, compressed, local })
    offset += 46 + nameLength + extraLength + commentLength
  }
  return map
}

function read(buffer, entry) {
  const nameLength = buffer.readUInt16LE(entry.local + 26)
  const extraLength = buffer.readUInt16LE(entry.local + 28)
  const start = entry.local + 30 + nameLength + extraLength
  const data = buffer.subarray(start, start + entry.compressed)
  if (entry.method === 0) return Buffer.from(data)
  if (entry.method === 8) return zlib.inflateRawSync(data)
  throw new Error(`unsupported zip method ${entry.method}`)
}

function open(buffer) {
  const map = entries(buffer)
  return {
    names: () => [...map.keys()],
    has: (name) => map.has(name),
    text: (name) => (map.has(name) ? read(buffer, map.get(name)).toString('utf8') : null)
  }
}

module.exports = { open }
