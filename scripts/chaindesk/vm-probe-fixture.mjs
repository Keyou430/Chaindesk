import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import http from 'node:http'
import https from 'node:https'
import net from 'node:net'
import path from 'node:path'

const hosts = new Set([
  'www.gstatic.com:443',
  'cp.cloudflare.com:443',
  'api.ipify.org:443',
  'www.cloudflare.com:443',
])
const modes = new Set([
  'ok',
  'unavailable',
  'inconsistent',
  'changed',
  'invalid',
  'working-failure',
])

export async function createVmProbeFixture(directory) {
  let mode = 'ok'
  let httpsFailuresRemaining = 0
  const requests = {}
  const connections = {}
  const tlsErrors = {}
  const events = []
  const record = (event) => {
    events.push({ at: new Date().toISOString(), ...event })
    if (events.length > 64) events.shift()
  }
  const sockets = new Set()
  const tls = https.createServer(
    {
      key: await readFile(path.join(directory, 'probe.key')),
      cert: await readFile(path.join(directory, 'probe.pem')),
    },
    (request, response) => {
      const host = request.headers.host?.split(':')[0]
      record({
        event: 'request',
        service: hosts.has(`${host}:443`) ? host : 'refused',
        mode,
      })
      requests[host] = (requests[host] ?? 0) + 1
      if (['www.gstatic.com', 'cp.cloudflare.com'].includes(host)) {
        if (httpsFailuresRemaining > 0) {
          if (--httpsFailuresRemaining === 0) mode = 'ok'
          response.writeHead(503).end()
          return
        }
        response.writeHead(204).end()
        return
      }
      if (mode === 'unavailable') {
        response.writeHead(503).end()
        return
      }
      const ip =
        mode === 'invalid'
          ? 'not-an-ip'
          : mode === 'changed' ||
              (mode === 'inconsistent' && host === 'www.cloudflare.com')
            ? '203.0.113.81'
            : '203.0.113.80'
      if (host === 'api.ipify.org')
        response
          .writeHead(200, { 'Content-Type': 'application/json' })
          .end(JSON.stringify({ ip }))
      else if (host === 'www.cloudflare.com')
        response
          .writeHead(200, { 'Content-Type': 'text/plain' })
          .end(`ip=${ip}\nfixture=disposable-vm-only\n`)
      else response.writeHead(404).end()
    },
  )
  tls.on('tlsClientError', (error) => {
    const code = /^ERR_[A-Z0-9_]+$/.test(error.code ?? '')
      ? error.code
      : 'TLS_HANDSHAKE_FAILED'
    tlsErrors[code] = (tlsErrors[code] ?? 0) + 1
    record({ event: 'tlsError', code })
  })
  await new Promise((resolve) => tls.listen(0, '127.0.0.1', resolve))
  const proxy = http.createServer((_, response) =>
    response.writeHead(405).end(),
  )
  proxy.on('connect', (request, client, head) => {
    const host = hosts.has(request.url) ? request.url : 'refused'
    connections[host] = (connections[host] ?? 0) + 1
    record({ event: 'connect', service: host })
    if (!hosts.has(request.url)) {
      client.end('HTTP/1.1 403 Forbidden\r\n\r\n')
      return
    }
    const upstream = net.connect(tls.address().port, '127.0.0.1', () => {
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n')
      if (head.length) upstream.write(head)
      client.pipe(upstream).pipe(client)
    })
    for (const socket of [client, upstream]) {
      sockets.add(socket)
      socket.setTimeout(15000, () => socket.destroy())
      socket.on('error', () => socket.destroy())
      socket.on('close', () => {
        sockets.delete(socket)
        if (socket === client) upstream.destroy()
        else client.destroy()
      })
    }
  })
  await new Promise((resolve) => proxy.listen(0, '127.0.0.1', resolve))
  return {
    get state() {
      return {
        mode,
        httpsFailuresRemaining,
        requests,
        connections,
        tlsErrors,
        events,
        proxyPort: proxy.address().port,
        syntheticIpv4: true,
        trustScope: 'Disposable guest CurrentUser only',
      }
    },
    setMode(value) {
      assert.ok(modes.has(value), 'Unsupported probe fixture mode')
      mode = value
      httpsFailuresRemaining = value === 'working-failure' ? 2 : 0
    },
    close() {
      for (const socket of sockets) socket.destroy()
      tls.closeAllConnections()
      tls.close()
      proxy.closeAllConnections()
      proxy.close()
    },
  }
}
