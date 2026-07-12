import { Diagram, GildPage } from '../components'

export const metadata = {
  title: 'Gild vault | Deka',
  description:
    'Reference for the gild vault hardware-rooted secrets daemon, namespaces, recovery model, and shop-secrets roadmap.',
}

export default function HararPage() {
  return (
    <GildPage
      slug="vault"
      title="Gild vault"
      description="Hardware-rooted secrets for Tana services, agents, and future merchant shops."
      lastUpdated="2026-05-14"
    >
      <div className="not-prose border border-primary/40 bg-primary/10 rounded-md p-4 text-sm text-foreground">
        <strong>v1 status:</strong> platform secrets only. Agent-scoped and
        shop-scoped secrets are designed but not shipped here; the shop-secrets
        phase is tracked in{' '}
        <a className="underline" href="https://admin.tana.gg/issues/236">
          tana#236
        </a>
        .
      </div>

      <p>
        Gild vault is the host-local secrets daemon introduced by{' '}
        <a href="https://admin.tana.gg/issues/235">tana#235</a>. It replaces
        plaintext <code>.env</code> files with an authenticated local service
        that unseals secret material from machine hardware at boot, keeps the
        database key in daemon memory, and serves only the secrets a caller is
        allowed to read.
      </p>

      <p>
        On macOS hosts the seal is rooted in Apple Secure Enclave. On Linux it
        is rooted in firmware TPM 2.0 / Intel PTT when that host is enabled.
        Each machine has its own hardware recipient. A service no longer reads
        a file from disk; it asks the daemon over a protected Unix socket, and
        the daemon derives the caller&apos;s principal from socket credentials plus
        a scoped bearer token before checking ACLs and writing an audit row.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Architecture
      </h2>
      <Diagram>{`                 host machine

  launchd/systemd
       |
       v
  harar daemon
       |  unseal DB key at start
       v
  SEP / PTT hardware seal
       |
       v
  encrypted SQLite secrets store

  services and dispatchers
       |
       |  Unix socket + SO_PEERCRED + bearer token
       v
  /run/harar/sock
       |
       |  load/read/list/write, ACL check, audit write
       v
  harar daemon
       |
       +--> process env for approved service command
       +--> audit row for allowed and denied reads`}</Diagram>

      <p>
        The normal service path is <code>gild vault load &lt;service&gt; -- &lt;cmd&gt;</code>.
        The client fetches the service&apos;s allowed keys from the daemon, sets
        them in the child process environment, and execs the command. The goal
        is that operational wrappers stop sourcing plaintext files and fail
        closed if the vault is unreachable.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Namespaces
      </h2>
      <p>
        Vault uses namespace prefixes as the authorization boundary. The daemon
        should never trust a caller-supplied principal embedded in a path; it
        binds the caller first, then checks whether that principal may read the
        requested namespace.
      </p>

      <ul className="list-disc pl-6 space-y-2">
        <li>
          <code>platform/*</code> stores internal service secrets for Tana and
          Deka infrastructure. This is the v1 shipped scope.
        </li>
        <li>
          <code>agent/*</code> is reserved for per-agent dispatcher
          credentials. It is part of the vault namespace model, but remains
          follow-up work after the platform path.
        </li>
        <li>
          <code>shop/&lt;shop_id&gt;/*</code> is the merchant secrets namespace.
          It is tracked in <a href="https://admin.tana.gg/issues/236">tana#236</a>
          and depends on the shop runtime identity and dashboard work.
        </li>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Recovery layers
      </h2>
      <p>
        The recovery model is self-managed. There is no external cloud KMS in
        the v1 design. Each layer is additive: routine operation should use the
        hardware seal, while deeper recovery paths cover machine loss and
        wider fleet failures.
      </p>

      <ol className="list-decimal pl-6 space-y-2">
        <li>
          <strong>Hardware seal:</strong> the daemon unseals locally through
          SEP or PTT during normal host operation.
        </li>
        <li>
          <strong>Cross-fleet replication:</strong> encrypted vault bundles are
          replicated to peer machines so one dead host does not destroy the
          only copy.
        </li>
        <li>
          <strong>Diceware passphrase:</strong> a separate passphrase recovery
          artifact exists for multi-machine failure. It is not mixed into the
          hardware age recipient set.
        </li>
        <li>
          <strong>YubiKey:</strong> planned as an additional recipient path
          when the keys arrive. Do not treat YubiKeys as part of the current
          deployed v1.
        </li>
        <li>
          <strong>BIP-39 paper:</strong> planned last-resort paper recovery for
          the same vault root generation, with no service or shop names written
          on the paper.
        </li>
      </ol>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Merchant secrets preview
      </h2>
      <p>
        The merchant-facing phase will expose shop secrets through a
        Vercel-style environment variables screen: add, edit, rotate, delete,
        masked values, last-rotated timestamps, and read audit history. Shop
        owners administer <code>shop/&lt;shop_id&gt;/*</code>; storefront
        isolates read only their own shop namespace during warmup.
      </p>

      <p>
        That dashboard is not part of vault v1. Until tana#236 lands, this page
        should be read as the platform vault reference plus the public shape of
        the upcoming shop-secrets work.
      </p>
    </GildPage>
  )
}
