import Link from 'next/link'
import { DocBreadcrumbs } from '@/components/docs/DocBreadcrumbs'

export function GildPage({
  slug,
  title,
  description,
  children,
}: {
  slug?: string
  title: string
  description: string
  children: React.ReactNode
}) {
  return (
    <div className="max-w-5xl mx-auto px-8 py-12">
      <article className="prose prose-invert max-w-none">
        <DocBreadcrumbs
          items={[
            { label: 'docs', href: '/docs' },
            { label: 'gild', href: '/docs/gild' },
            ...(slug ? [{ label: slug }] : []),
          ]}
        />

        <p className="not-prose text-sm text-muted-foreground mb-3">
          Last updated 2026-05-13
        </p>
        <h1 className="text-4xl font-bold text-foreground mb-2 not-prose">
          {title}
        </h1>
        <p className="text-xl text-muted-foreground mb-8 not-prose">
          {description}
        </p>

        <div className="space-y-5 text-foreground/90 leading-relaxed">
          {children}
        </div>
      </article>
    </div>
  )
}

export function GildCard({
  title,
  href,
  children,
}: {
  title: string
  href: string
  children: React.ReactNode
}) {
  return (
    <Link
      href={href}
      className="not-prose block border border-border rounded-lg p-5 hover:border-primary/60 hover:bg-card transition-colors"
    >
      <h3 className="text-lg font-semibold text-foreground mb-2">{title}</h3>
      <p className="text-sm text-muted-foreground">{children}</p>
    </Link>
  )
}

export function DefendedClaim({
  claim,
  children,
}: {
  claim: string
  children: React.ReactNode
}) {
  return (
    <li>
      <strong>{claim}.</strong> {children}
    </li>
  )
}

export function Diagram({ children }: { children: string }) {
  return (
    <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">
      {children}
    </pre>
  )
}
