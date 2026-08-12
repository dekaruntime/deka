#!/usr/bin/env node
/**
 * Publish docs from the core repo into the website content tree.
 *
 * Usage:
 *   node scripts/publish-docs.js --manual docs/phpx --scan . --out ../deka-website/content/docs
 *
 * Flags:
 *   --manual   Source directories for hand-written docs (comma-separated, default: docs/phpx)
 *   --scan     Directory to scan for docid comments (default: .)
 *   --map      Doc routing map (default: docs/docmap.json)
 *   --examples Directory for example files (default: examples)
 *   --sections Allowed doc sections (comma-separated, default: phpx)
 *   --version  Docs version marker in frontmatter (default: latest)
 *   --require-module-docs Validate all exported php_modules APIs have docid blocks (default: true)
 *   --no-require-module-docs Disable php_modules doc validation
 *   --out      Output directory inside deka-website (required)
 *   --force    Overwrite existing output files
 *   --dry-run  Print planned writes without touching disk
 */

const fs = require('fs')
const path = require('path')

function parseArgs(argv) {
  const args = new Map()
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

  return {
    manual: args.get('--manual') || 'docs/phpx',
    scan: args.get('--scan') || '.',
    map: args.get('--map') || 'docs/docmap.json',
    examples: args.get('--examples') || 'examples',
    sections: args.get('--sections') || 'phpx',
    version: args.get('--version') || 'latest',
    requireModuleDocs: !Boolean(args.get('--no-require-module-docs')),
    out: args.get('--out'),
    force: Boolean(args.get('--force')),
    dryRun: Boolean(args.get('--dry-run')),
  }
}

function ensureDir(dir, dryRun) {
  if (dryRun) return
  fs.mkdirSync(dir, { recursive: true })
}

function parseFrontmatter(content) {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/)
  if (!match) {
    return { frontmatter: null, body: content }
  }
  return {
    frontmatter: match[1],
    body: content.slice(match[0].length),
  }
}

function serializeFrontmatter(lines, body) {
  const front = lines.join('\n').trimEnd()
  return `---\n${front}\n---\n${body}`
}

function ensureFrontmatter(content, additions) {
  const { frontmatter, body } = parseFrontmatter(content)
  const lines = frontmatter ? frontmatter.split(/\r?\n/) : []
  const hasKey = (key) => lines.some((line) => line.trim().startsWith(`${key}:`))

  if (additions.category && !hasKey('category')) {
    lines.push(`category: "${additions.category}"`)
  }
  if (additions.categoryLabel && !hasKey('categoryLabel')) {
    lines.push(`categoryLabel: "${additions.categoryLabel}"`)
  }
  if (typeof additions.categoryOrder === 'number' && !hasKey('categoryOrder')) {
    lines.push(`categoryOrder: ${additions.categoryOrder}`)
  }
  if (additions.version && !hasKey('version')) {
    lines.push(`version: "${additions.version}"`)
  }

  if (frontmatter) {
    return serializeFrontmatter(lines, body)
  }

  if (lines.length === 0) {
    return content
  }

  return serializeFrontmatter(lines, content)
}

function deriveManualMeta(rootDir, filePath) {
  const relPath = path.relative(rootDir, filePath)
  const parts = relPath.split(path.sep)
  if (parts.length < 2) return null
  const section = parts[0]
  const fileName = path.basename(filePath).replace(/\.mdx?$/, '')

  const category = parts.length >= 3 ? parts[1] : fileName
  return { section, category }
}

function copyManualDocs(sourceDir, outDir, dryRun, docMap, allowedSections, version, rootDir = sourceDir) {
  if (!fs.existsSync(sourceDir)) {
    console.warn(`Manual docs directory not found: ${sourceDir}`)
    return
  }

  const entries = fs.readdirSync(sourceDir, { withFileTypes: true })
  for (const entry of entries) {
    const src = path.join(sourceDir, entry.name)
    const dest = path.join(outDir, entry.name)
    if (entry.isDirectory()) {
      copyManualDocs(src, dest, dryRun, docMap, allowedSections, version, rootDir)
    } else if (entry.name.endsWith('.md') || entry.name.endsWith('.mdx')) {
      if (dryRun) {
        console.log(`[dry-run] copy ${src} -> ${dest}`)
      } else {
        const meta = deriveManualMeta(rootDir, src)
        if (meta) {
          if (allowedSections.size && !allowedSections.has(meta.section)) {
            continue
          }
          const content = fs.readFileSync(src, 'utf8')
          const categoryMeta = getCategoryMeta(meta.section, meta.category, docMap)
          const updated = ensureFrontmatter(content, {
            category: meta.category,
            categoryLabel: categoryMeta.categoryLabel,
            categoryOrder: categoryMeta.categoryOrder,
            version,
          })
          ensureDir(path.dirname(dest), dryRun)
          fs.writeFileSync(dest, updated, 'utf8')
          continue
        }
        ensureDir(path.dirname(dest), dryRun)
        fs.copyFileSync(src, dest)
      }
    }
  }
}

