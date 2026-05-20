import { GildPage } from '../components'

export const metadata = {
  title: 'Gild operations runbook | Deka',
  description:
    'Rootfs invariants, gild-guest mount/env requirements, host quirks, and deka-git protocol gotchas — catalogued during the agents-in-gild pilot.',
}

export default function GildOperationsPage() {
  return (
    <GildPage
      slug="operations"
      title="Operations runbook"
      description="Catalog of operational invariants and quirks discovered while bringing the agents-in-gild pilot to a working state on demon (2026-05-17 → 2026-05-19). The place to look before debugging an existing issue or modifying the rootfs build / host setup."
    >
      <h2 className="text-2xl font-semibold text-foreground mt-10 not-prose">
        Rootfs invariants (<code>deploy/rootfs/build-ubuntu.sh</code>)
      </h2>
      <ul className="list-disc pl-6 space-y-2">
        <li>
          <strong><code>ROOTFS_SIZE=5120M</code>.</strong> 1 GiB and 2 GiB both
          fill during the global <code>npm install</code> of{' '}
          <code>opencode-ai</code> (the package + dependencies expand past
          1.5 GiB on disk before cleanup). 5 GiB is the comfortable floor.
        </li>
        <li>
          <strong>bun installer URL.</strong> Upstream{"'"}s official installer
          fetches a <code>.zip</code>, not a <code>.tar.gz</code>, and the
          build script must install <code>unzip</code> from apt before the
          installer can complete. The <code>.tar.gz</code> URL returns 404
          from Bun{"'"}s CDN.
        </li>
        <li>
          <strong><code>universe</code> apt source is required</strong> for{' '}
          <code>unzip</code> and several other packages used by the rootfs
          (it is not in <code>main</code>).
        </li>
        <li>
          <strong>LLM CLI install order.</strong> Install <code>bun</code>{' '}
          and <code>rustup</code> first, then{' '}
          <code>
            npm install -g @openai/codex @anthropic-ai/claude-code opencode-ai
          </code>
          . The CLIs need <code>PATH</code> to include node + bun globals.
        </li>
        <li>
          <strong>Clean <code>/tmp</code> after npm install.</strong>{' '}
          <code>npm install -g opencode-ai</code> leaves hundreds of megabytes
          under <code>/tmp/opencode</code> inside the chroot. Run{' '}
          <code>{'rm -rf "${ROOTDIR}/tmp"/*'}</code> once npm is done, or the
          rootfs bloats and the warm pool starts each VM with stale caches.
        </li>
        <li>
          <strong><code>make-private</code> propagation fix.</strong> Bind
          mounts inside the chroot must be marked <code>MS_PRIVATE</code>{' '}
          (or unshare the mount namespace) before <code>rm -rf</code>{' '}
          operations run inside it. Without this, an <code>rm -rf</code>{' '}
          inside the chroot can propagate back to the host{"'"}s{' '}
          <code>/dev</code>, wiping device nodes. This was observed on demon
          and required manual <code>mknod</code> recovery. See gild PR #62
          for the canonical fix.
        </li>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10 not-prose">
        gild-guest (Rust) invariants
      </h2>
      <p>
        <code>gild-guest</code> is the Rust binary that ships as{' '}
        <code>/sbin/init</code> inside the rootfs. The following invariants
        belong in <code>guest/crates/cli/src/cli/run.rs</code> and{' '}
        <code>guest/crates/cli/src/cli/serve.rs</code>.
      </p>

      <h3 className="text-xl font-semibold text-foreground mt-6 not-prose">
        tmpfs sizing
      </h3>
      <p>
        The guest creates tmpfs at <code>/root</code>, <code>/workspace</code>,
        and <code>/tmp</code> so workload tooling can write to them despite
        the read-only base rootfs. Defaults:
      </p>
      <div className="not-prose overflow-x-auto">
        <table className="text-sm border border-border">
          <thead className="bg-secondary/40">
            <tr>
              <th className="text-left px-3 py-2 border-b border-border">Mount</th>
              <th className="text-left px-3 py-2 border-b border-border">Min size</th>
              <th className="text-left px-3 py-2 border-b border-border">Why</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td className="px-3 py-2 border-b border-border align-top"><code>/root</code></td>
              <td className="px-3 py-2 border-b border-border align-top">256 MiB</td>
              <td className="px-3 py-2 border-b border-border align-top">
                opencode keeps its SQLite session DB under{' '}
                <code>/root/.local/share/opencode</code>; a 1 MiB tmpfs
                produced <code>SQLiteError: database or disk is full</code>.
              </td>
            </tr>
            <tr>
              <td className="px-3 py-2 border-b border-border align-top"><code>/workspace</code></td>
              <td className="px-3 py-2 border-b border-border align-top">256 MiB</td>
              <td className="px-3 py-2 border-b border-border align-top">
                clones + node_modules + build artefacts for typical agent
                tasks.
              </td>
            </tr>
            <tr>
              <td className="px-3 py-2 align-top"><code>/tmp</code></td>
              <td className="px-3 py-2 align-top">64 MiB</td>
              <td className="px-3 py-2 align-top">CLI scratch, compile caches.</td>
            </tr>
          </tbody>
        </table>
      </div>

      <h3 className="text-xl font-semibold text-foreground mt-6 not-prose">
        Other guest invariants
      </h3>
      <ul className="list-disc pl-6 space-y-2">
        <li>
          <strong>No <code>MS_NOEXEC</code> on <code>/tmp</code>.</strong>{' '}
          opencode and codex spawn helper binaries out of <code>/tmp</code>.
          Mounting it noexec breaks them.
        </li>
        <li>
          <strong><code>NixPath</code> borrow pattern.</strong> Build the
          mount-options string into a named variable (
          <code>{`let opts = format!(...); Some(opts.as_str())`}</code>);
          inlining the <code>format!</code> drops the temporary before{' '}
          <code>mount()</code> is called and silently passes garbage options.
        </li>
        <li>
          <strong>
            <code>GILD_SUBCOMMAND=pool</code> routes <code>init</code> →{' '}
            <code>serve</code>.
          </strong>{' '}
          When the pool launches a VM in <code>pool</code> mode,{' '}
          <code>serve</code> must skip <code>load_config()</code> (the runtime
          config is host-managed in pool mode). See gild PR #66 — the{' '}
          <code>is_warm_pool()</code> gate.
        </li>
        <li>
          <strong>Pass LLM env through <code>exec_run</code>.</strong>{' '}
          <code>serve</code> receives an <code>LLM_ENV</code> payload from the
          workspace HTTP server and must propagate it as environment to the
          spawned workload process. See gild PR #68.
        </li>
        <li>
          <strong><code>HOME=/root</code> is required.</strong> bun{"'"}s first
          action is to create <code>$HOME/.local</code>; without{' '}
          <code>HOME</code> it defaults to <code>/</code> (read-only) and
          fails with <code>EROFS</code>. The workspace payload script must
          export <code>HOME=/root</code> before any bun command.
        </li>
        <li>
          <strong>Auth file write must bypass shell escape.</strong>{' '}
          <code>JSON.stringify()</code> produces literal <code>\n</code>{' '}
          sequences that the shell preserves; piping that through{' '}
          <code>{`echo "$x" > /path/auth.json`}</code> corrupts the JSON.
          Pipe the payload through <code>{`bun -e '...'`}</code> reading from
          stdin instead — the <code>core/executor/manager.ts</code>{' '}
          workspace-payload prelude is the reference implementation.
        </li>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10 not-prose">
        Host quirks (demon)
      </h2>
      <ul className="list-disc pl-6 space-y-2">
        <li>
          <strong>
            <code>vhost-net</code> and <code>vhost-vsock</code> kernel modules
          </strong>{' '}
          are required for VM networking and host↔guest message passing.{' '}
          <code>modprobe vhost_net vhost_vsock</code> at boot. If the rootfs
          build{"'"}s propagation bug fires and wipes <code>/dev</code>, the{' '}
          <code>vhost-net</code> (10:200) and <code>vhost-vsock</code>{' '}
          (10:241) device nodes need to be recreated with <code>mknod</code>{' '}
          and <code>udevadm trigger</code>.
        </li>
        <li>
          <strong>
            <code>br-gild</code> bridge IP drops on{' '}
            <code>systemctl restart gg.tana.gild</code>.
          </strong>{' '}
          Until this is moved into an <code>ExecStartPre</code>, the operator
          must run <code>ip addr add 172.30.0.1/24 dev br-gild</code> before
          each restart, or the VMs boot with no gateway.
        </li>
        <li>
          <strong>iptables rule ordering.</strong> The{' '}
          <code>GILD-VM-EGRESS</code> chain contains the rule{' '}
          <code>-A GILD-VM-EGRESS -d 172.30.0.0/24 -j ACCEPT</code> which
          <em> must be ordered before</em> any RFC1918 drop rules, or
          host↔VM forwarding silently fails. Persisted via{' '}
          <code>netfilter-persistent</code> from{' '}
          <code>/etc/iptables/rules.v4.d/gild.rules</code>.
        </li>
        <li>
          <strong>
            <code>systemd-networkd-wait-online</code> blocks dependent
            services
          </strong>{' '}
          post-reboot if any interface is in a transient state. Either
          disable the wait service or mark gild{"'"}s unit{' '}
          <code>After=network.target</code> instead of{' '}
          <code>network-online</code>.
        </li>
        <li>
          <strong>Firecracker serial console capture.</strong> The host
          spawns firecracker with its stdout redirected to{' '}
          <code>{'<chroot>/serial.log'}</code> inside the jailer directory.
          This was the single biggest debugging unlock for the boot path —
          without it you cannot see why a microVM is failing to come up.
          See gild PR #69.
        </li>
        <li>
          <strong><code>/var/lib/gild/audit.db</code></strong> records{' '}
          <code>boot_ms</code>, <code>exec_ms</code>, and{' '}
          <code>exit_code</code> per run. Use it to verify warm-pool latency
          after any change to the rootfs or guest binary.
        </li>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10 not-prose">
        deka-git protocol quirks
      </h2>
      <ul className="list-disc pl-6 space-y-2">
        <li>
          <strong><code>bad band #65/#78</code> on fetch.</strong> deka-git
          {"'"}s protocol-v2 sideband implementation occasionally corrupts
          pack frames mid-fetch. Workaround: use a fresh clone with{' '}
          <code>-c protocol.version=0</code>. Tracked as deka-git issue #67.
        </li>
        <li>
          <strong>
            No <code>PUT</code>/<code>POST /merge</code> endpoint.
          </strong>{' '}
          To mark a PR merged, <code>PATCH</code>{' '}
          <code>/api/repos/.../pulls/&lt;n&gt;</code> with{' '}
          <code>{'{ "state": "merged" }'}</code>. The <code>/merge</code>{' '}
          endpoint returns 404.
        </li>
      </ul>

      <h2 className="text-2xl font-semibold text-foreground mt-10 not-prose">
        Cutover bits
      </h2>
      <ul className="list-disc pl-6 space-y-2">
        <li>
          agent-dispatcher gild routing is gated on{' '}
          <code>GILD_ROUTING_AGENTS=&lt;csv&gt;</code> env var. Default OFF.
          Setting it to <code>agent-khalid</code> (for example) routes that
          agent{"'"}s dispatched runs through gild instead of the local unix
          user.
        </li>
        <li>
          <code>/home/sami/.agent-dispatcher.env</code> on demon holds the
          dispatcher HMAC key.
        </li>
        <li>
          Agent shells use <code>~/.cargo/bin/cargo</code> and{' '}
          <code>~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu</code>{' '}
          for Rust builds.
        </li>
      </ul>
    </GildPage>
  )
}
