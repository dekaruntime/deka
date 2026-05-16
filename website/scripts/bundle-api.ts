#!/usr/bin/env bun
/**
 * Pre-bundle all API markdown files into a static JSON file
 * for Cloudflare Workers compatibility.
 *
 * Usage:
 *   bun scripts/bundle-api.ts --source content/api/api-reference --lang en
 *   bun scripts/bundle-api.ts --source content-i18n/es/api/api-reference --lang es
 */

import fs from 'fs'
import path from 'path'
import matter from 'gray-matter'
import { remark } from 'remark'
import html from 'remark-html'

interface APIDoc {
  slug: string[]
  metadata: {
    title: string
    description?: string
  }
  content: string
  html: string
  codeBlocks: Array<{lang: string, code: string}>
}

function parseArgs(argv: string[]) {
  const args = new Map<string, string | boolean>()
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i]
    if (!arg.startsWith('--')) continue
    const [key, value] = arg.split('=')
    if (value === undefined) {
      const next = argv[i + 1]
      if (next && !next.startsWith('--')) {
        args.set(key, next)
        i += 1
      } else {
        args.set(key, true)
      }
    } else {
      args.set(key, value)
    }
  }

  const source = String(args.get('--source') || 'content/api/api-reference')
  const lang = String(args.get('--lang') || 'en')
  const out = args.get('--out')

  return { source, lang, out }
}

const { source, lang, out } = parseArgs(process.argv)
const apiDirectory = path.join(process.cwd(), source)
const outputPath = out
  ? path.join(process.cwd(), String(out))
  : path.join(
      process.cwd(),
      'lib',
      lang === 'en' ? 'bundled-api.json' : `bundled-api.${lang}.json`
    )

async function convertToHtml(content: string): Promise<string> {
  try {
    const result = await remark()
      .use(html, { sanitize: false })
      .process(content)
    return result.toString()
  } catch (error) {
    console.error('Error converting markdown:', error)
    return `<p>Error rendering content</p>`
  }
}

async function getAllAPIDocs(): Promise<APIDoc[]> {
  const docs: APIDoc[] = []

  async function readDir(dir: string, slugParts: string[] = []) {
    const files = fs.readdirSync(dir).sort()

    for (const file of files) {
      const filePath = path.join(dir, file)
      const stat = fs.statSync(filePath)

      if (stat.isDirectory()) {
        await readDir(filePath, [...slugParts, file])
      } else if (file.endsWith('.md') || file.endsWith('.mdx')) {
        const fileContents = fs.readFileSync(filePath, 'utf8')
        const { data, content } = matter(fileContents)

        const fileName = file.replace(/\.mdx?$/, '')
        const slug = [...slugParts, fileName]

        const codeBlocks: Array<{lang: string, code: string}> = []
        const contentWithPlaceholders = content.replace(/```(\w+)?\n([\s\S]*?)```/g, (match, lang, code) => {
          const index = codeBlocks.length
          codeBlocks.push({ lang: lang || 'text', code: code.trim() })
          return `\n\n<div class="code-block-placeholder" data-index="${index}"></div>\n\n`
        })

        const htmlContent = await convertToHtml(contentWithPlaceholders)

        docs.push({
          slug,
          metadata: {
            title: data.title || fileName,
            description: data.description,
          },
          content,
          html: htmlContent,
          codeBlocks,
        })
      }
    }
  }

  await readDir(apiDirectory)
  return docs
}

console.log(`📦 Bundling API docs (${lang})...`)
const docs = await getAllAPIDocs()
const bundleContent = JSON.stringify(docs, null, 2)

fs.writeFileSync(outputPath, bundleContent, 'utf8')

console.log(`✅ Bundled ${docs.length} API docs to ${outputPath}`)
console.log(`📊 Bundle size: ${(bundleContent.length / 1024).toFixed(2)} KB`)