function listFiles(rootDir) {
  const entries = fs.readdirSync(rootDir, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    if (entry.name === 'node_modules' || entry.name === '.git' || entry.name === 'target') {
      continue
    }
    const entryPath = path.join(rootDir, entry.name)
    if (entry.isSymbolicLink()) {
      continue
    }
    if (entry.isDirectory()) {
      files.push(...listFiles(entryPath))
    } else {
      files.push(entryPath)
    }
  }
  return files
}

function slugifyName(name) {
  return name
    .replace(/[^a-zA-Z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .toLowerCase()
}

function collectExamples(rootDir) {
  if (!rootDir || !fs.existsSync(rootDir)) return new Map()
  const files = listFiles(rootDir)
  const examples = new Map()

  for (const filePath of files) {
    const fileName = path.basename(filePath)
    const match = fileName.match(/^(.+)\.example\.([a-zA-Z0-9]+)$/)
    if (!match) continue

    const baseName = match[1]
    const language = match[2].toLowerCase()
    const code = fs.readFileSync(filePath, 'utf8')
    const relPath = path.relative(rootDir, filePath)
    const relParts = relPath.split(path.sep)

    if (relParts.length < 3) {
      console.warn(`Skipping example outside section/category: ${relPath}`)
      continue
    }

    const section = relParts[0]
    const category = relParts[1]
    const slug = slugifyName(baseName)
    const key = `${section}/${category}/${slug}`
    const entry = { baseName, language, code, filePath, key }

    if (!examples.has(key)) {
      examples.set(key, [])
    }
    examples.get(key).push(entry)
  }

  return examples
}

function loadDocMap(mapPath) {
  if (!mapPath) return null
  if (!fs.existsSync(mapPath)) return null
  try {
    const raw = fs.readFileSync(mapPath, 'utf8')
    return JSON.parse(raw)
  } catch (error) {
    console.warn(`Failed to read doc map at ${mapPath}: ${error.message}`)
    return null
  }
}

function normalizeDocId(value) {
  return value.trim()
}

function resolveDocId(raw, docMap) {
  const normalized = normalizeDocId(raw)
  if (!docMap || !docMap.aliases) return normalized
  if (docMap.aliases[normalized]) return docMap.aliases[normalized]
  const withoutParens = normalized.replace(/\(\)$/, '')
  if (docMap.aliases[withoutParens]) return docMap.aliases[withoutParens]
  return normalized
}

function getCategoryMeta(section, category, docMap) {
  const defaults = docMap && docMap.defaults ? docMap.defaults[section] : null
  const prefixMeta = docMap && docMap.prefixes ? docMap.prefixes[`${section}/${category}`] : null
  const isDefaultCategory = Boolean(defaults && defaults.category === category)
  const categoryLabel = prefixMeta && prefixMeta.categoryLabel
    ? prefixMeta.categoryLabel
    : (isDefaultCategory && defaults && defaults.categoryLabel ? defaults.categoryLabel : null)

  let categoryOrder = prefixMeta && typeof prefixMeta.categoryOrder === 'number'
    ? prefixMeta.categoryOrder
    : null

  if (categoryOrder === null && docMap && docMap.categoryOrder) {
    const orderSpec = docMap.categoryOrder[section]
    if (Array.isArray(orderSpec)) {
      const index = orderSpec.indexOf(category)
      if (index >= 0) categoryOrder = index
    } else if (orderSpec && typeof orderSpec === 'object' && orderSpec[category] !== undefined) {
      const value = Number(orderSpec[category])
      if (!Number.isNaN(value)) categoryOrder = value
    }
  }

  if (categoryOrder === null && isDefaultCategory && defaults && typeof defaults.categoryOrder === 'number') {
    categoryOrder = defaults.categoryOrder
  }

  return { categoryLabel, categoryOrder }
}

function parseDocId(raw, docMap) {
  const resolved = resolveDocId(raw, docMap)
  const parts = resolved.split('/').filter(Boolean)
  if (parts.length < 2) return null

  const section = parts[0]
  let category = parts.length >= 3 ? parts[1] : null
  const namePart = parts.length >= 3 ? parts.slice(2).join('/') : parts.slice(1).join('/')

  if (!category) {
    const defaults = docMap && docMap.defaults ? docMap.defaults[section] : null
    category = defaults && defaults.category ? defaults.category : 'general'
  }

  const name = namePart.replace(/\(\)$/, '')
  const slug = slugifyName(name)

  const { categoryLabel, categoryOrder } = getCategoryMeta(section, category, docMap)

  return { section, category, name, slug, categoryLabel, categoryOrder }
}

function extractDocBlocks(filePath) {
  const lines = fs.readFileSync(filePath, 'utf8').split(/\r?\n/)
  const docs = []
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i]
    const match = line.match(/^\s*\/\/\/\s*docid:\s*([^\s]+)\s*$/)
    if (!match) continue

    const docid = match[1]
    const body = []
    for (let j = i + 1; j < lines.length; j += 1) {
      const nextLine = lines[j]
      const docMatch = nextLine.match(/^\s*\/\/\/\s?(.*)$/)
      if (!docMatch) break
      body.push(docMatch[1])
      i = j
    }

    docs.push({ docid, body })
  }
  return docs
}

