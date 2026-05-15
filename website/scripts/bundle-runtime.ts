#!/usr/bin/env bun
/**
 * Pre-bundle runtime docs into a static JSON file
 * for Cloudflare Workers compatibility.
 *
 * Usage:
 *   bun scripts/bundle-runtime.ts --source content/docs --lang en
 *   bun scripts/bundle-runtime.ts --source content-i18n/es/docs --lang es
 */

import fs from 'fs'
import path from 'path'
import matter from 'gray-matter'
import { remark } from 'remark'
import remarkMdx from 'remark-mdx'
import html from 'remark-html'

interface RuntimeDocFile {
  section: string
  slug: string[]
  metadata: {
    title: string
    description?: string
    category?: string
    categoryLabel?: string
    categoryOrder?: number
    docid?: string
    source?: string
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

  const source = String(args.get('--source') || 'content/docs')
  const lang = String(args.get('--lang') || 'en')
  const out = args.get('--out')

  return { source, lang, out }
}

const { source, lang, out } = parseArgs(process.argv)
const docsDirectory = path.join(process.cwd(), source)
const outputPath = out
  ? path.join(process.cwd(), String(out))
  : path.join(
      process.cwd(),
      'lib',
      lang === 'en' ? 'bundled-runtime.json' : `bundled-runtime.${lang}.json`
    )

async function convertToHtml(content: string, sourcePath: string): Promise<string> {
  try {
    const result = await remark()
      .use(remarkMdx)
      .use(remarkFunctionBlocks)
      .use(html, { sanitize: false })
      .process(content)
    return result.toString()
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error)
    console.warn(`[bundle-runtime] mdx parse failed for ${sourcePath}; retrying as markdown (${reason})`)
    try {
      const fallback = await remark()
        .use(html, { sanitize: false })
        .process(content)
      return fallback.toString()
    } catch (fallbackError) {
      const fallbackReason = fallbackError instanceof Error ? fallbackError.message : String(fallbackError)
      console.error(`[bundle-runtime] markdown fallback failed for ${sourcePath}: ${fallbackReason}`)
      return '<p>Error rendering content</p>'
    }
  }
}

function remarkFunctionBlocks() {
  return (tree: { type: string; children?: any[] }) => {
    const walk = (node: any, parent: any = null, index: number | null = null) => {
      if (node && node.type === 'mdxJsxFlowElement' && node.name === 'Function') {
        const replacement = buildFunctionBlock(node)
        if (parent && typeof index === 'number') {
          parent.children[index] = replacement
        }
        return
      }

      if (node && Array.isArray(node.children)) {
        node.children.forEach((child: any, childIndex: number) =>
          walk(child, node, childIndex)
        )
      }
    }

    walk(tree)
  }
}

function buildFunctionBlock(node: any) {
  const name = getAttr(node, 'name') || 'Function'
  const parameters = []
  let returnType: { type: string; typeLink: string } | null = null
  let descriptionHtml = ''

  for (const child of node.children || []) {
    if (child.type !== 'mdxJsxFlowElement') continue
    if (child.name === 'Parameter') {
      parameters.push(parseParameter(child))
    } else if (child.name === 'ReturnType') {
      if (!returnType) {
        returnType = parseReturn(child)
      }
    } else if (child.name === 'Description') {
      if (!descriptionHtml) {
        descriptionHtml = renderInlineNodes(child.children || [])
      }
    }
  }

  if (!returnType || !returnType.type) {
    throw new Error(`Function "${name}" is missing a ReturnType.`)
  }

  const signatureHtml = `
    <div class="docs-function__signature">
      <span class="docs-function__signature-name">${escapeHtml(name)}</span>
      <span class="docs-function__signature-params">(${renderParamList(parameters)})</span>
      <span class="docs-function__signature-return">: ${renderType(returnType.type, returnType.typeLink)}</span>
    </div>
  `
  const signatureBlock = `
    <div class="docs-function__section">
      <div class="docs-function__label">Signature</div>
      ${signatureHtml}
    </div>
  `

  const descriptionBlock = descriptionHtml
    ? `
      <div class="docs-function__section">
        <div class="docs-function__label">Description</div>
        <div class="docs-function__description">${descriptionHtml}</div>
      </div>
    `
    : ''

  const paramsHtml = parameters.length
    ? `
      <div class="docs-function__section">
        <div class="docs-function__label">Parameters</div>
        <div class="docs-function__params">
          ${parameters.map(renderParameter).join('')}
        </div>
      </div>
    `
    : ''
  const returnHtml = `
    <div class="docs-function__section">
      <div class="docs-function__label">Return type</div>
      <div class="docs-function__return">
        ${renderType(returnType.type, returnType.typeLink)}
      </div>
    </div>
  `

  const htmlValue = `
    <div class="docs-function">
      ${signatureBlock}
      ${descriptionBlock}
      ${paramsHtml}
      ${returnHtml}
    </div>
  `

  return { type: 'html', value: htmlValue }
}

function parseParameter(node: any) {
  return {
    name: getAttr(node, 'name') || getAttr(node, 'param') || '',
    type: getAttr(node, 'type') || '',
    typeLink: getAttr(node, 'typeLink') || '',
    required: parseBoolAttr(node, 'required'),
    descriptionHtml: renderInlineNodes(node.children || []),
  }
}

function parseReturn(node: any) {
  return {
    type: getAttr(node, 'type') || '',
    typeLink: getAttr(node, 'typeLink') || '',
  }
}

