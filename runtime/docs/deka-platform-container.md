# Deka Platform Container

The storefront platform ships as an OCI image instead of a raw host binary.
Builds use Debian bookworm glibc 2.36 in both stages, so the resulting
`deka-cli` runs on Ubuntu 24.04 droplets without depending on the builder
host glibc.

## Build

From `runtime/`:

```sh
short_sha="$(git rev-parse --short HEAD)"
docker build \
  -f Dockerfile.platform \
  --build-arg DEKA_GIT_SHA="$short_sha" \
  --build-arg DEKA_BUILD_UNIX="$(date +%s)" \
  -t "tana/deka-platform:${short_sha}" \
  -t tana/deka-platform:latest \
  .
```

## Runtime Contract

The image entrypoint is:

```json
["/usr/local/bin/deka-cli", "platform"]
```

The default command is `/srv/tana/store`, so operators should mount the
storefront tree there. The platform listens on port `8530`.

Required environment:

- `NEO4J_URI`: shard Neo4j Bolt URI.
- `NEO4J_USER`: Neo4j user, defaults to `neo4j` when unset.
- `NEO4J_PASSWORD`: Neo4j password.
- `REDIS_URL`: shard Redis URL.
- `DEKA_SHARD_SELF`: this shard name, for example `phobos`, `bugsy`, or `jynx`.
- `DEKA_SHARD_INDEX`: numeric shard index consumed by deployment tooling.
- `DEKA_PLATFORM_BIND=0.0.0.0`: explicit container bind address.
- `PORT=8530`: deployment convention. `deka platform` currently uses `--port`
  for the listener, so pass `--port 8530` in the command when overriding it.

The platform also accepts the internal `DEKA_NEO4J_*` and `DEKA_REDIS_URL`
names. If only the container contract names above are present, startup copies
them into the internal names before loading database configuration.

Common optional environment:

- `DEKA_SHARD_CONFIG`: path to shard config mounted into the container.
- `DEKA_PLATFORM_API=1`: enables built-in `/api/*` platform routes.
- `DEKA_PLATFORM_ENV_ALLOWLIST`: comma-separated extra env vars to expose to
  tenant PHPX isolates.
- `STRIPE_PUBLISHABLE_KEY`, `TANA_INTERNAL_API_SECRET`, `STRIPE_STUB`: default
  tenant-visible platform env vars when present.

## Run

Recommended deployment shape for Tariq's cloud-init work:

```sh
docker run -d --name deka-platform \
  --restart unless-stopped \
  --network host \
  -e NEO4J_URI="$NEO4J_URI" \
  -e NEO4J_USER="$NEO4J_USER" \
  -e NEO4J_PASSWORD="$NEO4J_PASSWORD" \
  -e REDIS_URL="$REDIS_URL" \
  -e DEKA_SHARD_SELF="$DEKA_SHARD_SELF" \
  -e DEKA_SHARD_INDEX="$DEKA_SHARD_INDEX" \
  -e DEKA_PLATFORM_BIND=0.0.0.0 \
  -e DEKA_PLATFORM_API=1 \
  -v /opt/tana/store:/srv/tana/store:ro \
  tana/deka-platform:latest \
  /srv/tana/store --port 8530
```

Use `--network host` on droplets so the container can reach tailnet-local
Neo4j/Redis endpoints and bind the host's `8530` listener directly.

If host networking is not available, publish `8530/tcp` and make sure the
database URLs are reachable from Docker bridge networking:

```sh
docker run -d --name deka-platform \
  -p 8530:8530 \
  -e DEKA_PLATFORM_BIND=0.0.0.0 \
  -v /opt/tana/store:/srv/tana/store:ro \
  tana/deka-platform:latest \
  /srv/tana/store --port 8530
```

## Runtime Dependencies

The runtime stage installs:

- `ca-certificates`
- `libgcc-s1`
- `libssl3`
- `libzstd1`
- `zlib1g`

Verify any future dependency drift with:

```sh
docker run --rm --entrypoint /bin/sh tana/deka-platform:latest \
  -c 'ldd /usr/local/bin/deka-cli'
```

Clean build cache on demon after validation:

```sh
docker builder prune -f
```
