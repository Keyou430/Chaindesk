import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { createReadStream, createWriteStream } from 'node:fs'
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  stat,
  writeFile,
} from 'node:fs/promises'
import http from 'node:http'
import net from 'node:net'
import { homedir } from 'node:os'
import path from 'node:path'
import readline from 'node:readline'
import { setTimeout as delay } from 'node:timers/promises'
import { parseArgs } from 'node:util'

import AdmZip from 'adm-zip'

import {
  projectRoot,
  readBaseline,
  verifyFile,
  verifySource,
} from './baseline.mjs'
import { createVmProbeFixture } from './vm-probe-fixture.mjs'
import { createVmSubscriptionFixture } from './vm-subscription-fixture.mjs'
import { prepareVmTlsFixture } from './vm-tls-fixture.mjs'

const { values } = parseArgs({
  options: {
    image: { type: 'string' },
    'qemu-root': { type: 'string' },
    milestone: { type: 'string', default: 'm0' },
    'base-evidence': { type: 'string' },
    'original-app': { type: 'string' },
  },
})
assert.ok(
  ['m0', 'm1', 'm2', 'm3', 'm4'].includes(values.milestone),
  'Unsupported VM milestone',
)
const portable = values.milestone !== 'm0'
const chainScenario = ['m2', 'm3', 'm4'].includes(values.milestone)
const recoveryScenario = ['m3', 'm4'].includes(values.milestone)
const portableHarnesses = [
  'accept-portable-vm.ps1',
  'vm-debugger.ps1',
  ...(chainScenario ? ['chain-vm-scenario.ps1'] : []),
  ...(recoveryScenario ? ['recovery-vm-scenario.ps1'] : []),
]
const reuseGuest = portable && values['base-evidence'] !== undefined
const logPrefix = chainScenario
  ? `${values.milestone}-vm`
  : portable
    ? 'portable-vm'
    : 'clean-vm'
