# Linkhash Workspace

Linkhash is the PHPX package registry and the Deka ecosystem artifact registry.
It serves package metadata for `deka publish`/`deka install` and OCI-compatible
runtime artifacts for gild.

## Layout
- `phpx/`: Linkhash application (PHPX runtime, modules, DB migrations, app config)
- `rust/`: Rust services for Linkhash (`gild-vcs` fork)

## Gild OCI Pull Workflow

The Tana git server exposes a read-only OCI Distribution API surface for gild
root filesystem images:

- `GET /v2` and `GET /v2/`: registry ping.
- `GET /v2/{repository}/manifests/{reference}`: fetch an OCI image manifest,
  image index, or Docker-compatible manifest by tag or digest.
- `GET /v2/{repository}/blobs/{digest}`: fetch a config or layer blob by digest.

The registry reads from `LINKHASH_OCI_ROOT`, defaulting to `data/oci` relative
to the git-server process. The on-disk contract is:

```text
$LINKHASH_OCI_ROOT/
  repositories/
    tana/
      agent-rootfs/
        manifests/
          v1.json
          sha256/<manifest-digest>.json
        tags/
          latest/manifest.json
        blobs/
          sha256/<repo-local-blob-digest>
  blobs/
    sha256/<shared-blob-digest>
```

Manifest lookup by tag checks:

1. `repositories/{name}/manifests/{tag}.json`
2. `repositories/{name}/tags/{tag}/manifest.json`

Manifest lookup by digest checks:

1. `repositories/{name}/manifests/sha256/{hex}.json`
2. `blobs/sha256/{hex}`

Blob lookup checks shared storage first, then repository-local storage:

1. `blobs/sha256/{hex}`
2. `repositories/{name}/blobs/sha256/{hex}`

### Pull Sequence

For gild, the pull path is intentionally plain OCI:

1. Ping the registry and require `Docker-Distribution-API-Version: registry/2.0`.
2. Fetch the manifest for the configured rootfs reference.
3. Verify the returned manifest digest against `Docker-Content-Digest` when a
   pinned digest was requested.
4. Fetch the manifest config blob and every layer blob by digest.
5. Verify each blob digest locally before unpacking or caching.
6. Materialize the rootfs from the layers into gild's image cache.

Example against a local git-server:

```bash
REGISTRY=http://localhost:9418
IMAGE=tana/agent-rootfs
REF=latest

curl -i "$REGISTRY/v2/"
curl -sS "$REGISTRY/v2/$IMAGE/manifests/$REF" -o manifest.json

CONFIG_DIGEST=$(jq -r '.config.digest' manifest.json)
jq -r '.layers[].digest' manifest.json > layers.txt

curl -sS "$REGISTRY/v2/$IMAGE/blobs/$CONFIG_DIGEST" -o config.json
printf '%s  config.json\n' "${CONFIG_DIGEST#sha256:}" | sha256sum -c -

while read -r DIGEST; do
  HEX=${DIGEST#sha256:}
  curl -sS "$REGISTRY/v2/$IMAGE/blobs/$DIGEST" -o "layer-$HEX.tar"
  printf '%s  %s\n' "$HEX" "layer-$HEX.tar" | sha256sum -c -
done < layers.txt
```

Gild should pin production images by digest after tag resolution. Tags are fine
for local development and rollout channels, but the execution cache key should
be the resolved manifest digest plus the layer digests so a tag move never
silently changes an already prepared sandbox.

## Notes
- Post-MVP hardening backlog is tracked in `TODO.md`.
