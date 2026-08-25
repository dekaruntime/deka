import fs from 'fs'
import path from 'path'
import { fileURLToPath } from 'url'

export type HatsTestStatus = 'pass' | 'fail'

export type HatsTestStage = 'parse' | 'typecheck' | 'run'

export type HatsHost = 'native' | 'browser'

export interface HatsTest {
  slug: string
  category: string
  status: HatsTestStatus
  name: string
  source: string
  files?: Record<string, string>
  entryPath?: string
  title: string
  stage: HatsTestStage
  hosts: HatsHost[]
  expectedStdout?: string
  expectedStdoutNative?: string
  expectedStdoutBrowser?: string
  expectedCode?: string
  expectedDiagnosticContains?: string
  dekaJson?: Record<string, unknown>
  packages?: string[]
  notes?: string
}

export interface HatsCategory {
  name: string
  tests: HatsTest[]
}

const TESTS_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'testsuite')

function parseStatusFromFilename(filename: string): HatsTestStatus | null {
  if (filename.endsWith('.pass.ds')) return 'pass'
  if (filename.endsWith('.fail.ds')) return 'fail'
  return null
}

function baseNameFromFilename(filename: string): string {
  return filename.replace(/\.(pass|fail)\.ds$/, '')
}

function slugFromParts(category: string, name: string): string {
  return `${category}-${name}`.toLowerCase().replace(/[^a-z0-9]+/g, '-')
}

function readFile(dir: string, filename: string): string | undefined {
  const filePath = path.join(dir, filename)
  if (!fs.existsSync(filePath)) return undefined
  return fs.readFileSync(filePath, 'utf-8')
}

function collectDsFiles(dir: string, relativeTo: string): string[] {
  const results: string[] = []
  const entries = fs.readdirSync(dir, { withFileTypes: true })
  for (const entry of entries) {
    const relativePath = path.relative(relativeTo, path.join(dir, entry.name)).replace(/\\/g, '/')
    if (entry.isDirectory()) {
      results.push(...collectDsFiles(path.join(dir, entry.name), relativeTo))
    } else if (entry.isFile() && entry.name.endsWith('.ds')) {
      results.push(relativePath)
    }
  }
  return results
}

function parseHosts(raw: unknown): HatsHost[] {
  if (!Array.isArray(raw)) return ['native', 'browser']
  const hosts = raw.filter((item): item is HatsHost => item === 'native' || item === 'browser')
  return hosts.length > 0 ? hosts : ['native', 'browser']
}

function parsePackages(raw: unknown): string[] | undefined {
  if (!Array.isArray(raw)) return undefined
  const names = raw.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
  return names.length > 0 ? names : undefined
}

function readMetadata(dir: string, name: string): Partial<HatsTest> {
  const jsonPath = path.join(dir, `${name}.json`)
  if (!fs.existsSync(jsonPath)) return {}
  try {
    const raw = JSON.parse(fs.readFileSync(jsonPath, 'utf-8'))
    return {
      title: typeof raw.title === 'string' ? raw.title : undefined,
      stage: ['parse', 'typecheck', 'run'].includes(raw.stage) ? raw.stage : undefined,
      hosts: parseHosts(raw.hosts),
      expectedStdoutNative:
        typeof raw.expectedStdoutNative === 'string' ? raw.expectedStdoutNative : undefined,
      expectedStdoutBrowser:
        typeof raw.expectedStdoutBrowser === 'string' ? raw.expectedStdoutBrowser : undefined,
      expectedDiagnosticContains:
        typeof raw.expectedDiagnosticContains === 'string'
          ? raw.expectedDiagnosticContains
          : undefined,
      dekaJson:
        raw.dekaJson && typeof raw.dekaJson === 'object' && !Array.isArray(raw.dekaJson)
          ? (raw.dekaJson as Record<string, unknown>)
          : undefined,
      packages: parsePackages(raw.packages),
      notes: typeof raw.notes === 'string' ? raw.notes : undefined,
    }
  } catch {
    return {}
  }
}

export function loadAllTests(): HatsCategory[] {
  if (!fs.existsSync(TESTS_DIR)) return []

  const categories: HatsCategory[] = []
  const categoryEntries = fs.readdirSync(TESTS_DIR, { withFileTypes: true })

  for (const categoryEntry of categoryEntries) {
    if (!categoryEntry.isDirectory()) continue
    const categoryName = categoryEntry.name
    const categoryDir = path.join(TESTS_DIR, categoryName)
    const testEntries = fs.readdirSync(categoryDir, { withFileTypes: true })

    const tests: HatsTest[] = []
    for (const testEntry of testEntries) {
      if (!testEntry.isDirectory()) continue
      const testName = testEntry.name
      const testDir = path.join(categoryDir, testName)
      const dsFiles = collectDsFiles(testDir, testDir)
      const entryFile = dsFiles.find((f) => parseStatusFromFilename(f) && !f.includes('/'))
      if (!entryFile) continue

      const status = parseStatusFromFilename(entryFile)!
      const name = baseNameFromFilename(entryFile)
      const source = readFile(testDir, entryFile)
      if (source === undefined) continue

      const metadata = readMetadata(testDir, name)
      const expectedStdout = readFile(testDir, `${name}.stdout`)
      const expectedCode = readFile(testDir, `${name}.code`)

      const extraDsFiles = dsFiles.filter((f) => f !== entryFile)
      const filesRecord: Record<string, string> | undefined =
        extraDsFiles.length > 0
          ? Object.fromEntries(
              extraDsFiles
                .map((f) => [f, readFile(testDir, f)] as const)
                .filter(([, content]) => content !== undefined)
                .map(([f, content]) => [f, content as string])
            )
          : undefined

      tests.push({
        slug: slugFromParts(categoryName, testName),
        category: categoryName,
        status,
        name: testName,
        source,
        files: filesRecord,
        entryPath: entryFile,
        title: metadata.title ?? testName.replace(/_/g, ' '),
        stage: metadata.stage ?? 'run',
        hosts: metadata.hosts ?? ['native', 'browser'],
        expectedStdout,
        expectedStdoutNative: metadata.expectedStdoutNative,
        expectedStdoutBrowser: metadata.expectedStdoutBrowser,
        expectedCode,
        expectedDiagnosticContains: metadata.expectedDiagnosticContains,
        dekaJson: metadata.dekaJson,
        packages: metadata.packages,
        notes: metadata.notes,
      })
    }

    if (tests.length > 0) {
      tests.sort((a, b) => a.name.localeCompare(b.name))
      categories.push({ name: categoryName, tests })
    }
  }

  categories.sort((a, b) => a.name.localeCompare(b.name))
  return categories
}

export function loadTestBySlug(slug: string): HatsTest | undefined {
  for (const category of loadAllTests()) {
    const test = category.tests.find((t) => t.slug === slug)
    if (test) return test
  }
  return undefined
}

export function getAllSlugs(): string[] {
  return loadAllTests().flatMap((c) => c.tests.map((t) => t.slug))
}