const baseline = await readBaseline()
await verifySource(baseline)
const environment = baseline.cleanVmEnvironment
assert.ok(
  environment,
  'The VM environment must be pinned in engineeringBaseline.json',
)
const cache = path.join(homedir(), '.cache', 'chaindesk', 'virtualization')
const image = path.resolve(
  values.image ?? path.join(cache, environment.image.fileName),
)
const qemuRoot = path.resolve(values['qemu-root'] ?? path.join(cache, 'qemu'))
await verifyFile(image, environment.image.sha256)
assert.equal((await stat(image)).size, environment.image.size)
for (const file of environment.qemu.files) {
  await verifyFile(path.join(qemuRoot, file.path), file.sha256)
}
await mkdir(cache, { recursive: true })
const runDirectory = await mkdtemp(path.join(cache, `${values.milestone}-`))
const tlsFixtureDirectory = path.join(runDirectory, 'tls-fixture')
if (chainScenario) await prepareVmTlsFixture(tlsFixtureDirectory)
const answerDirectory = path.join(runDirectory, 'answer')
await mkdir(path.join(answerDirectory, 'M0'), { recursive: true })
const logDirectory = path.join(projectRoot, '.chaindesk', 'logs')
await mkdir(logDirectory, { recursive: true })
const payloadPath = path.join(runDirectory, 'payload.zip')
let manifest
let backingRun
if (portable) {
  const pointer = JSON.parse(
    await readFile(
      path.join(projectRoot, `.chaindesk/${values.milestone}-package.json`),
      'utf8',
    ),
  )
  manifest = JSON.parse(await readFile(pointer.manifestPath, 'utf8'))
  assert.equal(manifest.milestone, values.milestone.toUpperCase())
  manifest.originalApplicationSha256 =
    baseline.cleanVmEnvironment.acceptedApplicationSha256
  const original = path.resolve(
    projectRoot,
    values['original-app'] ??
      '.chaindesk/m0-closure/accepted-baseline/clash-verge.exe',
  )
  await verifyFile(original, manifest.originalApplicationSha256)
  await copyFile(original, path.join(runDirectory, 'original.exe'))
  for (const file of manifest.sourceFiles) {
    await verifyFile(path.join(projectRoot, file.path), file.sha256)
  }
  await verifyFile(pointer.archivePath, manifest.archiveSha256)
  await copyFile(pointer.archivePath, payloadPath)
  await verifyFile(payloadPath, manifest.archiveSha256)
  if (reuseGuest) {
    const accepted = JSON.parse(
      await readFile(
        path.resolve(
          projectRoot,
          values['base-evidence'] ?? '.chaindesk/logs/clean-vm-evidence.json',
        ),
        'utf8',
      ),
    )
    backingRun = accepted.runDirectory
    assert.equal(
      accepted.guestResult.guestPoweredOff,
      true,
      'Only a cleanly powered-off guest may be reused',
    )
    assert.equal(accepted.image.sha256, environment.image.sha256)
  }
} else {
  const runtimeDirectory = `Microsoft.WebView2.FixedVersionRuntime.${baseline.artifacts.webview2.version}.x64`
  const runtimeRoot = path.join(cache, 'payload', 'runtime', runtimeDirectory)
  await verifyFile(
    path.join(runtimeRoot, 'msedgewebview2.exe'),
    baseline.artifacts.webview2.mainExecutable.sha256,
  )
  const applicationPath = path.join(
    cache,
    '..',
    'target',
    'debug',
    'clash-verge.exe',
  )
  const corePath = path.join(
    projectRoot,
    'src-tauri',
    'sidecar',
    `verge-mihomo-${baseline.toolchain.rustTarget}.exe`,
  )
  await verifyFile(corePath, baseline.artifacts.core.files[0].sha256)
  const files = []
  {
    const archive = new AdmZip()
    for (const [source, target, expectedHash] of [
      [applicationPath, 'clash-verge.exe'],
      [corePath, 'verge-mihomo.exe', baseline.artifacts.core.files[0].sha256],
      ...baseline.artifacts.rules.assets.map((asset) => [
        path.join(projectRoot, 'src-tauri', 'resources', asset.name),
        `resources/${asset.name}`,
        asset.sha256,
      ]),
    ]) {
      if (expectedHash) await verifyFile(source, expectedHash)
      const bytes = await readFile(source)
      files.push({
        path: target,
        sha256: createHash('sha256').update(bytes).digest('hex'),
      })
      archive.addFile(target, bytes)
    }
    archive.addLocalFolder(runtimeRoot, 'runtime')
    files.push({
      path: 'runtime/msedgewebview2.exe',
      sha256: baseline.artifacts.webview2.mainExecutable.sha256,
    })
    archive.writeZip(payloadPath)
  }
  const digest = createHash('sha256')
  for await (const chunk of createReadStream(payloadPath)) digest.update(chunk)
  manifest = {
    sourceCommit: baseline.source.commit,
    archiveSha256: digest.digest('hex'),
    runtimeDirectory: 'runtime',
    files,
  }
}
await writeFile(
  path.join(runDirectory, 'manifest.json'),
  `${JSON.stringify(manifest, null, 2)}\n`,
)
const nonce = randomBytes(24).toString('hex')
const subscriptionFixture = chainScenario
  ? await createVmSubscriptionFixture(tlsFixtureDirectory, nonce)
  : undefined
const probeFixture = recoveryScenario
  ? await createVmProbeFixture(tlsFixtureDirectory)
  : undefined
