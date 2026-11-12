import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import {
  copyFile,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  writeFile,
} from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { parseArgs } from 'node:util'

import AdmZip from 'adm-zip'

import {
  projectRoot,
  readBaseline,
  verifyFile,
  verifySource,
} from './baseline.mjs'

const { values } = parseArgs({
  options: { full: { type: 'boolean', default: false } },
})
const baseline = await readBaseline()
await verifySource(baseline)
const configuration = JSON.parse(
  await readFile(path.join(projectRoot, 'src-tauri/tauri.conf.json'), 'utf8'),
)
assert.equal(configuration.identifier, 'local.chaindesk.portable')
const cache = path.join(homedir(), '.cache/chaindesk')
const releases = path.join(projectRoot, '.chaindesk/releases')
await mkdir(releases, { recursive: true })
const output = await mkdtemp(path.join(releases, 'm1-dev-'))
const product = path.join(output, 'ChainDesk')
await mkdir(path.join(product, 'core'), { recursive: true })
await mkdir(path.join(product, 'resources/rules'), { recursive: true })
await copyFile(
  path.join(cache, 'target/debug/clash-verge.exe'),
  path.join(product, 'ChainDesk.exe'),
)
const core = path.join(
  projectRoot,
  'src-tauri/sidecar',
  `verge-mihomo-${baseline.toolchain.rustTarget}.exe`,
)
await verifyFile(core, baseline.artifacts.core.files[0].sha256)
await copyFile(core, path.join(product, 'core/mihomo.exe'))
for (const asset of baseline.artifacts.rules.assets) {
  const source = path.join(projectRoot, 'src-tauri/resources', asset.name)
  await verifyFile(source, asset.sha256)
  await copyFile(source, path.join(product, 'resources/rules', asset.name))
}
await copyFile(path.join(projectRoot, 'LICENSE'), path.join(product, 'LICENSE'))
if (values.full) {
  const runtime = path.join(
    cache,
    'virtualization/payload/runtime',
    `Microsoft.WebView2.FixedVersionRuntime.${baseline.artifacts.webview2.version}.x64`,
  )
  await verifyFile(
    path.join(runtime, 'msedgewebview2.exe'),
    baseline.artifacts.webview2.mainExecutable.sha256,
  )
  await cp(runtime, path.join(product, 'runtime'), {
    recursive: true,
    errorOnExist: true,
    force: false,
  })
}
async function hash(file) {
  const digest = createHash('sha256')
  for await (const chunk of createReadStream(file)) digest.update(chunk)
  return digest.digest('hex')
}
const files = []
for (const entry of await readdir(product, {
  withFileTypes: true,
  recursive: true,
})) {
  if (!entry.isFile()) continue
  const source = path.join(entry.parentPath, entry.name)
  files.push({
    path: path.relative(output, source).split(path.sep).join('/'),
    sha256: await hash(source),
  })
}
files.sort((left, right) => left.path.localeCompare(right.path))
assert.ok(files.every((file) => !file.path.startsWith('ChainDesk/data/')))
const archivePath = path.join(
  output,
  `ChainDesk-M1-dev-${values.full ? 'full' : 'standard'}.zip`,
)
const archive = new AdmZip()
archive.addLocalFolder(product, 'ChainDesk')
archive.writeZip(archivePath)
const sourceCommit = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: projectRoot,
  encoding: 'utf8',
}).trim()
const sourcePaths = execFileSync(
  'git',
  [
    'ls-files',
    '-z',
    '--cached',
    '--others',
    '--exclude-standard',
    '--',
    'src',
    'src-tauri',
    'crates',
    'Cargo.toml',
    'Cargo.lock',
    '.cargo',
    'rust-toolchain.toml',
    'vite.config.mts',
    'package.json',
    'pnpm-lock.yaml',
  ],
  { cwd: projectRoot, encoding: 'utf8' },
)
  .split('\0')
  .filter(Boolean)
const sourceFiles = []
for (const file of sourcePaths)
  sourceFiles.push({
    path: file,
    sha256: await hash(path.join(projectRoot, file)),
  })
const manifest = {
  milestone: 'M1',
  developmentOnly: true,
  productDirectory: 'ChainDesk',
  fullRuntime: values.full,
  minimumWebview2: baseline.release.minimumInstalledWebview2,
  sourceCommit,
  sourceFingerprint: createHash('sha256')
    .update(JSON.stringify(sourceFiles))
    .digest('hex'),
  sourceFiles,
  files,
  archivePath,
  archiveSha256: await hash(archivePath),
  assembledAt: new Date().toISOString(),
}
const manifestPath = path.join(output, 'payload-manifest.json')
await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, {
  flag: 'wx',
})
await writeFile(
  path.join(projectRoot, '.chaindesk/m1-package.json'),
  `${JSON.stringify({ manifestPath, archivePath, outputDirectory: output }, null, 2)}\n`,
)
console.log(
  JSON.stringify({
    manifestPath,
    archivePath,
    sourceFingerprint: manifest.sourceFingerprint,
    archiveSha256: manifest.archiveSha256,
    files: files.length,
  }),
)
