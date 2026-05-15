# Linkhash Agent ACL Foundation (deka slice for tana#208)

- [x] Add first-class agent account, SSH key, repo ACL, secret ACL, and webhook subscription schema.
- [x] Add account bootstrap, SSH key registration, identity introspection, repo ACL, secret ACL, and webhook subscription endpoints.
- [x] Load explicit repo and secret grants into authenticated linkhash identities.
- [x] Enforce repo ACLs across Git write paths and package publish/preflight paths.
- [ ] Create a narrower `tana/deka` foundation issue for this PR and retarget the PR to close it. (blocked: current `TANA_GIT_TOKEN` lacks `issues:write`; API returns `issues:write scope required`)

## Review handoff

- PR #54 source branch: `feat/208-linkhash-agent-acl`
- Fresh clone/fetch from Samira workspace on 2026-05-15 succeeded for `feat/208-linkhash-agent-acl`.
- `git ls-remote origin feat/208-linkhash-agent-acl` resolved to `c2aee1f7c25baf949321226873ec2fb4ed40bdb9` before the v3 handoff commit.
- Narrow issue creation retry on 2026-05-15 still returned `{"error":"issues:write scope required"}` for `POST /api/repos/tana/deka/issues`.
- Local Rust validation is blocked in the Samira workspace because `cargo` and `rustc` are not installed on `PATH`.

Still tracked by umbrella tana#208:

- [ ] Wire SSH public-key authentication from per-agent Unix accounts into linkhash token identity.
- [ ] Connect `tana env pull` to vault-backed secret value retrieval using linkhash secret ACL decisions.
- [ ] Seed production Idris and Layla SSH keys after #207 provides stable public keys.
- [ ] Demonstrate live Idris allow/deny pushes, scoped env pull output, persisted webhook subscription, Layla onboarding, and rights audit before closing tana#208.
