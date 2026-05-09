import { DocBreadcrumbs } from '@/components/docs/DocBreadcrumbs'

export const metadata = {
  title: 'Agents | Deka',
  description:
    'In Tana, an agent is a persona with durable identity backed by a unix user and a persistent dispatcher. Workers are ephemeral.',
}

export default function PlatformAgentsPage() {
  return (
    <div className="max-w-5xl mx-auto px-8 py-12">
      <article className="prose prose-invert max-w-none">
        <DocBreadcrumbs
          items={[
            { label: 'docs', href: '/docs' },
            { label: 'platform', href: '/docs/platform' },
            { label: 'agents' },
          ]}
        />

        <h1 className="text-4xl font-bold text-foreground mb-2 not-prose">
          Agents
        </h1>
        <p className="text-xl text-muted-foreground mb-8 not-prose">
          A persona with durable identity. Workers are ephemeral.
        </p>

        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            In Tana, an <strong>agent</strong> is a persona — a named, scoped
            engineering role with its own responsibilities, its own filesystem
            home, and its own audit trail. An agent is not a process. It is not
            a single LLM call. It is closer to a coworker on the team: identity
            persists, the workspace persists, and any number of tasks can be
            dispatched to it over its lifetime.
          </p>
          <p>
            The thing that actually thinks — the LLM child process — is the
            short-lived part. It is spawned per task and discarded when the
            task is done.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Not a Claude subagent
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Claude Code has its own concept called a{' '}
            <em>subagent</em>: a transient helper invoked from inside a parent
            Claude session. Tana agents are different. They are first-class
            principals on the host. They show up in <code>/etc/passwd</code>,
            they own files on disk, they sign their own git commits, and they
            run their own long-lived dispatcher daemon. A Claude subagent is a
            tool call. A Tana agent is an account.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          The runtime model
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Each agent has three pieces:
          </p>
          <ul className="list-disc pl-6 space-y-2">
            <li>
              <strong>A unix user.</strong> Named{' '}
              <code>agent-&lt;slug&gt;</code> — for example{' '}
              <code>agent-yasmin</code>. The user owns a home directory, a
              project workspace, ssh keys, a git identity, and ACL-scoped
              access to the resources it needs. Files written by the agent
              are owned by the agent. Commits authored by the agent are
              signed by the agent. There is one user per persona.
            </li>
            <li>
              <strong>A persistent dispatcher.</strong> A{' '}
              <code>Bun.serve</code> HTTP server running as the agent&apos;s
              unix user, supervised by systemd as{' '}
              <code>gg.tana.&lt;slug&gt;.service</code>. It listens on a
              private socket, authenticates incoming requests with HMAC, and
              spawns a worker process for each accepted task. The dispatcher
              is the durable part — it stays up between tasks. It loads the
              persona file once at startup and reuses it.
            </li>
            <li>
              <strong>Ephemeral workers.</strong> One{' '}
              <code>claude --print</code> (or <code>codex</code>, or{' '}
              <code>opencode</code>) child process per task. The worker
              inherits the agent&apos;s unix uid, reads the persona as its
              system prompt, runs the task, prints a result, and exits. It
              does not persist between tasks. It does not see other tasks.
              When it dies, its context dies with it.
            </li>
          </ul>
          <p>
            The persona itself lives at{' '}
            <code>.claude/agents/&lt;slug&gt;.md</code> in the agent&apos;s
            home — a markdown file describing the agent&apos;s scope,
            responsibilities, and what they do not touch. The dispatcher
            reads it once. Every worker it spawns gets that text injected as
            the system prompt.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Worked example: three Yasmins
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Yasmin is the frontend agent. She owns the Next.js apps. There is
            one user (<code>agent-yasmin</code>), one home directory (
            <code>/home/agent-yasmin</code>), one persona file, and one
            dispatcher daemon.
          </p>
          <p>
            When three tasks arrive at the dispatcher concurrently — say, a
            navbar tweak, a bugfix on the admin dashboard, and a
            documentation update — the dispatcher spawns three independent{' '}
            <code>claude --print</code> child processes. Each one wears the
            Yasmin name tag. Each one has its own context window. None of
            them can see the other two. When each task completes, that
            worker exits. The dispatcher keeps running.
          </p>
          <p>Three concurrent calls = three child processes wearing the same name tag.</p>

          <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">
{`         ┌────────────────────────────────────────┐
         │  agent-yasmin (unix user)              │
         │  /home/agent-yasmin                    │
         │  .claude/agents/yasmin.md  (persona)   │
         │                                        │
         │  ┌──────────────────────────────────┐  │
         │  │ gg.tana.yasmin.service           │  │
         │  │ Bun.serve on private socket      │  │  durable
         │  │ persona loaded once at startup   │  │
         │  └────┬─────────────┬────────────┬──┘  │
         │       │ /task       │ /task      │     │
         │       ▼             ▼            ▼     │
         │   claude --print  claude --print claude --print
         │   (worker A)      (worker B)    (worker C)    ephemeral
         │   exits on done   exits on done exits on done
         └────────────────────────────────────────┘`}
          </pre>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Dispatching a task
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Tasks reach the dispatcher over HTTP. The request is signed with
            an HMAC over the body and a timestamp; the dispatcher rejects
            anything older than its skew window or with a bad signature.
            Once accepted, it forks a worker, streams the prompt in,
            collects output, and returns the result.
          </p>

          <pre className="not-prose bg-background border border-border rounded-md p-4 font-mono text-xs overflow-x-auto leading-relaxed">
{`# Send a task to the Yasmin dispatcher.
BODY='{"prompt":"Audit the Navbar component for a11y issues."}'
TS=$(date +%s)
SIG=$(printf '%s.%s' "$TS" "$BODY" \\
        | openssl dgst -sha256 -hmac "$AGENT_HMAC_KEY" -hex \\
        | awk '{print $2}')

curl -sS http://yasmin.agents.local/task \\
  -H "X-Tana-Timestamp: $TS" \\
  -H "X-Tana-Signature: $SIG" \\
  -H 'Content-Type: application/json' \\
  --data "$BODY"
# → dispatcher verifies HMAC
# → spawns: sudo -u agent-yasmin claude --print --append-system .claude/agents/yasmin.md
# → returns the worker's stdout`}
          </pre>

          <p>
            The dispatcher does not run the LLM itself. It is a router that
            knows how to spawn a worker as the right unix user with the
            right system prompt and the right working directory. The
            interesting part — the model — is in the child.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Why this shape
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            <strong>Durable identity, ephemeral workers</strong> falls out of
            two requirements that pull in opposite directions.
          </p>
          <p>
            Identity has to be durable because the rest of the system holds
            it accountable. Git commits need a stable author. Filesystem
            ownership needs a stable uid. ACLs need a stable principal. Cost
            telemetry needs a stable subject. Code review needs a stable
            reviewer. If every task spun up a fresh anonymous process, none
            of that would land — every commit would look like it came from
            nobody, every audit trail would be a list of disconnected
            invocations. The unix user is what makes &quot;Yasmin shipped
            this&quot; mean something.
          </p>
          <p>
            Workers have to be ephemeral because LLM context does not
            compose. Two unrelated tasks running in the same context window
            poison each other — the model starts blending the threads, the
            system prompt gets eroded, prior tool calls leak into the next
            task&apos;s reasoning. A fresh child per task is the cheapest
            fix. It is also what makes parallelism safe: three workers in
            three processes cannot interfere because they share nothing but
            the filesystem and a name.
          </p>
          <p>
            Pulled together, an agent looks like a coworker more than a
            function. Their laptop (the workspace) persists between tasks —
            cloned repos, shell history, cached dependencies, ssh keys. But
            their <em>brain</em> for any given task is fresh: the worker
            wakes up, reads the persona, reads the task, does the work,
            writes the result, and goes away. Nothing carries over from
            yesterday&apos;s work into today&apos;s except what got
            committed to disk.
          </p>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          What this enables
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <ul className="list-disc pl-6 space-y-2">
            <li>
              <strong>Parallel work.</strong> One agent can run as many
              workers concurrently as the host has capacity for. Three
              Yasmins, ten Yasmins — same dispatcher, separate processes,
              no cross-talk.
            </li>
            <li>
              <strong>Real attribution.</strong> Every commit is signed by
              the agent that authored it. Every file on disk has the
              agent&apos;s uid. Every dispatcher log line is tagged with
              the persona. &quot;Who wrote this?&quot; has an answer.
            </li>
            <li>
              <strong>Real cost telemetry.</strong> Workers are processes,
              processes have lifetimes, lifetimes have cost. Per-agent
              spend is the sum of its workers&apos; spend. No estimation
              required.
            </li>
            <li>
              <strong>Real grants.</strong> Because the agent is a unix
              principal, capability grants are unix grants — sudoers
              entries, group membership, ACLs on a directory. The same
              tools that scope a human engineer scope an agent.
            </li>
            <li>
              <strong>Hot persona reload.</strong> Editing{' '}
              <code>.claude/agents/&lt;slug&gt;.md</code> and restarting{' '}
              <code>gg.tana.&lt;slug&gt;.service</code> updates every
              future worker without touching their code.
            </li>
          </ul>
        </section>

        <h2 className="text-2xl font-semibold text-foreground mt-12 mb-4">
          Today
        </h2>
        <section className="space-y-4 text-foreground/90 leading-relaxed">
          <p>
            Agents currently run on a single host (<code>demon</code>). Each
            persona has one unix user, one systemd unit, and one persona
            file. Distribution across hosts, agent-to-agent direct
            messaging, and richer workspace policy are all open. The shape
            described above is the load-bearing part — the rest accretes
            around it.
          </p>
        </section>
      </article>
    </div>
  )
}
