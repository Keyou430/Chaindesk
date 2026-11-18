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
  const leaf = async (name, commonName, alternatives) => {
    run([
      'req',
      '-newkey',
      'rsa:2048',
      '-nodes',
      '-keyout',
      `${name}.key`,
      '-out',
      `${name}.csr`,
      '-subj',
      `/CN=${commonName}`,
    ])
    await writeFile(
      path.join(directory, `${name}.ext`),
      `basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=${alternatives}\n`,
    )
    run([
      'x509',
      '-req',
      '-in',
      `${name}.csr`,
      '-CA',
      'ca.pem',
      '-CAkey',
      'ca.key',
      '-CAcreateserial',
      '-out',
      `${name}.pem`,
      '-days',
      '2',
      '-extfile',
      `${name}.ext`,
    ])
  }
  await leaf(
    'server',
    'front.fixture.test',
    'DNS:front.fixture.test,IP:127.0.0.1,IP:10.0.2.2',
  )
  await leaf(
    'probe',
    'ChainDesk_VM_probe_fixture_only',
    'DNS:www.gstatic.com,DNS:cp.cloudflare.com,DNS:api.ipify.org,DNS:www.cloudflare.com',
  )
}
