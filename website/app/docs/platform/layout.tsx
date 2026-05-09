'use client'

import { useState } from 'react'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { Navbar } from '@/components/landing/Navbar'
import { Menu, Search } from 'lucide-react'
import { Button } from '@/components/ui/button'

const platformSections = [
  {
    category: 'Platform',
    items: [
      { name: 'Overview', slug: '', description: 'How Tana is structured' },
      { name: 'Agents', slug: 'agents', description: 'Personas, dispatchers, and ephemeral workers' },
    ],
  },
]

export default function DocsPlatformLayout({
  children,
}: {
  children: React.ReactNode
}) {
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const pathname = usePathname()

  return (
    <div className="h-screen bg-background text-foreground overflow-hidden flex flex-col">
      <Navbar />

      <div className="md:hidden sticky top-16 z-40 bg-background border-b border-border p-4 flex items-center justify-between">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setSidebarOpen(!sidebarOpen)}
        >
          <Menu className="w-5 h-5" />
        </Button>
        <div className="flex-1 mx-4">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
            <input
              type="text"
              placeholder="Search platform docs..."
              className="w-full pl-9 pr-4 py-2 bg-secondary/50 border border-border rounded-lg text-sm focus:outline-none focus:border-primary"
            />
          </div>
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">
        <aside
          className={`${
            sidebarOpen ? 'block' : 'hidden'
          } md:block w-64 shrink-0 border-r border-border bg-background overflow-y-auto`}
        >
          <nav className="p-4 space-y-6">
            {platformSections.map((section) => (
              <div key={section.category}>
                <div className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-2">
                  {section.category}
                </div>
                <ul className="space-y-1">
                  {section.items.map((item) => {
                    const href = item.slug
                      ? `/docs/platform/${item.slug}`
                      : '/docs/platform'
                    const active = pathname === href
                    return (
                      <li key={item.slug || 'overview'}>
                        <Link
                          href={href}
                          className={`block rounded-md px-2 py-1.5 text-sm ${
                            active
                              ? 'bg-secondary text-foreground'
                              : 'text-muted-foreground hover:bg-secondary/60 hover:text-foreground'
                          }`}
                        >
                          {item.name}
                        </Link>
                      </li>
                    )
                  })}
                </ul>
              </div>
            ))}
          </nav>
        </aside>

        <main className="flex-1 overflow-y-auto scrollbar-hide bg-secondary/30">
          {children}
        </main>
      </div>
    </div>
  )
}
