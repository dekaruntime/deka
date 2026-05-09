import Link from 'next/link'
import { DocBreadcrumbs } from '@/components/docs/DocBreadcrumbs'

export const metadata = {
  title: 'Platform | Deka',
  description:
    'How Tana is put together: agents, dispatchers, runtime, and the deploy pipeline.',
}

const sections = [
  {
    title: 'Agents',
    href: '/docs/platform/agents',
    description:
      'Personas with durable identity. Each agent has a unix user, a persistent dispatcher, and ephemeral workers spawned per task.',
  },
]

export default function PlatformOverviewPage() {
  return (
    <div className="max-w-5xl mx-auto px-8 py-12">
      <article className="max-w-none">
        <DocBreadcrumbs
          items={[
            { label: 'docs', href: '/docs' },
            { label: 'platform' },
          ]}
        />
        <h1 className="text-4xl font-bold text-foreground mb-2">Platform</h1>
        <p className="text-xl text-muted-foreground">
          How Tana is put together. The platform section documents the
          long-running components that sit underneath PHP, PHPX, and the CLI.
        </p>

        <div className="border-l-4 border-primary pl-4 py-2 mt-6">
          <p className="text-muted-foreground">
            The platform is young. Pages here describe the system as it runs
            today on demon, not an aspirational future shape. When the shape
            changes, the docs change with it.
          </p>
        </div>

        <div className="grid md:grid-cols-2 gap-6 pt-8">
          {sections.map((section) => (
            <Link
              key={section.href}
              href={section.href}
              className="border border-border rounded-lg p-6 hover:border-primary/60 hover:bg-card transition-colors"
            >
              <h3 className="text-lg font-semibold text-foreground mb-2">
                {section.title}
              </h3>
              <p className="text-sm text-muted-foreground">
                {section.description}
              </p>
            </Link>
          ))}
        </div>
      </article>
    </div>
  )
}