function resolveExamples(parsed, examplesMap) {
  if (!examplesMap) return []

  const key = `${parsed.section}/${parsed.category}/${parsed.slug}`
  return examplesMap.get(key) || []
}

function extractDescription(bodyLines) {
  if (!Array.isArray(bodyLines) || bodyLines.length === 0) return ''

  const descriptionStart = bodyLines.findIndex((line) => /<Description(?:\s|>)/.test(line))
  if (descriptionStart >= 0) {
    const parts = []
    let inBlock = false
    for (let i = descriptionStart; i < bodyLines.length; i += 1) {
      const line = bodyLines[i]
      if (!inBlock) {
        const match = line.match(/<Description(?:\s[^>]*)?>(.*)$/)
        if (match) {
          inBlock = true
          const after = match[1] || ''
          if (after.includes('</Description>')) {
            parts.push(after.split('</Description>')[0])
            break
          }
          parts.push(after)
        }
        continue
      }

      if (line.includes('</Description>')) {
        parts.push(line.split('</Description>')[0])
        break
      }

      parts.push(line)
    }

    const raw = parts.join(' ')
    const cleaned = raw.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim()
    if (cleaned) return cleaned
  }

  const fallbackLine = bodyLines.find((line) => {
    const trimmed = line.trim()
    if (!trimmed) return false
    if (trimmed.startsWith('<Function')) return false
    if (trimmed.startsWith('</Function')) return false
    return true
  })

  return fallbackLine ? fallbackLine.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim() : ''
}

function writeDoc(outRoot, filePath, doc, force, dryRun, docMap, examplesMap, allowedSections, version) {
  const parsed = parseDocId(doc.docid, docMap)
  if (!parsed) {
    console.warn(`Skipping invalid docid: ${doc.docid} (${filePath})`)
    return
  }
  if (allowedSections.size && !allowedSections.has(parsed.section)) {
    return
  }

  const outDir = path.join(outRoot, parsed.section, parsed.category)
  const outFile = path.join(outDir, `${parsed.slug}.mdx`)

  if (fs.existsSync(outFile) && !force) {
    console.warn(`Skipping existing doc: ${outFile}`)
    return
  }

  const description = extractDescription(doc.body)
  const frontmatter = [
    '---',
    `title: "${parsed.name}"`,
    `docid: "${doc.docid}"`,
    `section: "${parsed.section}"`,
    `category: "${parsed.category}"`,
    `version: "${version}"`,
    parsed.categoryLabel ? `categoryLabel: "${parsed.categoryLabel}"` : null,
    typeof parsed.categoryOrder === 'number' ? `categoryOrder: ${parsed.categoryOrder}` : null,
    `source: "${path.relative(process.cwd(), filePath)}"`,
    description ? `description: "${description.replace(/"/g, '\\"')}"` : null,
    '---',
    '',
  ]
    .filter((line) => line !== null && line !== undefined)
    .join('\n')

  const content = doc.body.length ? doc.body.join('\n') : 'TODO: add documentation content.'
  const examples = resolveExamples(parsed, examplesMap)
  let exampleSection = ''

  if (examples.length) {
    const blocks = examples
      .sort((a, b) => a.filePath.localeCompare(b.filePath))
      .map((example) => {
        return [
          `\`\`\`${example.language}`,
          example.code.trimEnd(),
          '```',
          '',
        ].join('\n')
      })
    exampleSection = ['## Examples', '', ...blocks].join('\n')
  }

  const outputParts = [frontmatter, content, exampleSection].filter(Boolean)
  const output = `${outputParts.join('\n\n')}\n`

  if (dryRun) {
    console.log(`[dry-run] write ${outFile}`)
    return
  }

  ensureDir(outDir, dryRun)
  fs.writeFileSync(outFile, output, 'utf8')
}

function isInternalModuleDocSource(filePath, scanRoot) {
  const rel = path.relative(scanRoot, filePath).split(path.sep).join('/')
  if (!rel.startsWith('php_modules/')) return false
  return rel.startsWith('php_modules/_') || rel.startsWith('php_modules/@') || rel.includes('/.cache/')
}

