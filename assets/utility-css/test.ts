#!/usr/bin/env bun
/**
 * Smoke tests for the shared utility-CSS generator.
 */

import { injectUtilityCss, collectClasses } from './index.ts'
import registry from './registry.json'

const registryJson = JSON.stringify(registry)

interface Case {
  cls: string
  expect: string
}

const cases: Case[] = [
  { cls: 'p-4', expect: 'padding:1rem' },
  { cls: 'px-6', expect: 'padding-left:1.5rem;padding-right:1.5rem' },
  { cls: 'mt-8', expect: 'margin-top:2rem' },
  { cls: 'mx-auto', expect: 'margin-left:auto;margin-right:auto' },
  { cls: 'bg-blue-600', expect: 'background-color:#2563eb' },
  { cls: 'text-white', expect: 'color:#ffffff' },
  { cls: 'text-xl', expect: 'font-size:1.25rem;line-height:1.75rem' },
  { cls: 'font-bold', expect: 'font-weight:700' },
  { cls: 'hover:bg-blue-700', expect: 'background-color:#1d4ed8' },
  { cls: 'md:grid-cols-3', expect: 'grid-template-columns:repeat(3,minmax(0,1fr))' },
  { cls: 'w-[100px]', expect: 'width:100px' },
  { cls: 'grid-cols-[1fr_2fr]', expect: 'grid-template-columns:1fr 2fr' },
  { cls: 'flex', expect: 'display:flex' },
  { cls: 'hidden', expect: 'display:none' },
  { cls: 'rounded-lg', expect: 'border-radius:0.5rem' },
  { cls: 'shadow-md', expect: 'box-shadow:0 4px 6px -1px' },
  { cls: 'gap-4', expect: 'gap:1rem' },
  { cls: 'items-center', expect: 'align-items:center' },
  { cls: 'justify-between', expect: 'justify-content:space-between' },
  { cls: 'min-h-screen', expect: 'min-height:100vh' },
  { cls: 'max-w-6xl', expect: 'max-width:72rem' },
  { cls: 'border', expect: 'border-width:1px;border-style:solid' },
  { cls: 'border-b', expect: 'border-bottom-width:1px;border-bottom-style:solid' },
  { cls: 'border-gray-300', expect: 'border-color:#d1d5db' },
  { cls: 'border-b-gray-300', expect: 'border-bottom-color:#d1d5db' },
]

let passed = 0
let failed = 0

for (const t of cases) {
  const html = `<div class="${t.cls}"></div>`
  const out = injectUtilityCss(html, registryJson, { includePreflight: false })
  const ok = out.includes(t.expect)
  if (ok) {
    passed++
  } else {
    failed++
    console.log(`FAIL ${t.cls}`)
    console.log(`  expected: ${t.expect}`)
    console.log(`  got:      ${out}`)
  }
}

// Class scanner
const classes = collectClasses('<div class="a b-c"></div><span class="d"></span>')
if (!classes.has('a') || !classes.has('b-c') || !classes.has('d')) {
  failed++
  console.log('FAIL class scanner')
} else {
  passed++
}

console.log(`\nPassed: ${passed} | Failed: ${failed} | Total: ${cases.length + 1}`)
if (failed > 0) process.exit(1)
