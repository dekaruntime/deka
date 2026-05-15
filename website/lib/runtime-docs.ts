import bundledRuntimeDocs from './bundled-runtime.json'

export type RuntimeLanguage = 'php' | 'phpx'

export interface RuntimeDocFile {
  section: RuntimeLanguage | string
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

export interface RuntimeSidebarItem {
  name: string
  slug: string
  description?: string
}

export interface RuntimeSidebarSection {
  category: string
  categorySlug: string
  items: RuntimeSidebarItem[]
}

const runtimeDocs = bundledRuntimeDocs as RuntimeDocFile[]

function getCategory(doc: RuntimeDocFile): string {
  if (doc.metadata.categoryLabel) return doc.metadata.categoryLabel
  if (doc.metadata.category) return doc.metadata.category
  if (doc.slug.length > 1) return doc.slug[0]
  return 'General'
}

function getCategoryOrder(doc: RuntimeDocFile): number | null {
  if (typeof doc.metadata.categoryOrder === 'number') return doc.metadata.categoryOrder
  return null
}

function getSortScore(label: string): number {
  const value = label.toLowerCase()
  if (value.includes('intro') || value.includes('overview')) return 0
  if (value.includes('getting started')) return 0
  return 1
}

export function getRuntimeDoc(
  language: RuntimeLanguage,
  slug: string[],
  docs: RuntimeDocFile[] = runtimeDocs
): RuntimeDocFile | null {
  const doc = docs.find((item) =>
    item.section === language &&
    item.slug.length === slug.length &&
    item.slug.every((part, index) => part === slug[index])
  )
  return doc || null
}

export function getAllRuntimeDocSlugs(
  language: RuntimeLanguage,
  docs: RuntimeDocFile[] = runtimeDocs
): string[][] {
  return docs
    .filter((doc) => doc.section === language)
    .map((doc) => doc.slug)
}

function isCategoryIndex(doc: RuntimeDocFile) {
  return doc.slug.length === 2 && doc.slug[1] === 'index'
}

export function getRuntimeSidebar(
  language: RuntimeLanguage,
  docs: RuntimeDocFile[] = runtimeDocs
): RuntimeSidebarSection[] {
  const docsForLanguage = docs.filter((doc) => doc.section === language && doc.slug.length > 0)
  const categoryMap = new Map<string, RuntimeSidebarItem[]>()
  const categoryOrder = new Map<string, number>()
  const categoryLabels = new Map<string, string>()

  for (const doc of docsForLanguage) {
    const categorySlug = doc.metadata.category || doc.slug[0]
    const category = getCategory(doc)
    const slugPath = doc.slug.join('/')
    const name = doc.metadata.title || doc.slug[doc.slug.length - 1]
    const description = doc.metadata.description
    const order = getCategoryOrder(doc)

    if (!categoryMap.has(categorySlug)) {
      categoryMap.set(categorySlug, [])
    }

    categoryLabels.set(categorySlug, category)

    categoryMap.get(categorySlug)!.push({
      name: isCategoryIndex(doc) ? 'Overview' : name,
      slug: isCategoryIndex(doc) ? categorySlug : slugPath,
      description,
    })

    if (typeof order === 'number') {
      const existing = categoryOrder.get(categorySlug)
      if (existing === undefined || order < existing) {
        categoryOrder.set(categorySlug, order)
      }
    }
  }

  const sections = Array.from(categoryMap.entries()).map(([categorySlug, items]) => {
    items.sort((a, b) => {
      const scoreA = getSortScore(a.name)
      const scoreB = getSortScore(b.name)
      if (scoreA !== scoreB) return scoreA - scoreB
      return a.name.localeCompare(b.name)
    })

    return {
      category: categoryLabels.get(categorySlug) || categorySlug,
      categorySlug,
      items,
    }
  })

  sections.sort((a, b) => {
    const orderA = categoryOrder.get(a.categorySlug)
    const orderB = categoryOrder.get(b.categorySlug)
    if (orderA !== undefined || orderB !== undefined) {
      const safeA = orderA ?? 999
      const safeB = orderB ?? 999
      if (safeA !== safeB) return safeA - safeB
    }
    const scoreA = getSortScore(a.category)
    const scoreB = getSortScore(b.category)
    if (scoreA !== scoreB) return scoreA - scoreB
    return a.category.localeCompare(b.category)
  })

  return sections
}

export function getRuntimeCategoryItems(
  language: RuntimeLanguage,
  category: string,
  docs: RuntimeDocFile[] = runtimeDocs
) {
  return docs
    .filter((doc) => doc.section === language)
    .filter((doc) => {
      const categorySlug = doc.metadata.category || doc.slug[0]
      if (categorySlug !== category) return false
      if (isCategoryIndex(doc)) return false
      return doc.slug.length >= 2
    })
    .map((doc) => ({
      title: doc.metadata.title || doc.slug[doc.slug.length - 1],
      description: doc.metadata.description,
      slug: doc.slug.join('/'),
    }))
    .sort((a, b) => a.title.localeCompare(b.title))
}
