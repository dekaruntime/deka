# gild-chain

`gild-chain` is the Rust chain orchestrator for agent work. It subscribes to pulse
SSE, stores chain state in SQLite, and drives the default dispatch -> review ->
merge flow that used to live in local watcher scripts.

## Supported Pulse Events

The daemon opens one SSE stream with `kind=agent.*` and filters the event kinds it
understands:

- `agent.run.completed`
- `agent.pr.opened`
- `agent.review.posted`
- `agent.pr.merged`

Pulse events use this envelope:

```json
{
  "id": 123,
  "source_id": "gild",
  "kind": "agent.pr.opened",
  "payload": {
    "run_id": "run_abc",
    "agent_slug": "agent-khalid",
    "repo": "tana/deka",
    "pr_number": 110
  },
  "ts": "2026-05-23T16:00:00Z"
}
```

The parser reads `payload.run_id`, `payload.agent_slug`, `payload.repo`, and
`payload.pr_number` where applicable. Legacy top-level fields are still accepted
for the original `agent.run.completed` path.

## Default Flow

`default-flow` stores one row per chain in `/var/lib/gild-chain/state.db`.

1. `agent.run.completed` records the completed run. If the event already includes
   a PR number, `gild-chain` requests an Amina review immediately. Otherwise it
   waits for `agent.pr.opened`.
2. `agent.pr.opened` links the PR to its parent run in SQLite and requests an
   Amina review once the run is completed.
3. `agent.review.posted` records the verdict. If the verdict is approved and the
   PR is still open, `gild-chain` asks deka-git to merge it.
4. `agent.pr.merged` marks the chain closed.

Without `--dispatcher-url` or `--git-api-url`, actions are logged as stubs. With
URLs configured, review dispatch posts to the dispatcher `/task` endpoint and
merge sends:

```text
PATCH {git_api_url}/api/repos/{owner}/{repo}/pulls/{pr_number}
{"state":"merged"}
```

## Manual Smoke

```bash
gild-chain daemon --pulse-url http://localhost:9460/v1/subscribe
```

For a local mock pulse server, emit the four supported events above in order and
watch `gild-chain status <chain_id>` move through waiting for PR, waiting for
review, approved, and closed.
