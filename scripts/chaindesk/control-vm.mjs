import assert from 'node:assert/strict'
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
  await delay(1500)
  await key('ctrl', 'a')
  await type(
    `powershell -nop -ep bypass -c "irm ${control.controller}/boot.ps1|iex"`,
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
