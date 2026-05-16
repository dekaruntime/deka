'use client'

import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { VersionSelect } from '@/components/docs/VersionSelect'
import { useDocsVersion } from '@/components/docs/useDocsVersion'
import { LanguageSelect } from '@/components/docs/LanguageSelect'

interface APISidebarProps {
  isOpen: boolean
  basePath?: string
}

const apiSections = [
  {
    category: 'Overview',
    items: [
      { name: 'Introduction', slug: 'intro', description: 'API overview and authentication' },
      { name: 'Rate Limiting', slug: 'rate-limit', description: 'API usage limits' },
    ]
  },
  {
    category: 'Authentication',
    items: [
      { name: 'Identity Service', slug: 'identity/index', description: 'QR code auth API' },
    ]
  },
  {
    category: 'Blockchain API',
    items: [
      { name: 'Users', slug: 'users/create', description: 'User management' },
      { name: 'User sign-in', slug: 'users/signin', description: 'User authentication' },
      { name: 'Balances', slug: 'balances/index', description: 'Balance queries' },
      { name: 'Transactions', slug: 'transactions/index', description: 'Transaction management' },
      { name: 'Blocks', slug: 'blocks/index', description: 'Block queries' },
      { name: 'Contracts', slug: 'contracts/index', description: 'Smart contracts' },
    ]
  },
  {
    category: 'Developer Tools',
    items: [
      { name: 'Keys', slug: 'keys/index', description: 'Cryptographic keys' },
      { name: 'Key details', slug: 'keys/id', description: 'Fetch a key by ID' },
      { name: 'Modules', slug: 'modules/kv', description: 'Storage modules' },
    ]
  }
]

export function APISidebar({ isOpen, basePath }: APISidebarProps) {
  const pathname = usePathname()
  const { version } = useDocsVersion()
  const resolvedBasePath = basePath
    ? (version && version !== 'latest' ? `${basePath}/${version}` : basePath)
    : '/api'

  return (
    <aside
      className={`${
        isOpen ? 'block' : 'hidden'
      } md:block w-64 overflow-y-auto scrollbar-hide border-r border-border/30 mx-auto md:mx-0 relative z-30`}
    >
      <nav className="p-4 space-y-6">
        <VersionSelect />
        {apiSections.map((section, sectionIndex) => (
          <div key={sectionIndex}>
            <h3 className="text-sm font-semibold text-foreground mb-2 uppercase tracking-wide">
              {section.category}
            </h3>
            <ul className="space-y-1">
              {section.items.map((item) => {
                const href = `${resolvedBasePath}/${item.slug}`
                const isActive = pathname === href
                return (
                  <li key={item.slug}>
                    <Link
                      href={href}
                      prefetch={true}
                      scroll={false}
                      className={`block px-3 py-1 rounded-lg text-sm transition-colors truncate ${
                        isActive
                          ? 'bg-primary/10 text-primary font-medium'
                          : 'text-muted-foreground hover:bg-secondary/50 hover:text-foreground'
                      }`}
                    >
                      <span className="block truncate">{item.name}</span>
                    </Link>
                  </li>
                )
              })}
            </ul>
          </div>
        ))}
        <div className="pt-4 border-t border-border/30">
          <LanguageSelect />
        </div>
      </nav>
    </aside>
  )
}
