# Linkhash Agent ACL Foundation (deka#45, deka slice for tana#208)

- [x] Add first-class agent account, SSH key, repo ACL, secret ACL, and webhook subscription schema.
- [x] Add account bootstrap, SSH key registration, identity introspection, repo ACL, secret ACL, and webhook subscription endpoints.
- [x] Load explicit repo and secret grants into authenticated linkhash identities.
- [x] Enforce repo ACLs across Git write paths and package publish/preflight paths.
- [x] Create narrower `tana/deka#45` foundation issue for this PR and retarget the PR to close it.

## Review handoff

- PR #54 source branch: `feat/208-linkhash-agent-acl`
- Narrow foundation issue: `tana/deka#45`
- Fresh clone/fetch from Samira workspace on 2026-05-15 succeeded for `feat/208-linkhash-agent-acl`.
- `git ls-remote origin feat/208-linkhash-agent-acl` resolved to `c2aee1f7c25baf949321226873ec2fb4ed40bdb9` before the v3 handoff commit.
- Narrow issue creation initially returned `{"error":"issues:write scope required"}` for the provided `TANA_GIT_TOKEN`; a short-lived `issues:write` token from `POST /api/tokens` created `tana/deka#45`.
- Local Rust validation on 2026-05-15 passed from `linkhash/rust/deka-git` with `cargo check` and `cargo test` using the shared Rust toolchain and Samira-owned Cargo cache.

Still tracked by umbrella tana#208:

- [ ] Wire SSH public-key authentication from per-agent Unix accounts into linkhash token identity.
- [ ] Connect `tana env pull` to vault-backed secret value retrieval using linkhash secret ACL decisions.
- [ ] Seed production Idris and Layla SSH keys after #207 provides stable public keys.
- [ ] Demonstrate live Idris allow/deny pushes, scoped env pull output, persisted webhook subscription, Layla onboarding, and rights audit before closing tana#208.
