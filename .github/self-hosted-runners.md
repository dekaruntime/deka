# Self-hosted runners

To reduce GitHub Actions billing, the Release workflow can run on your own machines instead of GitHub-hosted runners.

## Machine topology

| Platform | Machine | Runner labels | Bucket it can warm |
|----------|---------|---------------|-------------------|
| linux-x64 | demon | `self-hosted`, `linux`, `x64` | `deka-sccache-linux-x64` |
| darwin-x64 | iMac | `self-hosted`, `macos`, `x64` | `deka-sccache-darwin-x64` |
| darwin-arm64 | bugsy | `self-hosted`, `macos`, `arm64` | `deka-sccache-darwin-arm64` |

## Setup steps (per machine)

1. Create a directory for the runner and download the latest runner package.

   **linux-x64 (demon):**
   ```bash
   mkdir -p ~/actions-runner && cd ~/actions-runner
   curl -o actions-runner-linux-x64-2.319.1.tar.gz -L https://github.com/actions/runner/releases/download/v2.319.1/actions-runner-linux-x64-2.319.1.tar.gz
   tar xzf ./actions-runner-linux-x64-2.319.1.tar.gz
   ```

   **macos-x64 (iMac):**
   ```bash
   mkdir -p ~/actions-runner && cd ~/actions-runner
   curl -o actions-runner-osx-x64-2.319.1.tar.gz -L https://github.com/actions/runner/releases/download/v2.319.1/actions-runner-osx-x64-2.319.1.tar.gz
   tar xzf ./actions-runner-osx-x64-2.319.1.tar.gz
   ```

   **macos-arm64 (bugsy):**
   ```bash
   mkdir -p ~/actions-runner && cd ~/actions-runner
   curl -o actions-runner-osx-arm64-2.319.1.tar.gz -L https://github.com/actions/runner/releases/download/v2.319.1/actions-runner-osx-arm64-2.319.1.tar.gz
   tar xzf ./actions-runner-osx-arm64-2.319.1.tar.gz
   ```

2. Configure the runner against `dekaruntime/deka`.

   Get a registration token from the repo settings page (Settings → Actions → Runners → New self-hosted runner), then:

   ```bash
   ./config.sh --url https://github.com/dekaruntime/deka --token <TOKEN> --labels self-hosted,<os>,<arch> --name <machine-name>
   ```

   Examples:
   - demon: `./config.sh --url https://github.com/dekaruntime/deka --token <TOKEN> --labels self-hosted,linux,x64 --name demon`
   - iMac: `./config.sh --url https://github.com/dekaruntime/deka --token <TOKEN> --labels self-hosted,macos,x64 --name imac`
   - bugsy: `./config.sh --url https://github.com/dekaruntime/deka --token <TOKEN> --labels self-hosted,macos,arm64 --name bugsy`

3. Install and start the service.

   ```bash
   sudo ./svc.sh install
   sudo ./svc.sh start
   ```

4. Ensure the machine has the required tools installed:
   - Rust toolchain (1.96.0) with `wasm32-unknown-unknown` target on the Linux runner
   - Bun
   - sccache (`cargo install sccache`)
   - `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` available to the runner process (set in `~/.bashrc`, `/etc/environment`, or the repo secrets if using organization-level secrets)

## Pre-populating sccache

Before the first tagged release on self-hosted runners, warm the cache by running:

```bash
R2_ACCESS_KEY_ID=... R2_SECRET_ACCESS_KEY=... runtime/scripts/populate-sccache.sh
```

This writes compiled artifacts to the matching R2 bucket so the first Release workflow run is fast.

## Switching the Release workflow

Once all three runners show as "Idle" in GitHub Settings → Actions → Runners, update `.github/workflows/release.yml` to use the self-hosted labels:

```yaml
runs-on: [self-hosted, linux, x64]
```

etc. The workflow file in this repo is already prepared for that change.
