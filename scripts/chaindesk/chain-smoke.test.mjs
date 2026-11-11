import assert from 'node:assert/strict'
import { execFile, spawn } from 'node:child_process'
import { randomBytes, randomUUID } from 'node:crypto'
import { once } from 'node:events'
import { copyFile, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import http from 'node:http'
import net from 'node:net'
import { tmpdir } from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { setTimeout as delay } from 'node:timers/promises'
import { promisify } from 'node:util'

import { projectRoot, readBaseline, verifyFile } from './baseline.mjs'

const runFile = promisify(execFile)
const baseline = await readBaseline()
const executable = path.join(
  projectRoot,
  'src-tauri',
  'sidecar',
  `verge-mihomo-${baseline.toolchain.rustTarget}.exe`,
)

async function removeTemporaryDirectory(directory, prefix) {
  const resolved = path.resolve(directory)
  assert.equal(path.dirname(resolved), path.resolve(tmpdir()))
  assert.ok(path.basename(resolved).startsWith(prefix))
  await rm(resolved, { recursive: true, force: true })
}

test('a corrupted cached core cannot replace the prepared sidecar', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'chaindesk-integrity-'))
  try {
    await writeFile(
      path.join(directory, baseline.artifacts.core.name),
      'invalid',
    )
    await assert.rejects(
      runFile(
        process.execPath,
        [
          path.join(projectRoot, 'scripts/chaindesk/prepare-resources.mjs'),
          '--cache',
          directory,
        ],
        { cwd: projectRoot, timeout: 10_000, windowsHide: true },
      ),
      /SHA-256 mismatch/,
    )
    await verifyFile(executable, baseline.artifacts.core.files[0].sha256)
  } finally {
    await removeTemporaryDirectory(directory, 'chaindesk-integrity-')
  }
})

async function availablePort() {
  const server = net.createServer()
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const { port } = server.address()
  await new Promise((resolve) => server.close(resolve))
  return port
}

function request(port, requestPath, headers = {}) {
  return new Promise((resolve, reject) => {
    const outgoing = http.get(
      {
        host: '127.0.0.1',
        port,
        path: requestPath,
        headers: {
          ...(requestPath.startsWith('http://')
            ? { host: new URL(requestPath).host }
            : {}),
          ...headers,
        },
        agent: false,
      },
      (response) => {
        let body = ''
        response.setEncoding('utf8')
        response.on('data', (chunk) => {
          body += chunk
        })
        response.on('end', () => resolve({ status: response.statusCode, body }))
        response.on('error', reject)
      },
    )
    outgoing.setTimeout(5_000, () => outgoing.destroy(new Error('Timed out')))
    outgoing.on('error', reject)
  })
}

