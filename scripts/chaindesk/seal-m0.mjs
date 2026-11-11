import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'

import AdmZip from 'adm-zip'

import {
  projectRoot,
  readBaseline,
  verifyFile,
  verifySource,
  verifyVsCatalog,
} from './baseline.mjs'

const baseline = await readBaseline()
await verifySource(baseline)
const logs = path.join(projectRoot, '.chaindesk', 'logs')
const evidence = JSON.parse(
  await readFile(path.join(logs, 'clean-vm-evidence.json'), 'utf8'),
)
const result = evidence.guestResult
assert.equal(result.passed, true, 'Clean VM acceptance has not passed')
assert.equal(evidence.image.sha256, baseline.cleanVmEnvironment.image.sha256)
assert.equal(evidence.accelerator, 'whpx')
assert.equal(result.sourceCommit, baseline.source.commit)
assert.equal(result.windows.architecture, 'AMD64')
assert.ok(result.windows.build >= baseline.platform.minimumWindows11Build)
assert.equal(result.servicesBefore, 0)
assert.equal(result.servicesAfter, 0)
assert.equal(result.runtimeState.mode, 'Sidecar')
assert.equal(result.runtimeState.isAdmin, false)
assert.equal(result.runtimeState.serviceUsable, false)
assert.equal(result.runtimeState.tunCapable, false)
assert.equal(result.normalExitCode, 0)
for (const flag of [
  'standardUser',
  'developerToolsAbsent',
  'initialApplicationDataAbsent',
  'fixedWebviewUsed',
  'proxyRequestPassed',
  'ownedCoreStopped',
  'proxySettingsUnchanged',
  'applicationDataCreated',
  'updaterDisabledByFixture',
]) {
  assert.equal(result[flag], true, `Missing VM acceptance result: ${flag}`)
}
const cache = path.join(homedir(), '.cache', 'chaindesk')
const application = result.files.find((file) => file.path === 'clash-verge.exe')
assert.ok(application, 'The accepted application hash is missing')
await verifyFile(
  path.join(cache, 'target', 'debug', 'clash-verge.exe'),
  application.sha256,
)
assert.equal(
  result.files.find((file) => file.path === 'verge-mihomo.exe')?.sha256,
  baseline.artifacts.core.files[0].sha256,
)
assert.equal(
  result.files.find((file) => file.path === 'runtime/msedgewebview2.exe')
    ?.sha256,
  baseline.artifacts.webview2.mainExecutable.sha256,
)
await verifyFile(path.join(logs, 'clean-vm.png'), result.screenshotSha256)
await verifyFile(
  path.join(projectRoot, 'scripts', 'chaindesk', 'accept-clean-vm.ps1'),
  result.harnessSha256,
)
const signature = verifyVsCatalog(
  path.join(
    cache,
    'artifacts',
    baseline.buildToolchain.acceptedCatalog.fileName,
  ),
)
assert.equal(signature.signatureResult, 'Success')
assert.equal(signature.onlineCertificateChain, 'Valid')
const compiler = JSON.parse(
  await readFile(path.join(logs, 'pinned-msvc-compiler.json'), 'utf8'),
)
const buildEnvironment = JSON.parse(
  await readFile(path.join(logs, 'build-environment.json'), 'utf8'),
)
assert.equal(buildEnvironment.pinnedMsvc, true)
assert.equal(buildEnvironment.compiler.sha256, compiler.sha256)
assert.equal(
  buildEnvironment.compiler.version,
  `${baseline.buildToolchain.compilerTargetVersion}.0`,
)
await verifyFile(buildEnvironment.linker.path, buildEnvironment.linker.sha256)
const buildLog = await readFile(path.join(logs, 'baseline-build.log'), 'utf8')
assert.ok(
  buildLog
    .split('\n')
    .some(
      (line) =>
        line.includes('--crate-name clash_verge ') &&
        line.includes('linker=') &&
        line.includes(buildEnvironment.linker.path),
    ),
  'The accepted linker is missing from the actual rustc invocation',
)
const coreInputsRoot = path.join(cache, 'core-build-inputs')
const coreInputs = JSON.parse(
  await readFile(path.join(coreInputsRoot, 'build-inputs.json'), 'utf8'),
)
assert.equal(coreInputs.sourceCommit, baseline.artifacts.core.sourceCommit)
assert.equal(coreInputs.coreSha256, baseline.artifacts.core.files[0].sha256)
assert.equal(
  coreInputs.files.find(
    (file) => file.name === baseline.artifacts.coreBuildInputs.customGo.name,
  )?.sha256,
  baseline.artifacts.coreBuildInputs.customGo.sha256,
)
assert.equal(
  coreInputs.files.find((file) => file.name === 'ca-certificates.crt')?.sha256,
  baseline.artifacts.coreBuildInputs.embeddedCa.sha256,
)
for (const file of coreInputs.files)
  await verifyFile(path.join(coreInputsRoot, file.name), file.sha256)
