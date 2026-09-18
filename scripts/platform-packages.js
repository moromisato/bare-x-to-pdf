const fs = require('bare-fs')
const path = require('bare-path')

const root = path.join(__dirname, '..')
const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))

const TARGETS = [
  {
    target: 'android-arm',
    platform: 'android',
    arch: 'arm',
    label: 'Android ARM 32-bit',
    lib: 'libpdfium.so'
  },
  {
    target: 'android-arm64',
    platform: 'android',
    arch: 'arm64',
    label: 'Android ARM 64-bit',
    lib: 'libpdfium.so'
  },
  {
    target: 'android-x64',
    platform: 'android',
    arch: 'x64',
    label: 'Android x86 64-bit',
    lib: 'libpdfium.so'
  },
  {
    target: 'darwin-arm64',
    platform: 'darwin',
    arch: 'arm64',
    label: 'macOS ARM 64-bit',
    lib: 'libpdfium.dylib'
  },
  {
    target: 'darwin-x64',
    platform: 'darwin',
    arch: 'x64',
    label: 'macOS x86 64-bit',
    lib: 'libpdfium.dylib'
  },
  {
    target: 'ios-arm64',
    platform: 'ios',
    arch: 'arm64',
    label: 'iOS ARM 64-bit',
    lib: 'libpdfium.dylib'
  },
  {
    target: 'ios-arm64-simulator',
    platform: 'ios',
    arch: 'arm64',
    simulator: true,
    label: 'iOS ARM simulator 64-bit',
    lib: 'libpdfium.dylib'
  },
  {
    target: 'ios-x64-simulator',
    platform: 'ios',
    arch: 'x64',
    simulator: true,
    label: 'iOS x86 simulator 64-bit',
    lib: 'libpdfium.dylib'
  },
  {
    target: 'linux-arm64',
    platform: 'linux',
    arch: 'arm64',
    label: 'Linux ARM 64-bit',
    lib: 'libpdfium.so'
  },
  {
    target: 'linux-x64',
    platform: 'linux',
    arch: 'x64',
    label: 'Linux x86 64-bit',
    lib: 'libpdfium.so'
  },
  {
    target: 'win32-x64',
    platform: 'win32',
    arch: 'x64',
    label: 'Windows x86 64-bit',
    lib: 'pdfium.dll'
  }
]

const imports = {}
const optional = {}

for (const t of TARGETS) {
  const name = `${pkg.name}-${t.target}`
  const dir = path.join(root, 'npm', t.target)
  fs.mkdirSync(dir, { recursive: true })

  const manifest = {
    name,
    version: pkg.version,
    description: `The ${t.label} prebuild of ${pkg.name}`,
    exports: {
      './package': './package.json',
      '.': './index.js'
    },
    files: ['index.js', 'prebuilds'],
    addon: true,
    os: [t.platform],
    cpu: [t.arch],
    license: pkg.license,
    repository: pkg.repository,
    bugs: pkg.bugs,
    homepage: pkg.homepage,
    publishConfig: pkg.publishConfig,
    dependencies: { 'bare-path': pkg.dependencies['bare-path'] }
  }
  fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify(manifest, null, 2) + '\n')

  fs.writeFileSync(
    path.join(dir, 'index.js'),
    [
      "const path = require('bare-path')",
      '',
      'const addon = require.addon.resolve()',
      '',
      'exports.binding = require.addon()',
      'exports.pdfiumPath = path.join(',
      '  path.dirname(addon),',
      "  path.basename(addon, '.bare'),",
      `  '${t.lib}'`,
      ')',
      ''
    ].join('\n')
  )

  const byPlatform = (imports[t.platform] ||= {})
  if (t.platform === 'ios') {
    const byArch = (byPlatform[t.arch] ||= {})
    byArch[t.simulator ? 'simulator' : 'default'] = name
  } else {
    byPlatform[t.arch] = name
  }
  optional[name] = pkg.version
}

pkg.imports = { '#binding': imports }
pkg.optionalDependencies = optional
fs.writeFileSync(path.join(root, 'package.json'), JSON.stringify(pkg, null, 2) + '\n')

console.log(`wrote ${TARGETS.length} platform packages at version ${pkg.version}`)