test('fixed core enforces SS front -> authenticated SOCKS5 exit', {
  timeout: 60_000,
}, async (context) => {
  await verifyFile(executable, baseline.artifacts.core.files[0].sha256)
  const sessionRoot = await mkdtemp(path.join(tmpdir(), 'chaindesk-chain-'))
  const children = []
  const token = randomBytes(32).toString('hex')
  const marker = randomUUID()
  let targetRequests = 0
  const target = http.createServer((_request, response) => {
    targetRequests += 1
    response.end(marker)
  })
  const environment = {
    SystemRoot: process.env.SystemRoot,
    WINDIR: process.env.WINDIR,
    TEMP: sessionRoot,
    TMP: sessionRoot,
  }

  async function stopCore(core) {
    if (core.child.exitCode === null && core.child.signalCode === null) {
      core.child.kill()
    }
    await core.exited
  }

  async function startCore(name, options) {
    const directory = path.join(sessionRoot, name)
    await mkdir(directory)
    for (const asset of baseline.artifacts.rules.assets) {
      await copyFile(
        path.join(projectRoot, 'src-tauri', 'resources', asset.name),
        path.join(directory, asset.name),
      )
      await verifyFile(path.join(directory, asset.name), asset.sha256)
    }
    const controller = await availablePort()
    const configuration = {
      'allow-lan': false,
      'bind-address': '127.0.0.1',
      'external-controller': `127.0.0.1:${controller}`,
      secret: token,
      mode: 'rule',
      ipv6: false,
      'geo-auto-update': false,
      'geodata-mode': true,
      'log-level': 'warning',
      dns: { enable: false },
      tun: { enable: false },
      ...options,
    }
    const configPath = path.join(directory, 'config.yaml')
    await writeFile(configPath, JSON.stringify(configuration))
    await runFile(executable, ['-t', '-d', directory, '-f', configPath], {
      timeout: 10_000,
      windowsHide: true,
      env: environment,
    })
    const child = spawn(executable, ['-d', directory, '-f', configPath], {
      windowsHide: true,
      env: environment,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    const core = { child, controller, exited: once(child, 'exit'), logs: '' }
    children.push(core)
    for (const output of [child.stdout, child.stderr]) {
      output.on('data', (chunk) => {
        core.logs = (core.logs + chunk.toString()).slice(-8_192)
      })
    }
    const deadline = Date.now() + 5_000
    while (Date.now() < deadline) {
      if (child.exitCode !== null || child.signalCode !== null) {
        throw new Error(`Core ${name} exited: ${core.logs}`)
      }
      const response = await request(controller, '/version', {
        authorization: `Bearer ${token}`,
      }).catch(() => null)
      if (response?.status === 200) {
        assert.equal(
          JSON.parse(response.body).version,
          baseline.artifacts.core.version,
        )
        return core
      }
      await delay(50)
    }
    throw new Error(`Core ${name} was not ready: ${core.logs}`)
  }

  try {
    target.listen(0, '127.0.0.1')
    await once(target, 'listening')
    const targetUrl = `http://127.0.0.1:${target.address().port}/probe`
    const exitPort = await availablePort()
    const frontPort = await availablePort()
    const workingPort = await availablePort()
    const exit = await startCore('exit', {
      'socks-port': exitPort,
      authentication: ['fixture-user:fixture-password'],
      rules: ['MATCH,DIRECT'],
    })
    const front = await startCore('front', {
      listeners: [
        {
          name: 'fixture-front',
          type: 'shadowsocks',
          listen: '127.0.0.1',
          port: frontPort,
          cipher: 'aes-128-gcm',
          password: 'fixture-front-password',
        },
      ],
      rules: ['MATCH,DIRECT'],
    })
    const chainOptions = {
      'mixed-port': workingPort,
      proxies: [
        {
          name: 'FIXTURE_FRONT',
          type: 'ss',
          server: '127.0.0.1',
          port: frontPort,
          cipher: 'aes-128-gcm',
          password: 'fixture-front-password',
        },
        {
          name: 'FIXTURE_EXIT',
          type: 'socks5',
          server: '127.0.0.1',
          port: exitPort,
          username: 'fixture-user',
          password: 'fixture-password',
          'dialer-proxy': 'FIXTURE_FRONT_GROUP',
        },
      ],
      'proxy-groups': [
        {
          name: 'FIXTURE_FRONT_GROUP',
          type: 'select',
          proxies: ['FIXTURE_FRONT'],
        },
      ],
      rules: ['MATCH,FIXTURE_EXIT'],
    }
    const working = await startCore('working', chainOptions)

    await context.test('traffic reaches both front and exit', async () => {
      const response = await request(workingPort, targetUrl)
      assert.equal(response.status, 200, response.body + working.logs)
      assert.equal(response.body, marker)
      assert.equal(targetRequests, 1)
      for (const core of [front, exit]) {
        const traffic = await request(core.controller, '/connections', {
          authorization: `Bearer ${token}`,
        })
        assert.equal(traffic.status, 200)
        const statistics = JSON.parse(traffic.body)
        assert.ok(statistics.uploadTotal > 0 || statistics.downloadTotal > 0)
      }
    })

    await context.test(
      'wrong exit credentials do not fall back to direct',
      async () => {
        const badPort = await availablePort()
        const badOptions = structuredClone(chainOptions)
        badOptions['mixed-port'] = badPort
        badOptions.proxies[1].password = 'incorrect-fixture-password'
        await startCore('bad-authentication', badOptions)
        const previousRequests = targetRequests
        const response = await request(badPort, targetUrl).catch(() => null)
        assert.notEqual(response?.status, 200)
        assert.equal(targetRequests, previousRequests)
      },
    )

    await context.test(
      'stopping the exit never sends the request directly',
      async () => {
        await stopCore(exit)
        const previousRequests = targetRequests
        const response = await request(workingPort, targetUrl).catch(() => null)
        assert.notEqual(response?.status, 200)
        assert.equal(targetRequests, previousRequests)
      },
    )
  } finally {
    await Promise.all(children.map(stopCore))
    target.closeAllConnections()
    await new Promise((resolve) => target.close(resolve))
    await removeTemporaryDirectory(sessionRoot, 'chaindesk-chain-')
  }
})
