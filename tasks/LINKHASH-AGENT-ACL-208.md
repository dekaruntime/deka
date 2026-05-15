# Linkhash Agent ACL Foundation (#208)

- [x] Add first-class agent account, SSH key, repo ACL, secret ACL, and webhook subscription schema.
- [ ] Wire SSH public-key authentication from per-agent Unix accounts into linkhash token identity.
- [ ] Connect `tana env pull` to vault-backed secret value retrieval using linkhash secret ACL decisions.
- [ ] Seed production Idris and Layla SSH keys after #207 provides stable public keys.
- [ ] Enforce repo ACLs across package publish/preflight paths, not only Git transport.
