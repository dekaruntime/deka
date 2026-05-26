# deka-router

`deka-router` is the edge-only Deka platform router for non-shard nodes.

It accepts HTTP traffic, resolves the tenant account id from the request host
using the same `subdomain:{name}` Redis record shape as `deka platform`, maps the
account id to a shard with `deka-shard`, and proxies the request to that shard.

It does not depend on `pool`, `engine`, `runtime`, `deno_core`, or V8. It never
starts an isolate pool and is intended for DigitalOcean edge nodes that only
forward storefront traffic back to the owning shard.

Configuration:

- `--port <port>` or `PORT`: listen port, default `8531`.
- `DEKA_ROUTER_BIND`: listen address, default `127.0.0.1`.
- `REDIS_URL`: optional local Redis URL. When set, the router first tries to read
  the shard config JSON from Redis key `deka:shards`.
- `DEKA_SHARDS_REDIS_KEY`: optional Redis key override for the shard config.
- `DEKA_SHARD_CONFIG`: shard config file path fallback.
- `DEKA_ROUTER_TARGET_PORT`: default upstream platform port when a shard name
  does not already include a port, default `8530`.
- `DEKA_ROUTER_TRUST_ACCOUNT_HEADER=1`: allow `X-Deka-Account-ID` as a fallback
  account id source for local smoke tests.
