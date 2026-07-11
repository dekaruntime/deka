# deka-edge-publisher

`deka-edge-publisher` mirrors the canonical edge routing data from demon Neo4j
into demon Redis. Future edge nodes can boot from Redis key scans and then
subscribe to the `edge-updates` pub/sub channel for live deltas.

## Configuration

| Env | Default | Purpose |
|---|---|---|
| `NEO4J_URI` | `bolt://localhost:7687` | Local Neo4j primary |
| `NEO4J_USER` | `neo4j` | Neo4j username |
| `NEO4J_PASSWORD` | empty | Neo4j password; production should inject from harar |
| `REDIS_URL` | `redis://localhost:6379` | Local Redis |
| `STATE_DIR` | `/var/lib/deka-edge-publisher` | Persists `state.json` with `last_seen_timestamp` |
| `POLL_INTERVAL_SECS` | `5` | Poll interval for `updated_at` changes |

## Redis Keys

Every key is written with a 300 second TTL and refreshed on each push.

| Key | Value |
|---|---|
| `shop:<shop_id>:shard` | shard index integer |
| `domain:<name>` | JSON `{ "shop_id": "...", "verified": true }` |
| `domain:<name>:records` | JSON `[{ "type", "name", "value", "ttl", "priority" }]` |
| `subdomain:<sub>` | shop id |
| `verify:<domain>` | verification token |

Each successful write also publishes a JSON delta to `edge-updates`.

## Operation

Startup performs a full sync from `:Shop`, `:Domain`, and `:DnsRecord` rows so
Redis converges even after key expiry or a fresh daemon install. The daemon then
polls for rows where `updated_at > last_seen_timestamp`, applies the same
idempotent Redis writes, publishes deltas, and persists the newest timestamp to
`STATE_DIR/state.json`.

Build and test:

```bash
cargo build --release -p deka-edge-publisher
cargo test -p deka-edge-publisher
```
