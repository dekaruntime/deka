# deka-git (Linkhash fork)

`deka-git` is Linkhash's canonical Git + package backend.

This fork removes blockchain JWT/ledger auth and uses local identity with PAT/token auth and SSH key registration.

## MVP capabilities

- Git Smart HTTP push/pull/fetch
- Per-user bare repositories (`<repos>/<username>/<repo>.git`)
- Token auth for API and Git transport (`Bearer` or `Basic user:token`)
- SSH public key registry (ed25519)
- Package release publish/list/download endpoints
- Tarball download generated from git refs (`git archive`)

## configuration

Create `config.toml`:

```toml
port = 8508
database_url = "postgres://deka:deka_dev_password@localhost:5434/deka"
repos_path = "./repos"
bootstrap_username = "linkhash-admin"
bootstrap_token = "linkhash-dev-token-change-me"
```

## running

```bash
cargo run
```

## auth model

- Startup ensures a bootstrap user/token exists.
- Use `Authorization: Bearer <token>` for API calls.
- Git over HTTP can use `http://<username>:<token>@host/<username>/<repo>.git`.

## API endpoints

### health
- `GET /health`

### repos (auth required)
- `POST /api/repos/:repo`
- `GET /api/repos`
- `POST /api/repos/:owner/:repo/fork`

### repo preview resolver
- `GET /api/public/repos/:owner/:repo/resolve?ref=HEAD`

### ssh keys (auth required)
- `GET /api/user/ssh-keys`
- `POST /api/user/ssh-keys`
- `DELETE /api/user/ssh-keys/:fingerprint`

### git protocol (auth required)
- `GET /:owner/:repo/info/refs?service=git-receive-pack`
- `POST /:owner/:repo/git-receive-pack`
- `POST /:owner/:repo/git-upload-pack`

### packages
- `POST /api/packages/publish` (auth required)
- `GET /api/packages/:name`
- `GET /api/packages/:name/:version`
- `GET /api/packages/:name/:version/docs`
- `GET /api/packages/:name/:version/docs?symbol=<module::export>`
- `GET /api/packages/:name/:version/tree`
- `GET /api/packages/:name/:version/blob?path=<repo/path.phpx>`
- `GET /api/packages/:name/:version/download`

`/docs` responses are versioned snapshots extracted from source doccomments and export signatures at publish time.

## publish + install flow

1. Create repo: `POST /api/repos/<repo>`
2. Push git content to `http://<user>:<token>@host/<user>/<repo>.git`
3. Publish package release with `POST /api/packages/publish`
4. Install via Deka:

```bash
LINKHASH_REGISTRY_URL=http://localhost:8508 deka install --ecosystem php --spec stdlib/core@0.1.0
```

## Adwa integration notes

- Use resolver endpoint to map symbolic refs (`HEAD`, branch, tag) to immutable commit SHA before preview launch.
- Fork endpoint clones source bare repo into caller-owned target repository and returns target commit metadata.