let complete
const completion = new Promise((resolve) => {
  complete = resolve
})
const server = http.createServer(async (request, response) => {
  try {
    const resource = request.url?.slice(`/${nonce}/`.length)
    if (!request.url?.startsWith(`/${nonce}/`)) {
      response.writeHead(404).end()
    } else if (request.method === 'GET' && resource === 'probe') {
      response
        .writeHead(200, { 'Content-Type': 'text/plain' })
        .end('chaindesk-m0-proxy-ok')
    } else if (
      request.method === 'GET' &&
      portable &&
      ['boot.ps1', ...portableHarnesses].includes(resource)
    ) {
      const content =
        resource === 'boot.ps1'
          ? [
              "$ErrorActionPreference = 'Stop'",
              "if ($env:COMPUTERNAME -ne 'CHAINDESK-M0' -or (Get-CimInstance Win32_ComputerSystem).Manufacturer -notmatch 'QEMU') { throw 'Disposable guest required' }",
              'New-Item -ItemType Directory -Force C:\\M0\\M1 | Out-Null',
              "[IO.File]::WriteAllText('C:\\M0\\accept-clean-vm.ps1', 'exit')",
              ...portableHarnesses.map(
                (file) =>
                  `Invoke-WebRequest -UseBasicParsing -Uri '${controller}/${file}' -OutFile 'C:\\M0\\M1\\${file}'`,
              ),
              `& 'C:\\M0\\M1\\accept-portable-vm.ps1' -Controller '${controller}' -Milestone '${values.milestone}'`,
            ].join('\r\n')
          : await readFile(
              path.join(projectRoot, 'scripts/chaindesk', resource),
              'utf8',
            )
      response
        .writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8' })
        .end(content)
    } else if (
      request.method === 'GET' &&
      chainScenario &&
      ['tls-ca.pem', 'tls-server.pem', 'tls-server.key'].includes(resource)
    ) {
      const names = {
        'tls-ca.pem': 'ca.pem',
        'tls-server.pem': 'server.pem',
        'tls-server.key': 'server.key',
      }
      response
        .writeHead(200, { 'Content-Type': 'application/octet-stream' })
        .end(await readFile(path.join(tlsFixtureDirectory, names[resource])))
    } else if (
      chainScenario &&
      resource === 'subscription-fixture' &&
      ['GET', 'POST'].includes(request.method)
    ) {
      if (request.method === 'POST')
        await subscriptionFixture.setSource(request)
      response.writeHead(200, { 'Content-Type': 'application/json' }).end(
        JSON.stringify({
          url: subscriptionFixture.url,
          requests: subscriptionFixture.requests,
        }),
      )
    } else if (
      recoveryScenario &&
      resource === 'probe-fixture' &&
      ['GET', 'POST'].includes(request.method)
    ) {
      if (request.method === 'POST') {
        let body = ''
        for await (const chunk of request) {
          body += chunk
          assert.ok(body.length <= 256)
        }
        probeFixture.setMode(JSON.parse(body).mode)
      }
      response
        .writeHead(200, { 'Content-Type': 'application/json' })
        .end(JSON.stringify(probeFixture.state))
    } else if (
      request.method === 'POST' &&
      portable &&
      resource === 'control'
    ) {
      let body = ''
      for await (const chunk of request) {
        body += chunk
        assert.ok(body.length < 8192)
      }
      const input = JSON.parse(body)
      assert.ok(
        ['send-key', 'screendump', 'query-status'].includes(input.execute),
      )
      const args =
        input.execute === 'screendump'
          ? { filename: path.join(runDirectory, 'latest-screen.ppm') }
          : input.arguments
      let output
      try {
        output = await command(input.execute, args)
      } catch (error) {
        response.writeHead(400).end(String(error))
        return
      }
      response
        .writeHead(200, { 'Content-Type': 'application/json' })
        .end(JSON.stringify(output))
    } else if (
      request.method === 'GET' &&
      [
        'payload.zip',
        'manifest.json',
        ...(portable ? ['original.exe'] : []),
      ].includes(resource)
    ) {
      const target = path.join(runDirectory, resource)
      response.writeHead(200, {
        'Content-Length': (await stat(target)).size,
        'Content-Type':
          resource === 'manifest.json'
            ? 'application/json; charset=utf-8'
            : 'application/zip',
      })
      createReadStream(target).pipe(response)
    } else if (
      request.method === 'POST' &&
      ['result', 'screenshot', ...(portable ? ['progress'] : [])].includes(
        resource,
      )
    ) {
      const chunks = []
      let length = 0
      for await (const chunk of request) {
        length += chunk.length
        assert.ok(
          length <= 5 * 1024 * 1024,
          'Guest evidence exceeds the size limit',
        )
        chunks.push(chunk)
      }
      const evidence = Buffer.concat(chunks)
      await writeFile(
        path.join(
          logDirectory,
          resource === 'screenshot'
            ? `${logPrefix}.png`
            : `${logPrefix}-${resource}.json`,
        ),
        evidence,
      )
      response.writeHead(200).end('OK')
      if (resource === 'progress') console.log(evidence.toString('utf8'))
      if (resource === 'result') complete(JSON.parse(evidence.toString('utf8')))
    } else {
      response.writeHead(404).end()
    }
  } catch (error) {
    console.error('Disposable VM controller request failed:', String(error))
    if (!response.headersSent) response.writeHead(500)
    if (!response.writableEnded)
      response.end('Disposable VM controller request failed')
    complete({ passed: false, error: String(error) })
  }
})
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const forwardPort = server.address().port
const controller = `http://10.0.2.2:${forwardPort}/${nonce}`
if (portable)
  await writeFile(
    path.join(logDirectory, 'portable-vm-control.json'),
    JSON.stringify({
      endpoint: `http://127.0.0.1:${forwardPort}/${nonce}/control`,
      controller,
      runDirectory,
      backingRun,
    }),
  )