function extractCommentDocs(
  scanRoot,
  outRoot,
  force,
  dryRun,
  docMap,
  examplesMap,
  allowedSections,
  version
) {
  const files = listFiles(scanRoot)
  const seen = new Set()
  for (const filePath of files) {
    if (filePath.endsWith('.md') || filePath.endsWith('.mdx')) {
      continue
    }
    if (isInternalModuleDocSource(filePath, scanRoot)) {
      continue
    }
    const docs = extractDocBlocks(filePath)
    for (const doc of docs) {
      const resolved = resolveDocId(doc.docid, docMap)
      const section = resolved.split('/').filter(Boolean)[0] || ''
      if (allowedSections.size && !allowedSections.has(section)) {
        continue
      }
      if (seen.has(resolved)) {
        console.warn(`Duplicate docid detected: ${resolved}`)
      }
      seen.add(resolved)
      writeDoc(
        outRoot,
        filePath,
        doc,
        force,
        dryRun,
        docMap,
        examplesMap,
        allowedSections,
        version
      )
    }
  }
}


function isPublicModuleFile(filePath, scanRoot) {
  const rel = path.relative(scanRoot, filePath).split(path.sep).join('/')
  if (!rel.startsWith('php_modules/')) return false
  if (!rel.endsWith('.phpx')) return false
  if (rel.endsWith('.d.phpx')) return false
  if (rel.includes('/.cache/')) return false
  return true
}

function validateModuleDocCoverage(scanRoot) {
  const files = listFiles(scanRoot)
  const missing = []

  for (const filePath of files) {
    if (!isPublicModuleFile(filePath, scanRoot)) continue
    const lines = fs.readFileSync(filePath, 'utf8').split(/\r?\n/)

    for (let i = 0; i < lines.length; i += 1) {
      const match = lines[i].match(/^\s*export\s+function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/)
      if (!match) continue

      const fnName = match[1]
      let j = i - 1
      while (j >= 0 && lines[j].trim() === '') j -= 1

      let hasDocId = false
      while (j >= 0 && /^\s*\/\/\//.test(lines[j])) {
        if (/^\s*\/\/\/\s*docid:\s*[^\s]+\s*$/.test(lines[j])) {
          hasDocId = true
        }
        j -= 1
      }

      if (!hasDocId) {
        missing.push(`${filePath}:${i + 1} export function ${fnName}`)
      }
    }
  }

  if (missing.length > 0) {
    const preview = missing.slice(0, 50)
    const rest = missing.length - preview.length
    const details = preview.join('\n')
    const suffix = rest > 0 ? `\n... and ${rest} more` : ''
    throw new Error(
      `Missing php_modules doccomments (/// docid) on exported functions:\n${details}${suffix}\n\n` +
      `Add a doc block immediately above each export, e.g.:\n` +
      `/// docid: phpx/<category>/<name>()\n` +
      `/// <Function name=\"<name>\"> ... </Function>`
    )
  }
}


function main() {
  const options = parseArgs(process.argv)
  if (!options.out) {
    console.error('Missing required --out argument')
    process.exit(1)
  }

  const manualRoots = String(options.manual)
    .split(',')
    .map((value) => value.trim())
    .filter(Boolean)
    .map((value) => path.resolve(value))
  const scanRoot = path.resolve(options.scan)
  const outRoot = path.resolve(options.out)
  const mapPath = options.map ? path.resolve(options.map) : null
  const docMap = mapPath ? loadDocMap(mapPath) : null
  const examplesRoot = options.examples ? path.resolve(options.examples) : null
  const examplesMap = collectExamples(examplesRoot)
  const allowedSections = new Set(
    String(options.sections)
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean)
  )
  const version = String(options.version).trim() || 'latest'

  console.log(`Publishing docs to ${outRoot}`)
  for (const manualRoot of manualRoots) {
    const section = path.basename(manualRoot)
    const isSectionRoot = section === 'php' || section === 'phpx' || section === 'js'
    if (isSectionRoot && allowedSections.size && !allowedSections.has(section)) {
      continue
    }
    const manualOutRoot = isSectionRoot ? path.join(outRoot, section) : outRoot
    const metaRoot = isSectionRoot ? path.dirname(manualRoot) : manualRoot
    copyManualDocs(
      manualRoot,
      manualOutRoot,
      options.dryRun,
      docMap,
      allowedSections,
      version,
      metaRoot
    )
  }
  extractCommentDocs(
    scanRoot,
    outRoot,
    options.force,
    options.dryRun,
    docMap,
    examplesMap,
    allowedSections,
    version
  )
}

main()
