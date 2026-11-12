import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const projectRoot = fileURLToPath(new URL('../../', import.meta.url))

export async function readBaseline() {
  return JSON.parse(
    await readFile(
      new URL('../../docs/engineeringBaseline.json', import.meta.url),
      'utf8',
    ),
  )
}

export async function verifyFile(filePath, expectedSha256) {
  const digest = createHash('sha256')
  for await (const chunk of createReadStream(filePath)) digest.update(chunk)
  if (digest.digest('hex') !== expectedSha256) {
    throw new Error(`SHA-256 mismatch: ${filePath}`)
  }
}

export function verifyVsCatalog(catalogPath) {
  return JSON.parse(
    execFileSync(
      'powershell.exe',
      [
        '-NoProfile',
        '-NonInteractive',
        '-File',
        path.join(projectRoot, 'scripts', 'chaindesk', 'verify-vs-catalog.ps1'),
        '-CatalogPath',
        catalogPath,
      ],
      {
        windowsHide: true,
        encoding: 'utf8',
        env: {
          ...process.env,
          PSModulePath: path.join(
            process.env.SystemRoot,
            'System32',
            'WindowsPowerShell',
            'v1.0',
            'Modules',
          ),
        },
      },
    ),
  )
}

export async function verifySource(baseline) {
  execFileSync(
    'git',
    ['merge-base', '--is-ancestor', baseline.source.commit, 'HEAD'],
    { cwd: projectRoot },
  )
  for (const [filePath, expected] of Object.entries(baseline.source.files)) {
    const original = execFileSync(
      'git',
      ['show', `${baseline.source.commit}:${filePath}`],
      { cwd: projectRoot },
    )
    const digest = createHash('sha256').update(original).digest('hex')
    if (original.length !== expected.size || digest !== expected.sha256) {
      throw new Error(`Upstream source mismatch: ${filePath}`)
    }
  }
  for (const filePath of ['pnpm-lock.yaml', 'Cargo.lock']) {
    await verifyFile(
      new URL(`../../${filePath}`, import.meta.url),
      baseline.implementation.customLockfiles?.[filePath]?.sha256 ??
        baseline.source.files[filePath].sha256,
    )
  }
}
