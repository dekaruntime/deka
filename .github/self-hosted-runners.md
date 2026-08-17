# Self-hosted runners

To reduce GitHub Actions billing, the Release workflow runs on your own machines instead of GitHub-hosted runners.

## Machine topology

| Platform | Machine | Runner labels | SCCache bucket |
|----------|---------|---------------|----------------|
| linux-x64 | demon | `self-hosted`, `linux`, `x64` | `deka-sccache-linux-x64` |
| darwin-x64 | iMac | `self-hosted`, `macos`, `x64` | `deka-sccache-darwin-x64` |
| darwin-arm64 | bugsy | `self-hosted`, `macos`, `arm64` | `deka-sccache-darwin-arm64` |

## Current status

All three runners are registered and online:

- `demon` (linux-x64) — running as a systemd service
- `imac` (darwin-x64) — running under the current user via `nohup`
- `bugsy` (darwin-arm64) — running under the current user via `nohup`

To make the macOS runners survive reboots, install the service with `sudo ./svc.sh install $(whoami)` and `sudo ./svc.sh start` from each runner directory.

## Why ephemeral runners?

Each runner is registered with `--ephemeral`. After it completes one job, it unregisters from GitHub and deletes its `_work` directory. This guarantees every release build starts from a clean workspace and avoids leftover `target/` or `dist/` directories polluting the machine or future runs.

## Concurrent runs on one machine

A single runner processes exactly one job at a time. If you want concurrent jobs on the same machine (for example two darwin-arm64 builds), register and start multiple runner instances with unique `--name` values and the same labels. Each one will be ephemeral and clean up after itself.

## Setup steps (for reference or additional machines)

1. Get a fresh registration token from GitHub (Settings → Actions → Runners → New self-hosted runner). It expires after one hour.

   ```bash
   export RUNNER_TOKEN="<token-from-github>"
   ```

2. Download, configure, and start the runner.

   Replace `<VERSION>` with the latest runner release (e.g. `2.336.0`) from https://github.com/actions/runner/releases.

   **linux-x64 (demon):**
   ```bash
   mkdir -p ~/actions-runner && cd ~/actions-runner
   curl -o actions-runner-linux-x64-<VERSION>.tar.gz -L https://github.com/actions/runner/releases/download/v<VERSION>/actions-runner-linux-x64-<VERSION>.tar.gz
   tar xzf ./actions-runner-linux-x64-<VERSION>.tar.gz
   ./config.sh --url https://github.com/dekaruntime/deka --token "$RUNNER_TOKEN" --labels self-hosted,linux,x64 --name demon --ephemeral --unattended
   sudo ./svc.sh install "$(whoami)"
   sudo ./svc.sh start
   ```

   **macos-x64 (iMac):**
   ```bash
   mkdir -p ~/actions-runner && cd ~/actions-runner
   curl -o actions-runner-osx-x64-<VERSION>.tar.gz -L https://github.com/actions/runner/releases/download/v<VERSION>/actions-runner-osx-x64-<VERSION>.tar.gz
   tar xzf ./actions-runner-osx-x64-<VERSION>.tar.gz
   ./config.sh --url https://github.com/dekaruntime/deka --token "$RUNNER_TOKEN" --labels self-hosted,macos,x64 --name imac --ephemeral --unattended
   # If you have sudo access:
   sudo ./svc.sh install "$(whoami)"
   sudo ./svc.sh start
   # Otherwise run interactively:
   # nohup ./run.sh > runner.log 2>&1 &
   ```

   **macos-arm64 (bugsy):**
   ```bash
   mkdir -p ~/actions-runner && cd ~/actions-runner
   curl -o actions-runner-osx-arm64-<VERSION>.tar.gz -L https://github.com/actions/runner/releases/download/v<VERSION>/actions-runner-osx-arm64-<VERSION>.tar.gz
   tar xzf ./actions-runner-osx-arm64-<VERSION>.tar.gz
   ./config.sh --url https://github.com/dekaruntime/deka --token "$RUNNER_TOKEN" --labels self-hosted,macos,arm64 --name bugsy --ephemeral --unattended
   # If you have sudo access:
   sudo ./svc.sh install "$(whoami)"
   sudo ./svc.sh start
   # Otherwise run interactively:
   # nohup ./run.sh > runner.log 2>&1 &
   ```

3. Ensure the machine has the required tools installed:
   - Rust toolchain (1.96.0) with `wasm32-unknown-unknown` target on the Linux runner
   - Bun
   - sccache (`cargo install sccache`)
   - `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` available to the runner process

## Pre-populating sccache

Before the first tagged release on self-hosted runners, warm the cache by running on each machine:

```bash
R2_ACCESS_KEY_ID=... R2_SECRET_ACCESS_KEY=... runtime/scripts/populate-sccache.sh
```

This writes compiled artifacts to the matching R2 bucket so the first self-hosted Release workflow run is fast.
