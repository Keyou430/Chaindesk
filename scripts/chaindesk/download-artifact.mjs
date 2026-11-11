import { createWriteStream } from 'node:fs'
import { rename, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'

import { verifyFile } from './baseline.mjs'

export async function prepareArtifact(cache, artifact) {
  const destination = path.join(cache, artifact.name)
  let cached = true
  try {
    await stat(destination)
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
    cached = false
  }
  if (!cached) {
    const partial = `${destination}.partial`
    try {
      const response = await fetch(artifact.url, {
        signal: AbortSignal.timeout(120_000),
      })
      if (!response.ok || !response.url.startsWith('https://')) {
        throw new Error(`Artifact download failed: ${artifact.name}`)
      }
      await pipeline(
        Readable.fromWeb(response.body),
        createWriteStream(partial, { flags: 'wx' }),
      )
      await verifyFile(partial, artifact.sha256)
      await rename(partial, destination)
    } finally {
      await rm(partial, { force: true })
    }
  }
  await verifyFile(destination, artifact.sha256)
  return destination
}
