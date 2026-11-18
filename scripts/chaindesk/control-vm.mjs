import assert from 'node:assert/strict'
import { X509Certificate } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { parseArgs } from 'node:util'

import { projectRoot } from './baseline.mjs'

const { values } = parseArgs({
  options: {
    login: { type: 'boolean' },
    launch: { type: 'boolean' },
    capture: { type: 'boolean' },
    inspect: { type: 'boolean' },
    ping: { type: 'boolean' },
    processes: { type: 'boolean' },
    'approve-fixture-ca': { type: 'boolean' },
    'prepare-shutdown': { type: 'boolean' },
  },
})
const control = JSON.parse(
  await readFile(
    path.join(projectRoot, '.chaindesk/logs/portable-vm-control.json'),
    'utf8',
  ),
)
async function command(execute, arguments_) {
  const response = await fetch(control.endpoint, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ execute, arguments: arguments_ }),
    signal: AbortSignal.timeout(10000),
  })
  assert.ok(response.ok, await response.text())
}
async function key(...codes) {
  await command('send-key', {
    keys: codes.map((data) => ({ type: 'qcode', data })),
    'hold-time': 50,
  })
  await delay(90)
}
async function type(text) {
  assert.ok(text.length < 260, 'Guest Run commands must fit its 259-character input limit')
  const plain = {
    ' ': 'spc',
    '/': 'slash',
    '\\': 'backslash',
    '.': 'dot',
    '-': 'minus',
    '=': 'equal',
    ';': 'semicolon',
    ',': 'comma',
    "'": 'apostrophe',
    '[': 'bracket_left',
    ']': 'bracket_right',
  }
  const shifted = {
    '>': 'dot',
    ':': 'semicolon',
    '"': 'apostrophe',
    '+': 'equal',
    '|': 'backslash',
    '&': '7',
    '(': '9',
    ')': '0',
    $: '4',
    '!': '1',
    '@': '2',
    '{': 'bracket_left',
    '}': 'bracket_right',
  }
  for (const char of text) {
    if (shifted[char]) await key('shift', shifted[char])
    else if (/[A-Z]/.test(char)) await key('shift', char.toLowerCase())
    else await key(plain[char] ?? char)
  }
}
if (values.login) {
  const answer = await readFile(
    path.join(
      control.backingRun ?? control.runDirectory,
      'answer/autounattend.xml',
    ),
    'utf8',
  )
  const password = answer.match(/<Password><Value>([^<]+)<\/Value>/)?.[1]
  assert.ok(password)
  await key('spc')
  await delay(1500)
  await type(password)
  await key('ret')
}
if (values.launch) {
  await key('esc')
  await key('meta_l', 'r')
  await delay(10000)
  await key('ctrl', 'a')
  await type(
    `powershell -nop -ep bypass -c "[Net.WebRequest]::DefaultWebProxy=$null;irm ${control.controller}/boot.ps1|iex"`,
  )
  await key('ret')
}
if (values.inspect) {
  await key('esc')
  await key('meta_l', 'r')
  await delay(1500)
  await key('ctrl', 'a')
  await type(
    `cmd /c curl.exe --noproxy 10.0.2.2 -fsS --max-time 5 -o C:\\M0\\i.ps1 ${control.controller}/vm-debugger.ps1 && powershell -nop -ep bypass -c ". C:\\M0\\i.ps1;Send-VmDiagnostic"`,
  )
  await key('ret')
}
if (values.ping) {
  await key('esc')
  await key('meta_l', 'r')
  await delay(1500)
  await key('ctrl', 'a')
  await type(
    `curl.exe --noproxy 10.0.2.2 --connect-timeout 3 --max-time 5 --data {} ${control.controller}/progress`,
  )
  await key('ret')
}
if (values.processes) {
  await key('esc')
  await key('meta_l', 'r')
  await delay(1500)
  await key('ctrl', 'a')
  await type(
    `cmd /c tasklist /fo csv > C:\\M0\\t.csv && curl.exe --noproxy 10.0.2.2 --max-time 5 --data-binary @C:\\M0\\t.csv ${control.controller}/progress`,
  )
  await key('ret')
}
if (values['approve-fixture-ca']) {
  const certificate = new X509Certificate(
    await readFile(path.join(control.runDirectory, 'tls-fixture/ca.pem')),
  )
  await key('esc')
  await key('meta_l', 'r')
  await delay(1500)
  await key('ctrl', 'a')
  await type(
    `powershell -nop -ep bypass -c "[Net.WebRequest]::DefaultWebProxy=$null;irm ${control.controller}/vm-debugger.ps1|iex;Confirm-VmFixtureCertificate 0 '${certificate.fingerprint.replaceAll(':', '')}'"`,
  )
  await key('ret')
}
if (values['prepare-shutdown']) {
  await key('esc')
  await key('meta_l', 'r')
  await delay(1500)
  await key('ctrl', 'a')
  await type(
    'cmd /c "echo exit>C:\\M0\\M1\\accept-portable-vm.ps1 & taskkill /f /im powershell.exe & shutdown /s /f /t 0"',
  )
  await key('ret')
  console.log(
    'Requested clean shutdown of the disposable guest; acceptance is not implied',
  )
}
if (values.capture) {
  await command('screendump')
  console.log(path.join(control.runDirectory, 'latest-screen.ppm'))
}
