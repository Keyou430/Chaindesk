import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import https from 'node:https'
import path from 'node:path'
import { gzipSync } from 'node:zlib'

export async function createVmSubscriptionFixture(directory, nonce) {
  let source = ''
  const requests = {}
  const server = https.createServer(
    {
      key: await readFile(path.join(directory, 'server.key')),
      cert: await readFile(path.join(directory, 'server.pem')),
    },
    (request, response) => {
      const url = new URL(request.url, 'https://fixture.invalid')
      if (!url.pathname.startsWith(`/${nonce}/`)) {
        response.writeHead(404).end()
        return
      }
      const resource = url.pathname.slice(nonce.length + 2)
      requests[resource] = (requests[resource] ?? 0) + 1
      if (resource === 'denied') response.writeHead(403).end()
      else if (resource === 'retry' && requests.retry < 3)
        response.writeHead(503).end()
      else if (resource === 'large')
        response
          .writeHead(200, { 'Content-Encoding': 'gzip' })
          .end(gzipSync(Buffer.alloc(10 * 1024 * 1024 + 1, 32)))
      else if (resource === 'invalid-utf8')
        response.writeHead(200).end(Buffer.from([255]))
      else if (resource === 'downgrade')
        response.writeHead(302, { Location: 'http://127.0.0.1:1/' }).end()
      else if (['subscription', 'retry'].includes(resource))
        response.writeHead(200, { 'Content-Type': 'text/yaml' }).end(source)
      else response.writeHead(404).end()
    },
  )
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  return {
    url: `https://10.0.2.2:${server.address().port}/${nonce}`,
    requests,
    async setSource(request) {
      const chunks = []
      let length = 0
      for await (const chunk of request) {
        length += chunk.length
        assert.ok(length <= 10 * 1024 * 1024)
        chunks.push(chunk)
      }
      source = Buffer.concat(chunks).toString('utf8')
    },
    close() {
      server.closeAllConnections()
      server.close()
    },
  }
}
