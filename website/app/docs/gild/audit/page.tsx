import { GildPage } from '../components'

export const metadata = {
  title: 'Audit subsystem | Deka',
  description:
    'The gild audit database, runs endpoint, metrics endpoint, and audit hardening status.',
}

export default function GildAuditPage() {
  return (
    <GildPage
      slug="audit"
      title="Audit subsystem"
      description="Gild records host-side run data that operators can inspect, query, and scrape."
    >
      <p>
        Gild writes an <code>audit.db</code> SQLite database on the host side.
        The database records run identity, agent identity, workspace metadata,
        lifecycle timestamps, exit status, and resource pressure. Guest-side
        guard-rail event delivery in the real Firecracker path is still being
        wired through, so violation coverage should be treated as in progress.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Schema shape
      </h2>
      <p>
        The schema is built around runs and violations. A run is the unit of
        execution: one accepted dispatch, one microVM boot, one agent CLI
        process, and one final outcome. Violation rows are the intended place
        to attach guard-rail events with the relevant path, command, or
        resource threshold as the real-VM callback path lands.
      </p>

      <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">{`runs
  id
  agent
  workspace
  runtime
  status
  started_at
  finished_at
  exit_code

violations
  id
  run_id
  type
  detail
  created_at`}</pre>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Runs endpoint
      </h2>
      <p>
        The <code>/runs</code> endpoint exposes recent run records for the
        dispatcher and admin surfaces. It is the operator-facing view of what
        launched, where it ran, and whether it passed. Guard-rail rows appear
        as that event path is enabled for the runtime.
      </p>

      <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">{`curl -sS http://localhost:<gild-port>/runs \\
  -H "Authorization: Bearer $TANA_GIT_TOKEN"`}</pre>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Metrics endpoint
      </h2>
      <p>
        The <code>/metrics</code> endpoint returns Prometheus-format metrics
        for scrape-based monitoring. Operators use it for run counts, failure
        rates, and pressure near configured resource caps. Violation counters
        are part of the audit surface, but should not be read as complete
        coverage until guest event delivery is enabled in the real VM path.
      </p>

      <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">{`# HELP gild_runs_total Total gild runs observed by the supervisor.
# TYPE gild_runs_total counter
gild_runs_total{status="ok"} 42

# HELP gild_guardrail_violations_total Guard-rail violations by type.
# TYPE gild_guardrail_violations_total counter
gild_guardrail_violations_total{type="outside-workspace"} 1`}</pre>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Guard-rail violation targets
      </h2>
      <ul className="list-disc pl-6 space-y-2">
        <li>
          <strong>forbidden-binary</strong>: the task attempted to execute a
          high-risk binary tracked by the guard-rail rule set.
        </li>
        <li>
          <strong>boundary</strong>: the task attempted to cross a configured
          sandbox boundary.
        </li>
        <li>
          <strong>outside-workspace</strong>: the task attempted to write or
          operate outside <code>/workspace</code>. Complete real-VM coverage
          depends on the pending guest audit callback wiring.
        </li>
        <li>
          <strong>near-memory-cap</strong>: the guest approached its configured
          memory limit before completion.
        </li>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Admin dashboard
      </h2>
      <p>
        The admin dashboard surfaces gild status next to agent dispatcher
        status at{' '}
        <a href="https://admin.tana.gg/agents">admin.tana.gg/agents</a>. The
        page is the human view for recent runs and failed launches. Guard-rail
        pressure belongs there once violation event delivery is complete.
      </p>
    </GildPage>
  )
}
