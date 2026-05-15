#!/usr/bin/env bun
/**
 * Pre-bundle help markdown docs into a static JSON file for Cloudflare Workers.
 */

import fs from 'fs'
import path from 'path'
import matter from 'gray-matter'
import { remark } from 'remark'
import html from 'remark-html'

interface DocMetadata {
  title: string
  description?: string
  section?: string
  category?: string
  categoryLabel?: string
  categoryOrder?: number
  sidebar?: {
    order?: number
  }
}

interface DocFile {
  slug: string[]
  metadata: DocMetadata
  content: string
  html: string
  codeBlocks: Array<{ lang: string; code: string }>
  tableOfContents: Array<{ id: string; text: string; level: number }>
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

  const source = String(args.get('--source') || 'content/help')
  const lang = String(args.get('--lang') || 'en')
  const out = args.get('--out')

  return { source, lang, out }
}

function slugify(value: string): string {
  return value
    .toLowerCase()
    .trim()
    .replace(/[`*_~()[\]{}:;,.!?'"<>]/g, '')
    .replace(/&/g, 'and')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

function extractTableOfContents(content: string) {
  const items: Array<{ id: string; text: string; level: number }> = []
  const headingPattern = /^(#{2,3})\s+(.+)$/gm
  let match: RegExpExecArray | null

  while ((match = headingPattern.exec(content)) !== null) {
    const text = match[2].replace(/<[^>]+>/g, '').trim()
    items.push({
      id: slugify(text),
      text,
      level: match[1].length,
    })
  }

  return items
}

async function convertToHtml(content: string): Promise<string> {
  try {
    const result = await remark()
      .use(html, { sanitize: false })
      .process(content)
    return result.toString()
  } catch (error) {
    console.error('Error converting markdown:', error)
    return '<p>Error rendering content</p>'
  }
}

const { source, lang, out } = parseArgs(process.argv)
const docsDirectory = path.join(process.cwd(), source)
const outputPath = out
  ? path.join(process.cwd(), String(out))
  : path.join(
      process.cwd(),
      'lib',
      lang === 'en' ? 'bundled-docs.json' : `bundled-docs.${lang}.json`
    )

async function getAllDocs(): Promise<DocFile[]> {
  const docs: DocFile[] = []

  async function readDir(dir: string, slugParts: string[] = []) {
    if (!fs.existsSync(dir)) return
    const files = fs.readdirSync(dir)

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

        const codeBlocks: Array<{ lang: string; code: string }> = []
        const contentWithPlaceholders = content.replace(/```(\w+)?\n([\s\S]*?)```/g, (_match, blockLang, code) => {
          const index = codeBlocks.length
          codeBlocks.push({ lang: blockLang || 'text', code: code.trim() })
          return `\n\n<div class="code-block-placeholder" data-index="${index}"></div>\n\n`
        })

        docs.push({
          slug,
          metadata: {
            title: data.title || fileName,
            description: data.description,
            section: data.section,
            category: data.category,
            categoryLabel: data.categoryLabel,
            categoryOrder: data.categoryOrder,
            sidebar: data.sidebar,
          },
          content,
          html: await convertToHtml(contentWithPlaceholders),
          codeBlocks,
          tableOfContents: extractTableOfContents(content),
        })
      }
    }
  }

  await readDir(docsDirectory)
  return docs
}

console.log(`📦 Bundling docs (${lang})...`)
const docs = await getAllDocs()
const bundleContent = JSON.stringify(docs, null, 2)

fs.writeFileSync(outputPath, bundleContent, 'utf8')

console.log(`✅ Bundled ${docs.length} docs to ${outputPath}`)
console.log(`📊 Bundle size: ${(bundleContent.length / 1024).toFixed(2)} KB`)
