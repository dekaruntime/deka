# Linkhash Agent ACL Foundation (deka slice for tana#208)

- [x] Add first-class agent account, SSH key, repo ACL, secret ACL, and webhook subscription schema.
- [x] Add account bootstrap, SSH key registration, identity introspection, repo ACL, secret ACL, and webhook subscription endpoints.
- [x] Load explicit repo and secret grants into authenticated linkhash identities.
- [x] Enforce repo ACLs across Git write paths and package publish/preflight paths.
- [ ] Create a narrower `tana/deka` foundation issue for this PR and retarget the PR to close it. (blocked: current `TANA_GIT_TOKEN` lacks `issues:write`; API returns `issues:write scope required`)

Still tracked by umbrella tana#208:

- [ ] Wire SSH public-key authentication from per-agent Unix accounts into linkhash token identity.
- [ ] Connect `tana env pull` to vault-backed secret value retrieval using linkhash secret ACL decisions.
- [ ] Seed production Idris and Layla SSH keys after #207 provides stable public keys.
- [ ] Demonstrate live Idris allow/deny pushes, scoped env pull output, persisted webhook subscription, Layla onboarding, and rights audit before closing tana#208.
