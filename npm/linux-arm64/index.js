const path = require('bare-path')

const addon = require.addon.resolve()

exports.binding = require.addon()
exports.pdfiumPath = path.join(path.dirname(addon), path.basename(addon, '.bare'), 'libpdfium.so')
