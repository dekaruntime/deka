# gild-agent

Privileged local daemon for Gild operations that require root. It listens on
`/run/gild-agent.sock`, authenticates callers with `SO_PEERCRED`, and only
allows root or members of the `gild-orchestrator` group.

This crate is part of tana#385. Health, request parsing, peer credential checks,
and real `useradd`/`userdel` handlers are wired. Systemd, sudoers, HMAC
rotation, and richer agent config writes land in follow-up PRs.
