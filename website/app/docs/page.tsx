import Link from 'next/link'
import { Search } from 'lucide-react'
import { Navbar } from '@/components/landing/Navbar'

const docSections = [
  {
    title: 'PHPX Runtime',
    description: 'Reference documentation for the Deka PHPX language, syntax, and standard library.',
    href: '/docs/phpx'
  },
  {
    title: 'PHP Runtime',
    description: 'Reference documentation for the Deka PHP runtime APIs and extensions.',
    href: '/docs/php'
  },
  {
    title: 'Deka CLI',
    description: 'Command-line workflows for managing services and infrastructure.',
    href: '/docs/cli'
  },
  {
    title: 'Deploy Server',
    description: 'Operate and secure the Deka deploy server in production.',
    href: '/docs/deploy'
  },
  {
    title: 'Platform',
    description: 'How Tana is put together: agents, dispatchers, and the shape of the system.',
    href: '/docs/platform'
  }
]

const commonQuestions = [
  {
    title: 'How do I install the Deka CLI?',
    href: '/docs/cli/overview'
  },
  {
    title: 'Where do I start with the PHPX runtime?',
    href: '/docs/phpx/overview'
  },
  {
    title: 'How do I configure the deploy server?',
    href: '/docs/deploy/intro'
  }
]

export default function DocsLandingPage() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <Navbar />
      <main className="mx-auto max-w-5xl px-8 py-16">
        <div className="space-y-10">
          <div className="space-y-3">
            <h1 className="text-4xl font-bold text-foreground">Deka documentation</h1>
            <p className="text-xl text-muted-foreground">
              Find runtime references, CLI workflows, and deploy server guidance in one place.
            </p>
          </div>

          <div className="max-w-3xl">
            <div className="relative">
              <Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
              <input
                type="search"
                placeholder="Search docs, runtime APIs, and deploy guides..."
                className="w-full rounded-xl border border-border bg-secondary/40 py-3 pl-11 pr-4 text-sm text-foreground placeholder:text-muted-foreground focus:border-primary focus:outline-none"
              />
            </div>
          </div>

          <section className="space-y-4">
            <h2 className="text-xl font-semibold text-foreground">Documentation sections</h2>
            <div className="grid gap-4 md:grid-cols-2">
              {docSections.map((section) => (
                <Link
                  key={section.title}
                  href={section.href}
                  className="rounded-2xl border border-border bg-card/80 p-6 transition-colors hover:border-primary/60 hover:bg-card"
                >
                  <h3 className="text-lg font-semibold text-foreground mb-2">{section.title}</h3>
                  <p className="text-sm text-muted-foreground">{section.description}</p>
                </Link>
              ))}
            </div>
          </section>

          <section className="space-y-4">
            <h2 className="text-xl font-semibold text-foreground">Common questions</h2>
            <div className="grid gap-3">
              {commonQuestions.map((question) => (
                <Link
                  key={question.title}
                  href={question.href}
                  className="rounded-xl border border-border bg-secondary/30 px-4 py-3 text-sm text-foreground transition-colors hover:border-primary/60 hover:bg-secondary/40"
                >
                  {question.title}
                </Link>
              ))}
            </div>
          </section>
        </div>
      </main>
    </div>
  )
}
