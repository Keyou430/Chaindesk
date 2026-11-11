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

const { values } = parseArgs({
  options: { image: { type: 'string' }, 'qemu-root': { type: 'string' } },
})
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
const runDirectory = await mkdtemp(path.join(cache, 'm0-'))
const answerDirectory = path.join(runDirectory, 'answer')
await mkdir(path.join(answerDirectory, 'M0'), { recursive: true })
const logDirectory = path.join(projectRoot, '.chaindesk', 'logs')
await mkdir(logDirectory, { recursive: true })
const payloadPath = path.join(runDirectory, 'payload.zip')
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
const manifest = {
  sourceCommit: baseline.source.commit,
  archiveSha256: digest.digest('hex'),
  runtimeDirectory: 'runtime',
  files,
}
await writeFile(
  path.join(runDirectory, 'manifest.json'),
  `${JSON.stringify(manifest, null, 2)}\n`,
)
const nonce = randomBytes(24).toString('hex')
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
      ['payload.zip', 'manifest.json'].includes(resource)
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
      ['result', 'screenshot'].includes(resource)
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
          resource === 'result' ? 'clean-vm-result.json' : 'clean-vm.png',
        ),
        evidence,
      )
      response.writeHead(200).end('OK')
      if (resource === 'result') complete(JSON.parse(evidence.toString('utf8')))
    } else {
      response.writeHead(404).end()
    }
  } catch (error) {
    response.writeHead(500).end(String(error))
    complete({ passed: false, error: String(error) })
  }
})
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const forwardPort = server.address().port
const controller = `http://10.0.2.2:${forwardPort}/${nonce}`
await copyFile(
  path.join(projectRoot, 'scripts', 'chaindesk', 'accept-clean-vm.ps1'),
  path.join(answerDirectory, 'M0', 'accept-clean-vm.ps1'),
)
await writeFile(
  path.join(answerDirectory, 'M0', 'prepare.cmd'),
  [
    '@echo off',
    'powershell.exe -NoProfile -Command "if ((Get-CimInstance Win32_ComputerSystem).Manufacturer -notmatch \'QEMU\') { exit 2 }"',
    'if errorlevel 1 exit /b 2',
    'mkdir C:\\M0',
    'xcopy "%~dp0*" C:\\M0\\ /E /I /Y',
    'icacls C:\\M0 /grant "*S-1-5-32-545:(OI)(CI)M" /T',
    `reg.exe add HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Run /v M0Acceptance /t REG_SZ /d "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\\M0\\accept-clean-vm.ps1 -Controller ${controller}" /f`,
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
await copyFile(
  path.join(qemuRoot, 'share', 'edk2-i386-vars.fd'),
  path.join(runDirectory, 'vars.fd'),
)
const disk = path.join(runDirectory, 'system.qcow2')
execFileSync(
  path.join(qemuRoot, 'qemu-img.exe'),
  ['create', '-f', 'qcow2', disk, '64G'],
  { windowsHide: true, stdio: 'pipe' },
)

let monitor
let commandId = 0
const pending = new Map()
let ready
let resetRequested = false
const qmpServer = net.createServer((connection) => {
  monitor = connection
  const lines = readline.createInterface({ input: connection })
  lines.on('error', (error) => console.error(`QMP: ${error.message}`))
  lines.on('line', (line) => {
    const message = JSON.parse(line)
    if (message.QMP) ready()
    if (message.event === 'SHUTDOWN') {
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
  `if=ide,format=qcow2,file=${disk}`,
  '-drive',
  `if=ide,media=cdrom,readonly=on,file=${image}`,
  '-device',
  'qemu-xhci',
  '-drive',
  `if=none,id=answer,format=raw,file=fat:rw:${answerDirectory.replaceAll('\\', '/')}`,
  '-device',
  'usb-storage,drive=answer,removable=on',
  '-nic',
  'user,model=e1000e',
  // WHPX 11.1 does not reset all processor state; restart QEMU on guest reset.
  '-no-reboot',
  '-qmp',
  `tcp:127.0.0.1:${qmpServer.address().port}`,
]
const log = createWriteStream(path.join(logDirectory, 'clean-vm-qemu.log'))
console.log(
  JSON.stringify({
    phase: 'guestStarting',
    runDirectory,
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
    const connected = new Promise((resolve) => {
      ready = resolve
    })
    guest = spawn(
      path.join(qemuRoot, 'qemu-system-x86_64.exe'),
      [
        ...arguments_,
        '-boot',
        boots === 0 ? 'order=c,once=d,menu=off' : 'order=c,menu=off',
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
    if (boots === 0) {
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
    clearInterval(capture)
    if (result.resetRequested) monitor.destroy()
    if (result.resetRequested)
      console.log(JSON.stringify({ phase: 'guestReset', boots }))
  } while (result.resetRequested && boots < 5 && Date.now() < deadline)
  const hostEvidence = {
    runDirectory,
    accelerator: 'whpx',
    qemuVersion: environment.qemu.version,
    image: environment.image,
    hostApplicationStarted: false,
    hostProxyModified: false,
    platformSecurityAcceptance: false,
    boots,
    guestResult: result,
  }
  await writeFile(
    path.join(logDirectory, 'clean-vm-evidence.json'),
    `${JSON.stringify(hostEvidence, null, 2)}\n`,
  )
  console.log(JSON.stringify(hostEvidence))
  assert.equal(result.passed, true, result.error)
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
  log.end()
}
