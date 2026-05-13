import { GildPage } from '../components'

export const metadata = {
  title: 'Gild vault | Deka',
  description: 'Placeholder for gild vault documentation.',
}

export default function GildVaultPage() {
  return (
    <GildPage
      slug="vault"
      title="Gild vault"
      description="Documentation coming once v1 lands."
    >
      <p>
        Gild vault is tracked in{' '}
        <a href="https://admin.tana.gg/issues/235">tana#235</a>. This page
        will stay a stub until the v1 daemon lands.
      </p>

      <p>
        The direction is hardware-rooted secrets storage sealed to each
        machine&apos;s Secure Enclave or TPM 2.0. The merchant-facing page will
        explain how secrets are stored, scoped, rotated, and audited once that
        behavior is stable.
      </p>

      <ul className="list-disc pl-6 space-y-2">
        <li>
          <code>platform/*</code> for Tana platform secrets.
        </li>
        <li>
          <code>agent/*</code> for agent-scoped credentials.
        </li>
        <li>
          <code>shop/&lt;shop_id&gt;/*</code> for merchant shop-scoped
          secrets.
        </li>
      </ul>
    </GildPage>
  )
}
