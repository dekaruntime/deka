import { DocBreadcrumbs } from '@/components/docs/DocBreadcrumbs'

export const metadata = {
  title: 'Stripe Connect | Deka',
  description:
    'How merchants connect Stripe to Tana, complete embedded onboarding, manage dashboard status, and account for fees.',
}

const feeRows = [
  {
    tier: 'free',
    percent: '1.0%',
    cap: '$2.00',
  },
  {
    tier: 'standard',
    percent: '0.5%',
    cap: '$1.00',
  },
  {
    tier: 'premium',
    percent: '0.25%',
    cap: '$0.50',
  },
  {
    tier: 'enterprise',
    percent: '0.0%',
    cap: '$0.00',
  },
]

export default function StripeConnectPage() {
  return (
    <div className="max-w-5xl mx-auto px-8 py-12">
      <article className="max-w-none">
        <DocBreadcrumbs
          items={[
            { label: 'docs', href: '/docs' },
            { label: 'platform', href: '/docs/platform' },
            { label: 'stripe-connect' },
          ]}
        />

        <h1 className="text-4xl font-bold text-foreground mb-2">
          Stripe Connect
        </h1>
        <p className="text-xl text-muted-foreground mb-8">
          Merchant onboarding, payment readiness, dashboard recovery, and fee
          accounting for Stripe Express accounts.
        </p>

        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Tana uses Stripe Connect Express for merchant card payments. Each
            shop connects its own Stripe account from the Tana admin dashboard.
            Tana creates the connected account, sends the merchant through
            Stripe onboarding, stores the resulting account id on the shop, and
            uses that account id when storefront checkout creates charges.
          </p>
          <p>
            Merchants should treat Stripe as the system of record for bank
            account verification, payout timing, processing fees, disputes, and
            tax forms. Tana shows connection status and shop-level payment
            records so staff know whether checkout is ready and what platform
            fee was applied to each sale.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Connect Stripe
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            A shop owner or admin starts from the store admin dashboard. The
            payments settings view calls{' '}
            <code>/api/payments/connect/stripe/start</code> with the current
            shop id, session id, and an allowed dashboard return URL.
          </p>
          <p>
            The payments module validates that return URL before contacting
            Stripe. It then issues a one-use OAuth state token bound to the
            shop and session, creates a Stripe Express account, and creates an
            Account Link of type <code>account_onboarding</code>.
          </p>
          <p>
            The response contains the Stripe hosted onboarding URL and the
            newly-created account id. The dashboard opens that URL inside the
            onboarding surface. If Stripe asks the merchant to leave Tana to
            finish identity, banking, or ownership details, Stripe still returns
            to the same validated callback URL when onboarding completes.
          </p>
        </section>

        <div className="border border-border rounded-lg bg-background/70 p-5 mt-6">
          <h3 className="text-lg font-semibold text-foreground mb-3">
            Flow summary
          </h3>
          <ol className="list-decimal pl-5 space-y-2 text-foreground/90 leading-relaxed">
            <li>Merchant clicks Connect Stripe in the dashboard.</li>
            <li>Tana validates the return URL and creates a CSRF state token.</li>
            <li>Tana creates a Stripe Express account for the shop.</li>
            <li>Tana creates a Stripe Account Link for onboarding.</li>
            <li>Merchant completes the embedded Stripe onboarding flow.</li>
            <li>Stripe redirects back with state and account id.</li>
            <li>
              Tana consumes the state token, fetches the Stripe account, and
              stores the connected account status on the shop.
            </li>
          </ol>
        </div>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Embedded Onboarding
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Onboarding is merchant-driven. Stripe owns the form fields and
            verification checks; Tana owns when the flow starts, where the
            merchant returns, and how the connected account is represented in
            the shop dashboard.
          </p>
          <p>
            If the merchant closes the flow early or Stripe needs additional
            information, the dashboard should show the account as connected but
            not charge-ready. The merchant can resume onboarding from the same
            payments settings view, which creates a fresh Account Link and a
            fresh state token.
          </p>
          <p>
            A completed redirect is not enough to enable checkout by itself.
            Tana checks Stripe account status and only treats the shop as ready
            when <code>charges_enabled</code> is true.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Dashboard Views
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            The merchant dashboard should make payment readiness visible in
            every place where it affects work.
          </p>
          <ul className="list-disc pl-6 space-y-2">
            <li>
              <strong>Payments settings:</strong> shows whether Stripe is not
              connected, onboarding is incomplete, charges are enabled, or the
              account needs attention. Actions are Connect, Resume onboarding,
              Refresh status, and Disconnect.
            </li>
            <li>
              <strong>Checkout readiness:</strong> blocks card checkout until
              Stripe status reports <code>charges_enabled</code>. If Stripe
              returns a disabled reason or currently due requirements, show
              those as the reason the shop cannot accept card payments.
            </li>
            <li>
              <strong>Orders and payments:</strong> lists the Stripe
              PaymentIntent id, amount, currency, payment status, connected
              account provider, and Tana platform fee for each charge.
            </li>
            <li>
              <strong>Refunds:</strong> allows owner/admin refunds only. Every
              refund needs a reason and writes a refund audit entry whether the
              attempt succeeds, fails, or is rejected.
            </li>
            <li>
              <strong>Accounting export:</strong> separates gross order amount,
              Stripe processing fees, Tana platform fees, refunds, and net
              payout reconciliation.
            </li>
          </ul>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Fee Accounting
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Tana passes the shop account id in the <code>Stripe-Account</code>
            header when creating a PaymentIntent. The customer charge lands on
            the connected account. Tana records its own platform fee on the
            charge and sends that value to Stripe as{' '}
            <code>application_fee_amount</code>.
          </p>
          <p>
            The platform fee is separate from Stripe processing fees. Stripe
            calculates and deducts its own processing fees according to the
            merchant&apos;s Stripe agreement. Tana calculates only the platform
            fee for the merchant&apos;s Tana tier.
          </p>
        </section>

        <div className="overflow-x-auto border border-border rounded-lg bg-background/70 mt-6">
          <table className="w-full text-sm">
            <thead className="bg-secondary/60 text-muted-foreground">
              <tr>
                <th className="text-left font-medium px-4 py-3">Tier</th>
                <th className="text-left font-medium px-4 py-3">
                  Tana platform fee
                </th>
                <th className="text-left font-medium px-4 py-3">Cap</th>
              </tr>
            </thead>
            <tbody>
              {feeRows.map((row) => (
                <tr key={row.tier} className="border-t border-border">
                  <td className="px-4 py-3 font-medium text-foreground">
                    {row.tier}
                  </td>
                  <td className="px-4 py-3 text-foreground/90">
                    {row.percent} of charge amount
                  </td>
                  <td className="px-4 py-3 text-foreground/90">{row.cap}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <section className="space-y-4 text-foreground/90 leading-relaxed mt-6">
          <p>
            Accounting should store all money values in cents. For each Stripe
            charge, persist the gross amount, currency, PaymentIntent id,
            connected account id, platform fee cents, status, and idempotency
            key. Do not infer Stripe processing fees from Tana&apos;s platform
            fee; reconcile provider fees from Stripe balance transactions or
            Stripe exports.
          </p>
          <p>
            Refund accounting follows the original charge. The refund record
            should include the refund id, original PaymentIntent id, amount,
            status, actor, shop id, and reason. Full or partial refunds should
            be visible beside the original order so the merchant can reconcile
            Tana records with Stripe payouts.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Operational Notes
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <ul className="list-disc pl-6 space-y-2">
            <li>
              Configure <code>STRIPE_SECRET_KEY</code> for API calls and{' '}
              <code>STRIPE_WEBHOOK_SECRET</code> for webhook verification.
            </li>
            <li>
              Keep onboarding return URLs allowlisted. Invalid return URLs are
              rejected before Tana contacts Stripe.
            </li>
            <li>
              Treat OAuth state tokens as one-use. A callback with missing or
              invalid state must not connect the shop.
            </li>
            <li>
              Use idempotency keys when creating charges so checkout retries do
              not double charge customers.
            </li>
            <li>
              Refresh account status after onboarding, before enabling
              checkout, and whenever Stripe reports account requirement
              changes.
            </li>
          </ul>
        </section>
      </article>
    </div>
  )
}
