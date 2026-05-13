import { DefendedClaim, GildPage } from '../components'

export const metadata = {
  title: 'Threat model + boundaries | Deka',
  description:
    'The isolation and policy claims enforced by the gild agent sandbox.',
}

export default function GildThreatModelPage() {
  return (
    <GildPage
      slug="threat-model"
      title="Threat model + boundaries"
      description="Gild treats agent code and model-driven tool use as untrusted, then narrows what a task can see, execute, replay, mount, and reach."
    >
      <p>
        The claims below are the boundaries gild is designed to defend. They
        are written as operational claims, not slogans: each one names the
        control and the behavior it prevents.
      </p>

      <ul className="list-disc pl-6 space-y-3">
        <DefendedClaim claim="VM boundary">
          Each task runs inside a Firecracker microVM, so process isolation is
          enforced below the host userspace boundary.
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
        <DefendedClaim claim="Bounded replay">
          Dispatch timestamps and replay windows keep an old accepted request
          from becoming a reusable launch credential.
        </DefendedClaim>
        <DefendedClaim claim="boot_args lockdown">
          Kernel and guest boot arguments are fixed by the supervisor, not by
          the agent payload.
        </DefendedClaim>
        <DefendedClaim claim="Mount validation">
          Workspace mounts are validated before boot so the guest cannot be
          pointed at arbitrary host paths.
        </DefendedClaim>
        <DefendedClaim claim="Network egress filter">
          Egress passes four gates: internet access may be allowed, Tailscale
          ranges are denied, RFC1918 and multicast ranges are denied, and
          <code>phobos:9418</code> is the narrow internal exception for git.
        </DefendedClaim>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        What gild does not claim
      </h2>
      <p>
        Gild does not make generated code safe. It contains the code while it
        runs. A malicious package script can still fail the task, delete files
        inside <code>/workspace</code>, or attempt blocked network access. The
        boundary is that those actions stay inside the task&apos;s workspace and
        audit trail.
      </p>
    </GildPage>
  )
}
