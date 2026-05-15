'use client'

import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { VersionSelect } from '@/components/docs/VersionSelect'
import { useDocsVersion } from '@/components/docs/useDocsVersion'
import { LanguageSelect } from '@/components/docs/LanguageSelect'

interface CLISidebarProps {
  isOpen: boolean
  basePath?: string
}

const cliTools = [
  {
    category: 'Services',
    items: [
      { name: 'Overview', slug: 'overview', description: 'Installation and quick start' },
      { name: 'setup', slug: 'setup', description: 'First-run configuration' },
      { name: 'start', slug: 'start', description: 'Start services' },
      { name: 'stop', slug: 'stop', description: 'Stop services and volumes' },
      { name: 'restart', slug: 'restart', description: 'Restart services' },
      { name: 'status', slug: 'status', description: 'Health and service status' },
      { name: 'logs', slug: 'logs', description: 'Inspect service output' },
      { name: 'check', slug: 'check', description: 'Check for updates' },
      { name: 'update', slug: 'update', description: 'Pull latest images' },
      { name: 'upgrade', slug: 'upgrade', description: 'Upgrade and cleanup' },
      { name: 'monitor', slug: 'monitor', description: 'Automated update checks' },
    ]
  },
  {
    category: 'Containers',
    items: [
      { name: 'ps', slug: 'ps', description: 'List containers' },
      { name: 'run', slug: 'run', description: 'Create a container' },
      { name: 'c run', slug: 'c-run', description: 'Create and start a container' },
      { name: 'exec', slug: 'exec', description: 'Run a command' },
      { name: 'attach', slug: 'attach', description: 'Attach to a container' },
      { name: 'rm', slug: 'rm', description: 'Remove a container' },
    ]
  },
  {
    category: 'Users',
    items: [
      { name: 'whoami', slug: 'whoami', description: 'Show current user' },
      { name: 'user add', slug: 'user-add', description: 'Add a user' },
      { name: 'user list', slug: 'user-list', description: 'List users' },
      { name: 'user remove', slug: 'user-remove', description: 'Remove a user' },
    ]
  },
  {
    category: 'Repos',
    items: [
      { name: 'repo list', slug: 'repo-list', description: 'List repositories' },
      { name: 'repo view', slug: 'repo-view', description: 'View repository details' },
      { name: 'repo delete', slug: 'repo-delete', description: 'Delete a repository' },
      { name: 'repo import', slug: 'repo-import', description: 'Import from GitHub' },
    ]
  },
  {
    category: 'T4 Storage',
    items: [
      { name: 't4 buckets', slug: 't4-buckets', description: 'List buckets' },
      { name: 't4 create', slug: 't4-create', description: 'Create a bucket' },
      { name: 't4 delete', slug: 't4-delete', description: 'Delete a bucket' },
      { name: 't4 ls', slug: 't4-ls', description: 'List objects' },
      { name: 't4 put', slug: 't4-put', description: 'Upload an object' },
      { name: 't4 get', slug: 't4-get', description: 'Download an object' },
      { name: 't4 rm', slug: 't4-rm', description: 'Delete an object' },
    ]
  },
  {
    category: 'Agent',
    items: [
      { name: 'agent setup', slug: 'agent-setup', description: 'Configure deka-agent' },
      { name: 'agent status', slug: 'agent-status', description: 'Show agent status' },
    ]
  }
]

export function CLISidebar({ isOpen, basePath }: CLISidebarProps) {
  const pathname = usePathname()
  const { version } = useDocsVersion()
  const resolvedBasePath = basePath
    ? (version && version !== 'latest' ? `${basePath}/${version}` : basePath)
    : '/cli'

  return (
    <aside
      className={`${
        isOpen ? 'block' : 'hidden'
      } md:block w-64 overflow-y-auto scrollbar-hide border-r border-border/30 mx-auto md:mx-0 relative z-30`}
    >
      <nav className="p-4 space-y-6">
        <VersionSelect />
        {cliTools.map((section, sectionIndex) => (
          <div key={sectionIndex}>
            <h3 className="text-sm font-semibold text-foreground mb-2 uppercase tracking-wide">
              {section.category}
            </h3>
            <ul className="space-y-1">
              {section.items.map((tool) => {
                const href = `${resolvedBasePath}/${tool.slug}`
                const isActive = pathname === href
                return (
                  <li key={tool.slug}>
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
                      <span className="block truncate">{tool.name}</span>
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
