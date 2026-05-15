#!/usr/bin/env bun
/**
 * Keep the generated examples bundle available for builds.
 *
 * This repo currently stores the compiled examples directly in lib/bundled-examples.json.
 * When source examples are added later, this script can be expanded to regenerate that file.
 */

import fs from 'fs'
import path from 'path'

const outputPath = path.join(process.cwd(), 'lib', 'bundled-examples.json')

console.log('📦 Checking bundled examples...')

if (!fs.existsSync(outputPath)) {
  fs.writeFileSync(outputPath, '[]\n', 'utf8')
  console.log(`✅ Created empty examples bundle at ${outputPath}`)
} else {
  const raw = fs.readFileSync(outputPath, 'utf8')
  JSON.parse(raw)
  console.log(`✅ Examples bundle is valid at ${outputPath}`)
}