if (!reuseGuest) {
  const acceptanceDirectory = path.join(
    answerDirectory,
    'M0',
    ...(portable ? ['M1'] : []),
  )
  await mkdir(acceptanceDirectory, { recursive: true })
  const harness = portable ? 'accept-portable-vm.ps1' : 'accept-clean-vm.ps1'
  await copyFile(
    path.join(projectRoot, 'scripts/chaindesk', harness),
    path.join(acceptanceDirectory, harness),
  )
  await copyFile(
    path.join(projectRoot, 'scripts/chaindesk/vm-debugger.ps1'),
    path.join(acceptanceDirectory, 'vm-debugger.ps1'),
  )
  if (chainScenario)
    await copyFile(
      path.join(projectRoot, 'scripts/chaindesk/chain-vm-scenario.ps1'),
      path.join(acceptanceDirectory, 'chain-vm-scenario.ps1'),
    )
  if (recoveryScenario)
    await copyFile(
      path.join(projectRoot, 'scripts/chaindesk/recovery-vm-scenario.ps1'),
      path.join(acceptanceDirectory, 'recovery-vm-scenario.ps1'),
    )
  const guestHarness = `C:\\M0\\${portable ? 'M1\\' : ''}${harness}`
  await writeFile(
    path.join(answerDirectory, 'M0', 'prepare.cmd'),
    [
      '@echo off',
      'powershell.exe -NoProfile -Command "if ((Get-CimInstance Win32_ComputerSystem).Manufacturer -notmatch \'QEMU\') { exit 2 }"',
      'if errorlevel 1 exit /b 2',
      'mkdir C:\\M0',
      'xcopy "%~dp0*" C:\\M0\\ /E /I /Y',
      'icacls C:\\M0 /grant "*S-1-5-32-545:(OI)(CI)M" /T',
      `reg.exe add HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Run /v M0Acceptance /t REG_SZ /d "powershell.exe -NoProfile -ExecutionPolicy Bypass -File ${guestHarness} -Controller ${controller}${portable ? ` -Milestone ${values.milestone}` : ''}" /f`,
    ].join('\r\n'),
  )
  const template = await readFile(
    path.join(projectRoot, 'scripts', 'chaindesk', 'clean-vm-unattend.xml'),
    'utf8',
  )
  await writeFile(
    path.join(answerDirectory, 'autounattend.xml'),
    template.replaceAll(
      '{{testPassword}}',
      `M0${randomBytes(16).toString('hex')}!`,
    ),
  )
}
await copyFile(
  reuseGuest
    ? path.join(backingRun, 'vars.fd')
    : path.join(qemuRoot, 'share', 'edk2-i386-vars.fd'),
  path.join(runDirectory, 'vars.fd'),
)
const disk = path.join(runDirectory, 'system.qcow2')
execFileSync(
  path.join(qemuRoot, 'qemu-img.exe'),
  reuseGuest
    ? [
        'create',
        '-f',
        'qcow2',
        '-F',
        'qcow2',
        '-b',
        path.join(backingRun, 'system.qcow2'),
        disk,
      ]
    : ['create', '-f', 'qcow2', disk, '64G'],
  { windowsHide: true, stdio: 'pipe' },
)

