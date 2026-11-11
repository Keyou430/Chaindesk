import { copyFile, mkdir, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { parseArgs } from 'node:util'

import AdmZip from 'adm-zip'

import { log_success } from '../utils.mjs'

import {
  projectRoot,
  readBaseline,
  verifyFile,
  verifySource,
} from './baseline.mjs'
import { prepareArtifact } from './download-artifact.mjs'

const { values } = parseArgs({
  options: { cache: { type: 'string' }, webview: { type: 'boolean' } },
})
const cache = path.resolve(
  values.cache ?? path.join(homedir(), '.cache', 'chaindesk', 'artifacts'),
)
const baseline = await readBaseline()
await verifySource(baseline)
await mkdir(cache, { recursive: true })

const core = baseline.artifacts.core
const coreArchive = await prepareArtifact(cache, core)
const coreFile = core.files[0]
const sidecarDirectory = path.join(projectRoot, 'src-tauri', 'sidecar')
await mkdir(sidecarDirectory, { recursive: true })
const sidecar = path.join(
  sidecarDirectory,
  `verge-mihomo-${baseline.toolchain.rustTarget}.exe`,
)
await writeFile(
  sidecar,
  new AdmZip(coreArchive).getEntry(coreFile.name).getData(),
)
await verifyFile(sidecar, coreFile.sha256)

const resourceDirectory = path.join(projectRoot, 'src-tauri', 'resources')
await mkdir(resourceDirectory, { recursive: true })
for (const artifact of baseline.artifacts.rules.assets) {
  const source = await prepareArtifact(cache, artifact)
  const destination = path.join(resourceDirectory, artifact.name)
  await copyFile(source, destination)
  await verifyFile(destination, artifact.sha256)
}
if (values.webview) await prepareArtifact(cache, baseline.artifacts.webview2)
log_success('Pinned stable core and rule resources verified and prepared.')
