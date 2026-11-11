import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'

import AdmZip from 'adm-zip'

import { readBaseline, verifySource, verifyVsCatalog } from './baseline.mjs'
import { prepareArtifact } from './download-artifact.mjs'

const baseline = await readBaseline()
await verifySource(baseline)
const toolchain = baseline.buildToolchain
const cache = path.join(homedir(), '.cache', 'chaindesk')
const artifacts = path.join(cache, 'artifacts')
await mkdir(artifacts, { recursive: true })
const catalogPath = await prepareArtifact(artifacts, {
  ...toolchain.acceptedCatalog,
  name: toolchain.acceptedCatalog.fileName,
})
const signature = verifyVsCatalog(catalogPath)
assert.equal(signature.signatureResult, 'Success')
const catalog = JSON.parse(await readFile(catalogPath, 'utf8'))
const root = path.join(cache, `msvc-${toolchain.visualStudioVersion}`)
await mkdir(root, { recursive: true })
const packages = []
for (const pinned of toolchain.msvcPayloads) {
  const matches = catalog.packages.filter(
    (item) =>
      item.id === pinned.id &&
      item.version === pinned.version &&
      (item.language ?? 'en-US') === 'en-US',
  )
  assert.equal(matches.length, 1, `Ambiguous MSVC package: ${pinned.id}`)
  const payload = matches[0].payloads[0]
  assert.equal(payload.sha256, pinned.sha256)
  const archivePath = await prepareArtifact(root, {
    name: payload.fileName,
    url: payload.url,
    sha256: pinned.sha256,
  })
  packages.push({ ...pinned, archivePath })
}
const toolsetRoot = path.join(root, 'toolset')
const files = []
for (const pkg of packages) {
  for (const entry of new AdmZip(pkg.archivePath).getEntries()) {
    const prefix = `Contents/VC/Tools/MSVC/${toolchain.msvcDirectoryVersion}/`
    if (entry.isDirectory || !entry.entryName.startsWith(prefix)) continue
    const relativePath = entry.entryName.slice(prefix.length)
    const destination = path.resolve(toolsetRoot, relativePath)
    assert.ok(
      destination.startsWith(`${toolsetRoot}${path.sep}`),
      'MSVC archive path escapes the toolset directory',
    )
    const bytes = entry.getData()
    let existing
    try {
      existing = await readFile(destination)
    } catch (error) {
      if (error.code !== 'ENOENT') throw error
    }
    if (!existing?.equals(bytes)) {
      await mkdir(path.dirname(destination), { recursive: true })
      await writeFile(destination, bytes)
    }
    files.push({
      path: relativePath,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    })
  }
}
const manifest = {
  sourceCatalogSha256: toolchain.acceptedCatalog.sha256,
  toolsetRoot,
  packages,
  files,
}
await writeFile(
  path.join(root, 'toolset-manifest.json'),
  `${JSON.stringify(manifest, null, 2)}\n`,
)
console.log(
  JSON.stringify({
    toolsetRoot,
    compilerPackageVersion: toolchain.compilerTargetVersion,
    packagesVerified: packages.length,
    filesPrepared: files.length,
  }),
)
