import { expect, test } from 'bun:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { installPackages } from './build-native'

test('package cache preserves grants, bypasses legacy entries, and supports grant-free packages', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dump-cache-test-'))
  const previous = process.cwd()
  try {
    process.chdir(root)
    const cli = path.join(root, 'fake-deka')
    fs.writeFileSync(cli, `#!/bin/sh
set -eu
echo install >> '${root}/installs'
mkdir -p ds_modules/@deka/"$2"
echo '{"version":"1.0.0"}' > ds_modules/@deka/"$2"/deka.json
echo '{"packages":{}}' > deka.lock
if [ "$2" = fs ]; then
  echo '{"grants":[{"name":"@deka/fs","version":"1.0.0","digest":"fixture-digest","kinds":["fs"]}]}' > deka.grants.json
fi
`, { mode: 0o755 })
    // An old cache looks complete to the previous harness, but has no grants.
    const legacy = path.join(root, '.cache/deka-packages/fs')
    fs.mkdirSync(path.join(legacy, 'ds_modules/@deka/fs'), { recursive: true })
    fs.writeFileSync(path.join(legacy, 'deka.lock'), '{}')
    for (const pkg of ['fs', 'io']) {
      const cold = path.join(root, `${pkg}-cold`)
      const warm = path.join(root, `${pkg}-warm`)
      fs.mkdirSync(cold)
      fs.mkdirSync(warm)
      expect(installPackages(cli, cold, [pkg]).ok).toBe(true)
      expect(installPackages(cli, warm, [pkg]).ok).toBe(true)
      expect(fs.readFileSync(path.join(warm, 'deka.lock'), 'utf8'))
        .toBe(fs.readFileSync(path.join(cold, 'deka.lock'), 'utf8'))
      expect(JSON.parse(fs.readFileSync(path.join(warm, 'deka.json'), 'utf8')).dependencies)
        .toEqual({ [`@deka/${pkg}`]: '1.0.0' })
      if (pkg === 'fs') {
        expect(fs.readFileSync(path.join(warm, 'deka.grants.json'), 'utf8'))
          .toBe(fs.readFileSync(path.join(cold, 'deka.grants.json'), 'utf8'))
      } else {
        expect(fs.existsSync(path.join(warm, 'deka.grants.json'))).toBe(false)
      }
    }
    expect(fs.readFileSync(path.join(root, 'installs'), 'utf8')).toBe('install\ninstall\n')
  } finally {
    process.chdir(previous)
    fs.rmSync(root, { recursive: true, force: true })
  }
})
