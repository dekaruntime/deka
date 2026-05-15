import type { Metadata } from 'next'
import Link from 'next/link'
import { Navbar } from '@/components/landing/Navbar'

export const metadata: Metadata = {
  title: 'Merchant Onboarding | Deka Documentation',
  description: 'Expected merchant sign-up, shop setup, storefront, checkout, and fulfillment flow for Tana merchants.',
}

const steps = [
  {
    title: '1. Sign up + IdP',
    status: '🚧 in progress — see #244',
    expected: [
      'A merchant starts at tana.gg and chooses the merchant sign-up path.',
      'Email sign-up creates an account, starts a session, and records the merchant as the shop owner.',
      'OAuth sign-in uses NextAuth provider selection, completes the provider callback, and lands the merchant in the same account creation path.',
    ],
    progress: [
      'Issue #244 found dev sign-up working through POST /api/signup.',
      'Production deka.gg NextAuth currently returns HTTP 500 before provider selection, so full browser OAuth is not verified yet.',
    ],
  },
  {
    title: '2. Shop creation',
    status: 'Expected behavior',
    expected: [
      'The merchant selects a subdomain that becomes the public shop handle.',
      'The platform creates the tenant directory and shop records for that handle.',
      'Redis maps the selected subdomain to the tenant so {shop}.tana.gg requests route to the correct storefront.',
    ],
    progress: [
      'Issue #244 verified dev shop creation returned a shopId, subdomain, accountId, and owner session for the smoke shop.',
    ],
  },
  {
    title: '3. Merchant admin',
    status: 'Expected behavior',
    expected: [
      'The merchant signs in to store.tana.gg with the owner account.',
      'The admin dashboard loads the merchant shop context, including shop name, shop ID, owner email, and role.',
      'The dashboard provides the starting point for products, payment setup, orders, and fulfillment.',
    ],
    progress: [
      'Issue #244 verified the dev merchant admin rendered the dashboard and /api/me returned owner shop context.',
    ],
  },
  {
    title: '4. Add a product',
    status: '🚧 in progress — see #244',
    expected: [
      'The merchant creates a product with name, description, price, and at least one image.',
      'Image upload stores the asset in T4 and returns a browser-reachable URL suitable for public storefront rendering.',
      'Saving the product creates a SKU and publishes product data to the shop catalog.',
    ],
    progress: [
      'Issue #244 verified product upload and product creation work through the same API path used by the Add Product dialog.',
      'Uploaded product image URLs are currently serialized as http://localhost:9500/... on storefront pages, so public storefront browsers cannot load them.',
      'Product deletion for smoke cleanup currently returns HTTP 405 and is not wired yet.',
    ],
  },
  {
    title: '5. Configure payments',
    status: '🚧 in progress — see #244',
    expected: [
      'The merchant starts Stripe Connect onboarding from the admin payment settings.',
      'Test mode keeps smoke and QA flows on Stripe test accounts and test cards only.',
      'Live mode requires completed Connect onboarding before the shop can accept real customer payments.',
    ],
    progress: [
      'Issue #244 verified the Stripe Connect start endpoint returned a Stripe-hosted onboarding URL and accountId.',
      'The smoke shop remained chargesEnabled=false and onboardingCompleted=false, so a completed test-mode Connect path is still missing.',
    ],
  },
  {
    title: '6. Storefront live',
    status: '🚧 in progress — see #244',
    expected: [
      '{shop}.tana.gg renders the tenant storefront for the selected subdomain.',
      'The default theme shows shop identity, product cards, descriptions, prices, and product links without merchant customization.',
      'Product media loads from public asset URLs, not from localhost or internal service origins.',
    ],
    progress: [
      'Issue #244 verified the smoke storefront returned HTTP 200 and showed seeded plus smoke-created product text and prices.',
      'Storefront product image rendering is still blocked by localhost asset URLs from the upload path.',
    ],
  },
  {
    title: '7. First sale',
    status: '🚧 in progress — see #244',
    expected: [
      'A customer browses the storefront, opens a product, adds it to the cart, and proceeds to checkout.',
      'Checkout uses the merchant payment configuration and reaches Stripe Checkout or an equivalent card-entry flow.',
      'A successful paid checkout creates an order visible in merchant admin.',
    ],
    progress: [
      'Issue #244 verified cart add worked over HTTPS and rendered the product, quantity, subtotal, total, and checkout link.',
      'Checkout currently returns HTTP 503 before Stripe test-card entry because the smoke shop has no completed test-mode Connect path.',
      'Order creation is not verified yet because checkout does not reach payment completion; /api/orders returned an empty list after the blocked checkout attempt.',
    ],
  },
  {
    title: '8. Fulfillment',
    status: '🚧 in progress — see #244',
    expected: [
      'Paid orders appear in merchant admin with customer, line item, payment, and shipping details.',
      'The merchant can mark an order as shipped.',
      'Fulfillment status moves through clear states such as pending, paid, shipped, and fulfilled.',
    ],
    progress: [
      'Issue #244 could not verify fulfillment because no paid order exists while checkout is blocked.',
      'The orders page rendered an empty state, so fulfillment controls still need verification with a seeded or paid test order.',
    ],
  },
]

