import { DefendedClaim, GildPage } from '../components'

export const metadata = {
  title: 'Threat model + boundaries | Deka',
  description:
    'Current and planned isolation boundaries for the gild agent sandbox.',
}

export default function GildThreatModelPage() {
  return (
    <GildPage
      slug="threat-model"
      title="Threat model + boundaries"
      description="Gild treats agent code and model-driven tool use as untrusted, then narrows what a task can see, execute, mount, and reach."
    >
      <p>
        The claims below separate the boundaries already present in the merged
        runtime from hardening that is still blocked or planned. They are
        written as operational claims, not slogans: each one names the control
        and the behavior it prevents.
      </p>

      <ul className="list-disc pl-6 space-y-3">
        <DefendedClaim claim="VM boundary">
          In gild mode, a task runs inside a Firecracker microVM, so process
          isolation is enforced below the host userspace boundary. Most
          dispatchers are temporarily in host mode while tana#239 is fixed, so
          this boundary does not currently cover the whole fleet.
        </DefendedClaim>
        <DefendedClaim claim="Dedicated gild user">
          The host supervisor runs as the <code>gild</code> user, which is not
          in the <code>docker</code> group and does not inherit agent users&apos;
          broader permissions.
        </DefendedClaim>
        <DefendedClaim claim="Environment scrub">
          The guest receives a narrow environment assembled for the task rather
          than the host&apos;s ambient shell variables.
        </DefendedClaim>
        <DefendedClaim claim="Bearer auth">
          Dispatch calls use bearer authentication so the control plane accepts
          only requests with a valid task token.
        </DefendedClaim>
        <DefendedClaim claim="Constant-time compare">
          Token comparison avoids timing-based early exits when checking
          authorization material.
        </DefendedClaim>
        <DefendedClaim claim="Replay hardening">
          Dispatch replay-window enforcement is planned hardening. Current
          protected routes use bearer authentication and constant-time token
          comparison, but this page does not claim a shipped replay window.
        </DefendedClaim>
        <DefendedClaim claim="Firecracker jailer">
          Jailer-based process confinement is part of the defense-in-depth
          target, but the current runtime client launches Firecracker directly.
          Treat jailer confinement as planned until that path lands.
        </DefendedClaim>
        <DefendedClaim claim="boot_args lockdown">
          Kernel and guest boot arguments are fixed by the supervisor, not by
          the agent payload.
        </DefendedClaim>
        <DefendedClaim claim="Mount validation">
          Workspace mount paths are validated before boot. The real
          Firecracker client still needs the configured workspace and artifact
          attachment path before gild can be re-enabled broadly.
        </DefendedClaim>
        <DefendedClaim claim="Network egress filter">
          The checked-in policy allows internet egress, denies Tailscale,
          RFC1918, and multicast ranges, and keeps <code>phobos:9418</code> as
          the narrow internal exception for git. Live rule-table verification
          is tracked separately by infra.
        </DefendedClaim>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        What gild does not claim
      </h2>
      <p>
        Gild does not make generated code safe. It contains the code while it
        runs. A malicious package script can still fail the task, delete files
        inside <code>/workspace</code>, or attempt blocked network access. The
        intended boundary is that those actions stay inside the task&apos;s
        workspace and audit trail. During host-mode rollout exceptions, those
        VM filesystem and egress boundaries do not apply to that dispatcher.
      </p>
    </GildPage>
  )
}