function renderParameter(param: {
  name: string
  type: string
  typeLink: string
  required: boolean | null
  descriptionHtml: string
}) {
  const typeHtml = renderType(param.type, param.typeLink)
  const requirement =
    param.required === null
      ? ''
      : `<em class="docs-function__param-required">${param.required ? 'required' : 'optional'}</em>`
  const requirementSpacer = requirement ? ' ' : ''
  const description = param.descriptionHtml
    ? `<div class="docs-function__param-desc">${param.descriptionHtml}</div>`
    : ''

  return `
    <div class="docs-function__param">
      <div class="docs-function__param-head">
        <span class="docs-function__param-name">${escapeHtml(param.name)}</span>
        ${typeHtml}${requirementSpacer}${requirement}
      </div>
      ${description}
    </div>
  `
}

function renderType(type: string, typeLink: string) {
  if (!type) return ''
  if (typeLink) {
    return `<a class="docs-function__type" href="${escapeAttr(typeLink)}">${escapeHtml(type)}</a>`
  }
  return `<span class="docs-function__type">${escapeHtml(type)}</span>`
}

function renderParamList(params: Array<{ name: string; type: string; typeLink: string }>) {
  return params
    .map((param) => {
      const name = escapeHtml(param.name || '')
      const typeHtml = param.type ? renderType(param.type, param.typeLink) : ''
      if (name && typeHtml) {
        return `<span class="docs-function__signature-param"><span class="docs-function__signature-param-name">${name}</span> ${typeHtml}</span>`
      }
      if (typeHtml) {
        return typeHtml
      }
      return `<span class="docs-function__signature-param-name">${name}</span>`
    })
    .join(', ')
}

function getAttr(node: any, name: string) {
  const attr = (node.attributes || []).find((item: any) =>
    item && item.type === 'mdxJsxAttribute' && item.name === name
  )
  if (!attr) return ''
  if (typeof attr.value === 'string') return attr.value
  if (attr.value && typeof attr.value.value === 'string') return attr.value.value
  return ''
}

function parseBoolAttr(node: any, name: string): boolean | null {
  const value = getAttr(node, name)
  if (!value) return null
  if (value === 'true') return true
  if (value === 'false') return false
  return null
}

function extractText(nodes: any[]): string {
  let value = ''
  for (const node of nodes) {
    if (!node) continue
    if (node.type === 'text' && typeof node.value === 'string') {
      value += node.value
    } else if (Array.isArray(node.children)) {
      value += extractText(node.children)
    }
  }
  return value.replace(/\s+/g, ' ').trim()
}

function renderInlineNodes(nodes: any[]): string {
  return nodes
    .map((node) => {
      if (!node) return ''
      if (node.type === 'text') {
        return escapeHtml(node.value || '')
      }
      if (node.type === 'inlineCode') {
        return `<code>${escapeHtml(node.value || '')}</code>`
      }
      if (node.type === 'strong') {
        return `<strong>${renderInlineNodes(node.children || [])}</strong>`
      }
      if (node.type === 'emphasis') {
        return `<em>${renderInlineNodes(node.children || [])}</em>`
      }
      if (node.type === 'link') {
        const href = node.url || '#'
        return `<a href="${escapeAttr(href)}">${renderInlineNodes(node.children || [])}</a>`
      }
      if (Array.isArray(node.children)) {
        return renderInlineNodes(node.children)
      }
      return ''
    })
    .join('')
    .replace(/\s+/g, ' ')
    .trim()
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function escapeAttr(value: string) {
  return escapeHtml(value).replace(/"/g, '&quot;')
}

async function getAllRuntimeDocs(): Promise<RuntimeDocFile[]> {
  const docs: RuntimeDocFile[] = []

  if (!fs.existsSync(docsDirectory)) {
    return docs
  }

  async function readDir(dir: string, relParts: string[] = []) {
    const files = fs.readdirSync(dir).sort()

    for (const file of files) {
      const filePath = path.join(dir, file)
      const stat = fs.statSync(filePath)

      if (stat.isDirectory()) {
        await readDir(filePath, [...relParts, file])
        continue
      }

      if (!file.endsWith('.md') && !file.endsWith('.mdx')) continue

      const fileContents = fs.readFileSync(filePath, 'utf8')
      const { data, content } = matter(fileContents)

      const fileName = file.replace(/\.mdx?$/, '')
      const fullParts = [...relParts, fileName]

      if (fullParts.length < 2) {
        continue
      }

      const section = fullParts[0]
      const slug = fullParts.slice(1)

      const codeBlocks: Array<{lang: string, code: string}> = []
      content.replace(/```([a-zA-Z0-9_-]+)?\r?\n([\s\S]*?)```/g, (_match, lang, code) => {
        codeBlocks.push({ lang: lang || 'text', code: code.trim() })
        return ''
      })

      // Keep code fences in markdown so remark-html emits <pre><code> blocks.
      // CLIContent hydrates those blocks into Monaco editors with line numbers.
      const htmlContent = await convertToHtml(content, filePath)

      docs.push({
        section,
        slug,
        metadata: {
          title: data.title || fileName,
          description: data.description,
          category: data.category,
          categoryLabel: data.categoryLabel,
          categoryOrder: data.categoryOrder,
          docid: data.docid,
          source: data.source,
        },
        content,
        html: htmlContent,
        codeBlocks,
      })
    }
  }

  await readDir(docsDirectory)
  return docs
}

console.log(`📦 Bundling runtime docs (${lang})...`)
const docs = await getAllRuntimeDocs()
const bundleContent = JSON.stringify(docs, null, 2)

fs.writeFileSync(outputPath, bundleContent, 'utf8')

console.log(`✅ Bundled ${docs.length} runtime docs to ${outputPath}`)
console.log(`📊 Bundle size: ${(bundleContent.length / 1024).toFixed(2)} KB`)