assert.equal(
  compiler.productVersion,
  `${baseline.buildToolchain.compilerTargetVersion}.0`,
)
assert.equal(compiler.signature, 'Valid')
await verifyFile(
  path.join(
    cache,
    `msvc-${baseline.buildToolchain.visualStudioVersion}`,
    'toolset',
    'bin',
    'Hostx64',
    'x64',
    'cl.exe',
  ),
  compiler.sha256,
)
const smoke = await readFile(path.join(logs, 'chain-smoke.log'), 'utf8')
assert.match(smoke, /pass 5\b/)
assert.match(smoke, /fail 0\b/)
assert.equal(baseline.implementation.status, 'm0AcceptancePassed')
assert.equal(baseline.validation.releaseReady, false)
const applicationChanges = execFileSync(
  'git',
  [
    'diff',
    '--name-only',
    baseline.source.commit,
    'HEAD',
    '--',
    'src',
    'src-tauri',
    'crates',
    'Cargo.toml',
    'Cargo.lock',
    '.cargo',
    'rust-toolchain.toml',
    'vite.config.*',
  ],
  { cwd: projectRoot, encoding: 'utf8' },
)
assert.equal(
  applicationChanges.trim(),
  '',
  'M0 accepts only unchanged upstream application sources',
)
const status = execFileSync('git', ['status', '--porcelain'], {
  cwd: projectRoot,
  encoding: 'utf8',
})
assert.equal(
  status.trim(),
  '',
  'Commit the reviewed M0 source and acceptance record before sealing',
)
const commit = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: projectRoot,
  encoding: 'utf8',
}).trim()
const output = path.join(
  projectRoot,
  '.chaindesk',
  'releases',
  `m0-${baseline.confirmedOn}-${commit.slice(0, 12)}`,
)
await mkdir(path.dirname(output), { recursive: true })
await mkdir(output, { recursive: false })
const archivePath = path.join(output, 'ChainDesk-M0-source-and-evidence.zip')
execFileSync(
  'git',
  [
    'archive',
    '--format=zip',
    '--prefix=source/',
    '--output',
    archivePath,
    commit,
  ],
  { cwd: projectRoot, windowsHide: true },
)
const archive = new AdmZip(archivePath)
for (const file of [
  'clean-vm-evidence.json',
  'clean-vm-result.json',
  'clean-vm.png',
  'clean-vm-run.log',
  'clean-vm-qemu.log',
  'ps-json-log-regression.json',
  'vs-catalog-verification.json',
  'vs-native-tamper-verification.json',
  'pinned-msvc-compiler.json',
  'build-environment.json',
  'application-dependencies.log',
  'baseline-build.log',
  'chain-smoke.log',
]) {
  archive.addLocalFile(path.join(logs, file), 'evidence')
}
archive.addLocalFile(
  path.join(coreInputsRoot, 'build-inputs.json'),
  'build-inputs/core',
)
for (const file of coreInputs.files)
  archive.addLocalFile(
    path.join(coreInputsRoot, file.name),
    'build-inputs/core',
  )
archive.writeZip(archivePath)
const digest = createHash('sha256')
for await (const chunk of createReadStream(archivePath)) digest.update(chunk)
const seal = {
  milestone: 'M0',
  status: 'sealed',
  sealedAt: new Date().toISOString(),
  repositoryCommit: commit,
  upstreamCommit: baseline.source.commit,
  applicationSha256: application.sha256,
  visualStudioCatalogSha256: signature.sha256,
  vmScreenshotSha256: result.screenshotSha256,
  sourceAndEvidenceArchive: path.basename(archivePath),
  sourceAndEvidenceSha256: digest.digest('hex'),
  productReleaseReady: false,
}
await writeFile(
  path.join(output, 'M0-SEAL.json'),
  `${JSON.stringify(seal, null, 2)}\n`,
  { flag: 'wx' },
)
console.log(JSON.stringify({ outputDirectory: output, ...seal }))
