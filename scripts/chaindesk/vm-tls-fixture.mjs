import { execFileSync } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'

export async function prepareVmTlsFixture(directory) {
  await mkdir(directory, { recursive: true })
  const gitPath = execFileSync('git', ['--exec-path'], {
    encoding: 'utf8',
  }).trim()
  const openssl = path.resolve(gitPath, '../../../usr/bin/openssl.exe')
  const run = (args) =>
    execFileSync(openssl, args, {
      cwd: directory,
      windowsHide: true,
      stdio: 'pipe',
    })
  run([
    'req',
    '-x509',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-days',
    '2',
    '-keyout',
    'ca.key',
    '-out',
    'ca.pem',
    '-subj',
    '/CN=ChainDesk_VM_fixture_only',
    '-addext',
    'basicConstraints=critical,CA:TRUE',
    '-addext',
    'keyUsage=critical,keyCertSign,cRLSign',
  ])
  run([
    'req',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-keyout',
    'server.key',
    '-out',
    'server.csr',
    '-subj',
    '/CN=front.fixture.test',
  ])
  await writeFile(
    path.join(directory, 'leaf.ext'),
    'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:front.fixture.test,IP:127.0.0.1,IP:10.0.2.2\n',
  )
  run([
    'x509',
    '-req',
    '-in',
    'server.csr',
    '-CA',
    'ca.pem',
    '-CAkey',
    'ca.key',
    '-CAcreateserial',
    '-out',
    'server.pem',
    '-days',
    '2',
    '-extfile',
    'leaf.ext',
  ])
}