export default function MerchantOnboardingDocsPage() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <Navbar />
      <main className="mx-auto max-w-5xl px-8 py-12">
        <div className="space-y-10">
          <div className="space-y-3">
            <div className="text-sm text-muted-foreground">
              <Link href="/docs" className="hover:text-foreground">docs</Link>
              <span className="px-2">/</span>
              <span>merchants</span>
            </div>
            <h1 className="text-4xl font-bold text-foreground">Merchant onboarding flow</h1>
            <p className="max-w-3xl text-lg text-muted-foreground">
              This is the canonical sign-up to storefront reference for Tana merchants. It describes what should work
              and marks the gaps tracked by the MVP merchant smoke audit in tana#244.
            </p>
          </div>

          <div className="rounded-lg border border-amber-300/60 bg-amber-50 px-5 py-4 text-sm text-amber-950">
            Items marked <strong>🚧 in progress — see #244</strong> are expected parts of the merchant journey that are
            either broken, blocked, or not fully verifiable in the current smoke environment.
          </div>

          <section className="space-y-6">
            {steps.map((step) => (
              <article key={step.title} className="border-b border-border pb-8 last:border-b-0">
                <div className="flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
                  <h2 className="text-2xl font-semibold text-foreground">{step.title}</h2>
                  <span className="rounded-md border border-border bg-secondary/50 px-3 py-1 text-sm text-muted-foreground">
                    {step.status}
                  </span>
                </div>

                <div className="mt-4 grid gap-5 md:grid-cols-2">
                  <div>
                    <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
                      Expected behavior
                    </h3>
                    <ul className="mt-3 list-disc space-y-2 pl-5 text-sm leading-6 text-foreground">
                      {step.expected.map((item) => (
                        <li key={item}>{item}</li>
                      ))}
                    </ul>
                  </div>

                  <div>
                    <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
                      Current note
                    </h3>
                    <ul className="mt-3 list-disc space-y-2 pl-5 text-sm leading-6 text-muted-foreground">
                      {step.progress.map((item) => (
                        <li key={item}>{item}</li>
                      ))}
                    </ul>
                  </div>
                </div>
              </article>
            ))}
          </section>

          <section className="rounded-lg border border-border bg-secondary/30 p-6">
            <h2 className="text-xl font-semibold text-foreground">In-progress links</h2>
            <ul className="mt-4 list-disc space-y-2 pl-5 text-sm text-muted-foreground">
              <li>tana#244: production NextAuth returns HTTP 500 before provider selection.</li>
              <li>tana#244: product uploads publish localhost asset URLs to storefront HTML.</li>
              <li>tana#244: checkout returns HTTP 503 before Stripe test-card entry.</li>
              <li>tana#244: order creation and fulfillment are not verifiable until checkout is unblocked.</li>
              <li>tana#244: product cleanup/delete returns HTTP 405.</li>
            </ul>
          </section>
        </div>
      </main>
    </div>
  )
}