let monitor
let commandId = 0
const pending = new Map()
let ready
let resetRequested = false
let shutdownReason
const qmpServer = net.createServer((connection) => {
  monitor = connection
  const lines = readline.createInterface({ input: connection })
  lines.on('error', (error) => console.error(`QMP: ${error.message}`))
  lines.on('line', (line) => {
    const message = JSON.parse(line)
    if (message.QMP) ready()
    if (message.event === 'SHUTDOWN') {
      shutdownReason = message.data.reason
      resetRequested = message.data.reason === 'guest-reset'
    }
    if (pending.has(message.id)) {
      pending.get(message.id)(message)
      pending.delete(message.id)
    }
  })
})
await new Promise((resolve) => qmpServer.listen(0, '127.0.0.1', resolve))
async function command(execute, arguments_) {
  const id = ++commandId
  const reply = new Promise((resolve) => {
    pending.set(id, resolve)
  })
  monitor.write(`${JSON.stringify({ execute, arguments: arguments_, id })}\n`)
  const result = await Promise.race([
    reply,
    delay(5000, undefined, { ref: false }).then(() => {
      throw new Error(`QMP timed out: ${execute}`)
    }),
  ])
  if (result.error) throw new Error(JSON.stringify(result.error))
  return result.return
}
const arguments_ = [
  '-machine',
  'q35',
  '-accel',
  'whpx',
  '-cpu',
  'Skylake-Client,-hle,-rtm,-arat,-xsave,-xsaveopt,-xsavec,-xsaves,-avx,-avx2,-fma,-f16c',
  '-m',
  '6144',
  '-smp',
  '4',
  '-display',
  'none',
  '-rtc',
  'base=utc',
  '-drive',
  `if=pflash,format=raw,readonly=on,file=${path.join(qemuRoot, 'share', 'edk2-x86_64-code.fd')}`,
  '-drive',
  `if=pflash,format=raw,file=${path.join(runDirectory, 'vars.fd')}`,
  '-drive',
  `if=ide,format=qcow2,cache=writethrough,file=${disk}`,
  ...(!reuseGuest
    ? ['-drive', `if=ide,media=cdrom,readonly=on,file=${image}`]
    : []),
  '-device',
  'qemu-xhci',
  ...(!reuseGuest
    ? [
        '-drive',
        `if=none,id=answer,format=raw,file=fat:rw:${answerDirectory.replaceAll('\\', '/')}`,
        '-device',
        'usb-storage,drive=answer,removable=on',
      ]
    : []),
  '-nic',
  'user,model=e1000e',
  // WHPX 11.1 does not reset all processor state; restart QEMU on guest reset.
  '-no-reboot',
  '-qmp',
  `tcp:127.0.0.1:${qmpServer.address().port}`,
]
const log = createWriteStream(path.join(logDirectory, `${logPrefix}-qemu.log`))
console.log(
  JSON.stringify({
    phase: 'guestStarting',
    runDirectory,
    backingRun,
    milestone: values.milestone,
    accelerator: 'whpx',
    networking: 'userNatWithLoopbackController',
    platformSecurityAcceptance: false,
  }),
)
let capture
let guest
let exited
let boots = 0
try {
  const deadline = Date.now() + 45 * 60 * 1000
  let result
  do {
    resetRequested = false
    shutdownReason = undefined
    const connected = new Promise((resolve) => {
      ready = resolve
    })
    guest = spawn(
      path.join(qemuRoot, 'qemu-system-x86_64.exe'),
      [
        ...arguments_,
        '-boot',
        boots === 0 && !reuseGuest
          ? 'order=c,once=d,menu=off'
          : 'order=c,menu=off',
      ],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] },
    )
    guest.stdout.pipe(log, { end: false })
    guest.stderr.pipe(log, { end: false })
    exited = new Promise((resolve) =>
      guest.once('exit', (code) =>
        resolve({
          passed: false,
          resetRequested: code === 0 && resetRequested,
          error: `QEMU exited before acceptance: ${code}`,
        }),
      ),
    )
    guest.once('error', (error) =>
      complete({ passed: false, error: String(error) }),
    )
    await Promise.race([
      connected,
      exited.then((failure) => {
        throw new Error(failure.error)
      }),
      delay(20000, undefined, { ref: false }).then(() => {
        throw new Error('QMP startup timed out')
      }),
    ])
    await command('qmp_capabilities')
    assert.equal((await command('query-status')).running, true)
    capture = setInterval(() => {
      command('screendump', {
        filename: path.join(runDirectory, 'latest-screen.ppm'),
      }).catch((error) => console.error(String(error)))
    }, 30000)
    capture.unref()
    if (boots === 0 && !reuseGuest) {
      for (let attempt = 0; attempt < 4; attempt++) {
        await delay(1000)
        await command('send-key', {
          keys: [{ type: 'qcode', data: 'spc' }],
          'hold-time': 100,
        })
      }
    }
    boots++
    result = await Promise.race([
      completion,
      exited,
      delay(Math.max(1, deadline - Date.now()), undefined, { ref: false }).then(
        () => ({
          passed: false,
          error: 'Guest acceptance timed out',
        }),
      ),
    ])
    if (result.finishedAt) {
      if (guest.exitCode === null) await Promise.race([exited, delay(120000)])
      result.guestPoweredOff =
        guest.exitCode === 0 && shutdownReason === 'guest-shutdown'
      result.guestShutdownReason = shutdownReason
    }
    clearInterval(capture)
    if (result.resetRequested) monitor.destroy()
    if (result.resetRequested)
      console.log(JSON.stringify({ phase: 'guestReset', boots }))
  } while (result.resetRequested && boots < 5 && Date.now() < deadline)
  const hostEvidence = {
    runDirectory,
    backingRun,
    milestone: values.milestone,
    accelerator: 'whpx',
    qemuVersion: environment.qemu.version,
    image: environment.image,
    hostApplicationStarted: false,
    hostProxyModified: false,
    platformSecurityAcceptance: false,
    boots,
    probeFixture: probeFixture?.state,
    guestResult: result,
  }
  await writeFile(
    path.join(logDirectory, `${logPrefix}-evidence.json`),
    `${JSON.stringify(hostEvidence, null, 2)}\n`,
  )
  console.log(JSON.stringify(hostEvidence))
  assert.equal(result.passed, true, result.error)
  if (recoveryScenario)
    assert.equal(
      result.guestPoweredOff,
      true,
      'Verified guest shutdown is required for M3/M4 acceptance',
    )
} finally {
  clearInterval(capture)
  if (guest?.exitCode === null) {
    try {
      await command('quit')
    } catch {
      guest.kill()
    }
    await Promise.race([exited, delay(5000)])
    if (guest.exitCode === null) guest.kill()
  }
  monitor?.destroy()
  qmpServer.close()
  server.close()
  subscriptionFixture?.close()
  probeFixture?.close()
  log.end()
}
