import { GildCard, GildPage, Diagram } from './components'

export const metadata = {
  title: 'Gild — Agent Sandbox | Deka',
  description:
    'Gild is the Firecracker microVM sandbox used for Tana agent work.',
}

const sections = [
  {
    title: 'The gild OS userland',
    href: '/docs/gild/userland',
    description:
      'Rootfs, agent CLIs, init path, auth injection, host table, and filesystem shape.',
  },
  {
    title: 'Threat model + boundaries',
    href: '/docs/gild/threat-model',
    description:
      'Current enforced boundaries, rollout gaps, and planned hardening for VM isolation, users, auth, mounts, boot args, and egress.',
  },
  {
    title: 'Audit subsystem',
    href: '/docs/gild/audit',
    description:
      'The local audit database, runs API, Prometheus metrics, and guard-rail work that is still landing.',
  },
]

export default function GildOverviewPage() {
  return (
    <GildPage
      title="Gild — Agent Sandbox"
      description="Gild is the hardware-rooted sandbox path for Tana agent work. The fleet rollout is in progress while the rootfs readonly-fs flake is fixed."
    >
      <p>
        Gild is Tana&apos;s agent sandbox. It is designed to run each agent
        task inside a Firecracker microVM with a Debian bookworm-derived
        userland, a scrubbed environment, an audit log, and a network policy
        that denies broad internal reachability by default.
      </p>

      <p>
        Rollout status: the target is for all agent dispatchers to run with{' '}
        <code>AGENT_WORKER_SANDBOX=gild</code>. Today, 10 of 11 dispatchers
        are temporarily pinned to <code>AGENT_WORKER_SANDBOX=host</code> while
        the readonly-fs sandbox flake in tana#239 is fixed. Samira is the
        current gild-mode dispatcher. Full sandbox mode returns once tana#79
        and tana#239 land.
      </p>

      <p>
        It exists because agents are useful only if their blast radius is
        small. A coding agent needs a checkout, package managers, runtimes, and
        enough network to fetch public dependencies or talk to the git server.
        It does not need merchant databases, production service networks,
        dispatcher host files, host Docker privileges, or arbitrary access to
        the machine it landed on. Those are the boundaries gild is being
        rolled out to enforce for the dispatcher fleet.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Why it exists
      </h2>
      <p>
        On 2026-05-10, an agent shutdown incident exposed the weakness of the
        older model: agent work was isolated by convention and Unix user
        boundaries, but not by a hardware-backed VM boundary. The response was
        to make the safe path the normal path. Gild turns each task into a
        short-lived guest with a controlled filesystem, controlled network, and
        a local audit trail. During the rollout exception, agents still in host
        mode are not protected by those VM boundaries.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Threat model
      </h2>
      <p>
        Gild assumes agent prompts, generated code, package scripts, and model
        tool calls are not trusted. It also assumes an agent may try to inspect
        the host, reach internal services, persist credentials, or escape its
        assigned checkout. The sandbox is built to contain those behaviors
        before they become access to Tana or merchant systems; some hardening
        items are still tracked as rollout blockers.
      </p>

      <Diagram>{`merchant data / production networks
        ^
        | denied in gild mode
        |
host -> Firecracker microVM -> /workspace
          gild OS userland      allowed work area
          audit + metrics
          egress filters`}</Diagram>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Section map
      </h2>
      <div className="grid md:grid-cols-2 gap-4 not-prose">
        {sections.map((section) => (
          <GildCard key={section.href} title={section.title} href={section.href}>
            {section.description}
          </GildCard>
        ))}
      </div>
    </GildPage>
  )
}
