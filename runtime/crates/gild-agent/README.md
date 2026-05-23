# gild-agent

Privileged local daemon for Gild operations that require root. It listens on
`/run/gild-agent.sock`, authenticates callers with `SO_PEERCRED`, and only
allows root or members of the `gild-orchestrator` group.

This crate is the skeleton for tana#385. Health, request parsing, peer
credential checks, and useradd/userdel command skeletons are wired. More
complete systemd, sudoers, HMAC rotation, and audit behavior land in follow-up
PRs.
