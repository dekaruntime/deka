import { GildPage } from '../components'

export const metadata = {
  title: 'Audit subsystem | Deka',
  description:
    'The gild audit database, runs endpoint, metrics endpoint, and guard-rail violations.',
}

export default function GildAuditPage() {
  return (
    <GildPage
      slug="audit"
      title="Audit subsystem"
      description="Every gild run leaves a local record that operators can inspect, query, and scrape."
    >
      <p>
        Gild writes an <code>audit.db</code> SQLite database on the host side.
        The database records run identity, agent identity, workspace metadata,
        lifecycle timestamps, exit status, resource pressure, and guard-rail
        violations observed while the guest was running.
      </p>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Schema shape
      </h2>
      <p>
        The schema is built around runs and violations. A run is the unit of
        execution: one accepted dispatch, one microVM boot, one agent CLI
        process, and one final outcome. Violation rows attach to the run and
        preserve the guard rail that fired plus the relevant path, command, or
        resource threshold.
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
        launched, where it ran, whether it passed, and which guard rails fired.
      </p>

      <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">{`curl -sS http://localhost:<gild-port>/runs \\
  -H "Authorization: Bearer $TANA_GIT_TOKEN"`}</pre>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Metrics endpoint
      </h2>
      <p>
        The <code>/metrics</code> endpoint returns Prometheus-format metrics
        for scrape-based monitoring. Operators use it for run counts, failure
        rates, violation counts, and pressure near configured resource caps.
      </p>

      <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">{`# HELP gild_runs_total Total gild runs observed by the supervisor.
# TYPE gild_runs_total counter
gild_runs_total{status="ok"} 42

# HELP gild_guardrail_violations_total Guard-rail violations by type.
# TYPE gild_guardrail_violations_total counter
gild_guardrail_violations_total{type="outside-workspace"} 1`}</pre>

      <h2 className="text-2xl font-semibold text-foreground mt-10">
        Guard-rail violations
      </h2>
      <ul className="list-disc pl-6 space-y-2">
        <li>
          <strong>forbidden-binary</strong>: the task attempted to execute a
          binary that is not allowed in the guest policy.
        </li>
        <li>
          <strong>boundary</strong>: the task attempted to cross a configured
          sandbox boundary.
        </li>
        <li>
          <strong>outside-workspace</strong>: the task attempted to write or
          operate outside <code>/workspace</code>.
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
        page is the human view for recent runs, failed launches, and guard-rail
        pressure that needs follow-up.
      </p>
    </GildPage>
  )
}
