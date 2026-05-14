import { Diagram, GildPage } from '../components'

export const metadata = {
  title: 'The gild OS userland | Deka',
  description:
    'The Debian bookworm-derived userland used inside gild microVMs.',
}

export default function GildUserlandPage() {
  return (
    <GildPage
      slug="userland"
      title="The gild OS userland"
      description="The guest image gives agents the tools they need to work, then removes the host-shaped assumptions they do not need."
    >
      <p>
        The gild guest starts from a Debian bookworm-slim root filesystem. The
        image adds the agent runtimes used by Tana dispatch: Node 22, Bun, and
        the installed <code>claude-code</code>, <code>codex</code>, and{' '}
        <code>opencode</code> CLIs. The image is intentionally small enough to
        boot quickly but complete enough that normal repository work does not
        need host bind mounts.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Boot path
      </h2>
      <p>
        Gild uses a custom <code>init.sh</code> and <code>init.js</code> inside
        the guest. The shell init performs early mount and environment setup.
        The JavaScript init receives the dispatch payload, prepares the
        workspace, injects runtime auth, and launches the selected agent CLI.
        Guest audit callback wiring is still landing in the real Firecracker
        path.
      </p>

      <Diagram>{`Firecracker
  |
  +-- gild kernel + rootfs
       |
       +-- /init.sh
            |
            +-- init.js
                 |
                 +-- audit callback path (in progress)
                 +-- auth injection
                 +-- agent CLI
                 +-- /workspace`}</Diagram>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Vsock bridge
      </h2>
      <p>
        The guest uses <code>socat</code> as the vsock bridge between the
        microVM and the host-side supervisor. The bridge carries the bounded
        control channel. It is not a general-purpose host network escape.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Auth injection
      </h2>
      <p>
        Authentication material is injected at task start, scoped to the
        runtime that needs it, and scrubbed from the broader environment. The
        agent gets enough bearer auth to perform the accepted dispatch. It does
        not inherit the host user&apos;s shell, login session, or ambient secrets.
        Audit-token injection for sandboxed cross-owner review is still tracked
        as follow-up work.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Fleet hostnames
      </h2>
      <p>
        The guest owns a hosts table for the fleet hostnames agents are allowed
        to resolve. This keeps code and tooling that refer to fleet names
        stable while the network policy still blocks broad internal egress.
        The special exception is <code>phobos:9418</code>, the git server path
        agents need for repository work.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Filesystem boundary
      </h2>
      <p>
        The intended writable work area is <code>/workspace</code>. Outside
        tmpfs work areas, the guest rootfs is attached read-only. Package
        managers and agent CLIs can write inside guest task scratch space, but
        cannot rewrite the guest base image. Host workspace and artifact
        attachment in the real Firecracker client is still a rollout blocker.
      </p>

      <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">{`/                  read-only guest root
/usr/bin           agent runtimes and tools
/etc/hosts         fleet hostname table
/workspace         writable checkout and task state`}</pre>
    </GildPage>
  )
}
