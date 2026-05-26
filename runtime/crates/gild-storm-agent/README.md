# gild-storm-agent

`gild-storm-agent` is the per-host fault injector used by the `gild-storm`
scenario runner. It is intended to run through SSH with passwordless sudo.

Every reversible fault arms a detached supervisor before applying the fault. The
supervisor double-forks, starts a new session, execs `gild-storm-agent
__supervisor`, sleeps for the restore deadline, runs the undo action, and writes
an audit line to `/var/log/gild-storm-agent.log`. The restore path is therefore
independent of the invoking CLI or scenario runner.

## Primitives

```bash
gild-storm-agent process kill --service gild-vault --signal SIGTERM --undo-by 60s
gild-storm-agent process kill --service gg.tana.gild-vault --signal SIGKILL --mask-restart --undo-by 35s
gild-storm-agent process start --service gild-vault
gild-storm-agent process pause --service gild-vault --secs 5 --undo-by 10s
gild-storm-agent process pause --service gild-vault --secs 5 --undo-by 10s --mask-restart
gild-storm-agent recover

gild-storm-agent net partition --to demon --secs 10 --undo-by 15s
gild-storm-agent net latency --to tailscale0 --ms 100 --jitter 25 --secs 10 --undo-by 15s
gild-storm-agent net loss --to tailscale0 --pct 5 --secs 10 --undo-by 15s

gild-storm-agent disk fill --path /var/tmp --gb 5 --secs 30 --undo-by 45s
gild-storm-agent mem stress --gb 2 --secs 30 --undo-by 45s
gild-storm-agent cpu stress --pct 80 --secs 30 --undo-by 45s
```

`process kill` restores with `systemctl start`. `process pause` restores with
`systemctl kill -s SIGCONT` after `--secs`; `--undo-by` must be at least that
long and remains the safety deadline for reversible faults. Network netem faults
remove the qdisc at expiry. Disk fill creates one file and unlinks it at expiry.
Stress primitives run `stress-ng` and arm a supervisor to terminate the child if
the parent dies.

`--mask-restart` suppresses systemd auto-restart for the target service while
the fault is active. Before signaling the service, the agent creates
`/etc/systemd/system/<service>.service.d/storm-mask.conf` with `Restart=no`,
writes an owner file under `/run/gild-storm-agent/masks/`, writes a per-run lock
under `/run/gild-storm-agent/`, and runs `systemctl daemon-reload`. The detached
supervisor removes the drop-in, reloads systemd, and starts the service for
`process kill`; for `process pause`, it removes the drop-in, sends `SIGCONT`,
and ensures the unit is started. A concurrent masked fault for the same service
is rejected. `gild-storm-agent recover` clears any orphaned storm-owned masks
and restarts the affected services.
