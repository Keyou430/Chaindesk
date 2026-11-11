import { execFileSync } from 'node:child_process'

import { projectRoot, readBaseline, verifySource } from './baseline.mjs'

const baseline = await readBaseline()
await verifySource(baseline)
if (process.versions.node !== baseline.toolchain.node) {
  throw new Error(`Node.js ${baseline.toolchain.node} is required`)
}

const rust = execFileSync('rustc', ['--version'], {
  cwd: projectRoot,
  encoding: 'utf8',
}).trim()
if (rust.split(' ')[1] !== baseline.toolchain.rust) {
  throw new Error(`Rust ${baseline.toolchain.rust} is required`)
}
const pnpm = execFileSync('pnpm.exe', ['--version'], {
  cwd: projectRoot,
  encoding: 'utf8',
}).trim()
if (pnpm !== baseline.toolchain.pnpm) {
  throw new Error(`pnpm ${baseline.toolchain.pnpm} is required`)
}

console.log(
  JSON.stringify({
    sourceCommit: baseline.source.commit,
    sourceFilesVerified: Object.keys(baseline.source.files).length,
    lockfilesVerified: 2,
    node: process.versions.node,
    pnpm,
    rust,
    releaseReady: false,
    releaseGates: baseline.releaseGates,
  }),
)
