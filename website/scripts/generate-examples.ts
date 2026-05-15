#!/usr/bin/env bun
/**
 * Validate the pre-bundled landing-page examples.
 *
 * The examples are currently maintained in lib/bundled-examples.json. This
 * script exists so bundle:all can smoke-test that bundle without requiring
 * filesystem reads at runtime on Cloudflare Workers.
 */

import fs from 'fs'
import path from 'path'

type CodeExample = {
  id: string
  title: string
  description: string
  category: string
  code: string
  result: unknown
}

const outputPath = path.join(process.cwd(), 'lib', 'bundled-examples.json')

console.log('📦 Validating bundled examples...')

if (!fs.existsSync(outputPath)) {
  throw new Error(`Missing examples bundle: ${outputPath}`)
}

const examples = JSON.parse(fs.readFileSync(outputPath, 'utf8')) as CodeExample[]

if (!Array.isArray(examples)) {
  throw new Error('Examples bundle must be an array.')
}

for (const example of examples) {
  if (!example.id || !example.title || !example.description || !example.category || !example.code) {
    throw new Error(`Invalid example entry: ${JSON.stringify(example)}`)
  }
}

console.log(`✅ Validated ${examples.length} examples in ${outputPath}`)
