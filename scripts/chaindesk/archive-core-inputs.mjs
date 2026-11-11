import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'

import {
  projectRoot,
  readBaseline,
  verifyFile,
  verifySource,
} from './baseline.mjs'
import { prepareArtifact } from './download-artifact.mjs'

const baseline = await readBaseline()
await verifySource(baseline)
const core = baseline.artifacts.core
const inputs = baseline.artifacts.coreBuildInputs
const cache = path.join(homedir(), '.cache', 'chaindesk', 'core-build-inputs')
await mkdir(cache, { recursive: true })
const goArchive = await prepareArtifact(cache, inputs.customGo)
const tar = path.join(process.env.SystemRoot, 'System32', 'tar.exe')
const goVersion = execFileSync(
  tar,
  ['-xOf', goArchive, inputs.customGo.versionFile],
  { windowsHide: true, encoding: 'utf8' },
).trim()
assert.equal(goVersion.split('\n')[0], core.goVersion)
assert.ok(
  execFileSync(tar, ['-xOf', goArchive, './go/src/cmd/compile/main.go'], {
    windowsHide: true,
  }).length > 0,
)

const corePath = path.join(
  projectRoot,
  'src-tauri',
  'sidecar',
  `verge-mihomo-${baseline.toolchain.rustTarget}.exe`,
)
await verifyFile(corePath, core.files[0].sha256)
const binary = await readFile(corePath)
const ca = inputs.embeddedCa
const pe = binary.readUInt32LE(0x3c)
const sections = pe + 24 + binary.readUInt16LE(pe + 20)
const imageBase = binary.readBigUInt64LE(pe + 48)
let mappedAddress
for (let index = 0; index < binary.readUInt16LE(pe + 6); index++) {
  const section = sections + index * 40
  const rawSize = binary.readUInt32LE(section + 16)
  const rawOffset = binary.readUInt32LE(section + 20)
  if (rawOffset <= ca.fileOffset && ca.fileOffset < rawOffset + rawSize) {
    mappedAddress =
      imageBase +
      BigInt(binary.readUInt32LE(section + 12) + ca.fileOffset - rawOffset)
  }
}
assert.equal(binary.readBigUInt64LE(ca.sliceHeaderFileOffset), mappedAddress)
assert.equal(
  binary.readBigUInt64LE(ca.sliceHeaderFileOffset + 8),
  BigInt(ca.size),
)
assert.equal(
  binary.readBigUInt64LE(ca.sliceHeaderFileOffset + 16),
  BigInt(ca.size),
)
const certificates = binary.subarray(ca.fileOffset, ca.fileOffset + ca.size)
assert.equal(createHash('sha256').update(certificates).digest('hex'), ca.sha256)
assert.equal(
  certificates.toString('ascii').match(/-----BEGIN CERTIFICATE-----/g)?.length,
  ca.certificateCount,
)
await writeFile(path.join(cache, ca.name), certificates)

const source = path.join(cache, 'mihomo-source')
try {
  await stat(source)
} catch (error) {
  if (error.code !== 'ENOENT') throw error
  execFileSync(
    'git',
    [
      'clone',
      '--filter=blob:none',
      '--depth',
      '1',
      '--branch',
      core.version,
      '--single-branch',
      `${core.repository}.git`,
      source,
    ],
    {
      windowsHide: true,
      stdio: 'pipe',
      env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
    },
  )
}
assert.equal(
  execFileSync('git', ['-C', source, 'rev-parse', 'HEAD'], {
    encoding: 'utf8',
  }).trim(),
  core.sourceCommit,
)
assert.equal(
  execFileSync('git', ['-C', source, 'status', '--porcelain'], {
    encoding: 'utf8',
  }).trim(),
  '',
)
const sourceArchive = path.join(cache, `mihomo-${core.sourceCommit}.tar.gz`)
execFileSync(
  'git',
  [
    '-C',
    source,
    'archive',
    '--format=tar.gz',
    `--output=${sourceArchive}`,
    'HEAD',
  ],
  { windowsHide: true },
)
const files = []
for (const file of [goArchive, path.join(cache, ca.name), sourceArchive]) {
  const digest = createHash('sha256')
  for await (const chunk of createReadStream(file)) digest.update(chunk)
  files.push({
    name: path.basename(file),
    size: (await stat(file)).size,
    sha256: digest.digest('hex'),
  })
}
const manifest = {
  sourceCommit: core.sourceCommit,
  coreSha256: core.files[0].sha256,
  goVersion,
  customGoAssetId: inputs.customGo.assetId,
  caRecovery: {
    ...ca,
    proof:
      'PE section mapping and Go byte-slice pointer, length and capacity match the pinned binary',
  },
  parameters: inputs.parameters,
  sourceOverlay:
    'Copy ca-certificates.crt over component/ca/ca-certificates.crt before building; the upstream Git file is empty.',
  files,
  bitReproducibilityProven: false,
}
await writeFile(
  path.join(cache, 'build-inputs.json'),
  `${JSON.stringify(manifest, null, 2)}\n`,
)
console.log(
  JSON.stringify({
    cache,
    sourceCommit: core.sourceCommit,
    goVersion,
    certificateCount: ca.certificateCount,
    filesArchived: files.length,
    bitReproducibilityProven: false,
  }),
)
