#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
script="$script_dir/gild-agent-migrate-groups.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/bin" "$tmp/state/primary"
cat >"$tmp/state/passwd" <<'PASSWD'
root:x:0:0:root:/root:/bin/bash
agent-primary:x:2001:777::/nonexistent:/usr/sbin/nologin
PASSWD
cat >"$tmp/state/groups" <<'GROUPS'
gild-orchestrator:x:777:
GROUPS
printf '%s\n' gild-orchestrator >"$tmp/state/primary/agent-primary"

cat >"$tmp/bin/getent" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  passwd)
    cat "$MIGRATE_TEST_STATE/passwd"
    ;;
  group)
    if [[ "$#" -eq 2 ]]; then
      grep -E "^$2:" "$MIGRATE_TEST_STATE/groups"
    else
      cat "$MIGRATE_TEST_STATE/groups"
    fi
    ;;
  *)
    exit 2
    ;;
esac
STUB

cat >"$tmp/bin/id" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
mode="$1"
user="$2"
primary=$(cat "$MIGRATE_TEST_STATE/primary/$user")
case "$mode" in
  -gn)
    printf '%s\n' "$primary"
    ;;
  -nG)
    {
      printf '%s\n' "$primary"
      awk -F: -v user="$user" '
        $4 != "" {
          split($4, members, ",")
          for (i in members) {
            if (members[i] == user) print $1
          }
        }
      ' "$MIGRATE_TEST_STATE/groups"
    } | awk '!seen[$0]++' | paste -sd' ' -
    printf '\n'
    ;;
  *)
    exit 2
    ;;
esac
STUB

cat >"$tmp/bin/groupadd" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
group="${@: -1}"
if ! grep -Eq "^$group:" "$MIGRATE_TEST_STATE/groups"; then
  printf '%s:x:778:\n' "$group" >>"$MIGRATE_TEST_STATE/groups"
fi
STUB

cat >"$tmp/bin/usermod" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == "-g" ]]
printf '%s\n' "$2" >"$MIGRATE_TEST_STATE/primary/$3"
STUB

cat >"$tmp/bin/gpasswd" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
op="$1"
user="$2"
group="$3"
awk -F: -v OFS=: -v op="$op" -v user="$user" -v group="$group" '
  $1 == group {
    n = split($4, members, ",")
    out = ""
    found = 0
    for (i = 1; i <= n; i++) {
      if (members[i] == "") continue
      if (members[i] == user) {
        found = 1
        if (op == "-d") continue
      }
      out = out (out == "" ? "" : ",") members[i]
    }
    if (op == "-a" && !found) {
      out = out (out == "" ? "" : ",") user
    }
    $4 = out
  }
  { print }
' "$MIGRATE_TEST_STATE/groups" >"$MIGRATE_TEST_STATE/groups.next"
mv "$MIGRATE_TEST_STATE/groups.next" "$MIGRATE_TEST_STATE/groups"
STUB

chmod +x "$tmp/bin/getent" "$tmp/bin/id" "$tmp/bin/groupadd" "$tmp/bin/usermod" "$tmp/bin/gpasswd"

export MIGRATE_TEST_STATE="$tmp/state"
export PATH="$tmp/bin:$PATH"

first_output=$("$script")
grep -qx '\[migrate\] created group gild-agents' <<<"$first_output"
grep -qx '\[migrate\] agent-primary: primary group gild-orchestrator -> gild-agents' <<<"$first_output"
[[ "$(id -gn agent-primary)" == "gild-agents" ]]
groups=$(id -nG agent-primary | tr ' ' '\n')
grep -qx gild-agents <<<"$groups"
! grep -qx gild-orchestrator <<<"$groups"

second_output=$("$script")
[[ -z "$second_output" ]]
