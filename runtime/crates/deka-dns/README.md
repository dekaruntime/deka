# deka-dns

Authoritative DNS for Tana merchant domains. The crate serves DNS over UDP and DNS over HTTPS, backed by local Redis records.

## Record Types

MVP support is intentionally small:

- `A`
- `AAAA`
- `NS`
- `MX`
- `TXT`

The `tana.gg` zone apex always serves `NS` and `SOA`. Unknown names return `NXDOMAIN`.

## Configuration

Environment variables:

- `DEKA_DNS_MODE`: runtime mode. Defaults to dev behavior. Set to `production` to use production defaults.
- `DEKA_DNS_UDP_ADDR`: full UDP bind address. Overrides host/port defaults.
- `DEKA_DNS_UDP_PORT`: UDP port when `DEKA_DNS_UDP_ADDR` is unset. Defaults to `8053` in dev mode and `53` when `DEKA_DNS_MODE=production`.
- `DEKA_DNS_DOH_ADDR`: full DoH HTTP bind address. Overrides host/port defaults.
- `DEKA_DNS_DOH_PORT`: DoH port when `DEKA_DNS_DOH_ADDR` is unset. Defaults to `8080`.
- `REDIS_URL`: Redis connection URL. Defaults to `redis://127.0.0.1:6379`.
- `DEKA_DNS_ZONE`: authoritative root zone. Defaults to `tana.gg`.
- `NS1_HOST`, `NS2_HOST`: apex name server hostnames. Defaults to `ns1.tana.gg` and `ns2.tana.gg`.
- `NS1_IP`, `NS2_IP`: optional glue records for the name servers.
- `DEKA_DNS_SOA_RNAME`: SOA responsible mailbox name. Defaults to `hostmaster.tana.gg`.

Dev mode:

```bash
cargo run -p deka-dns
```

Without `DEKA_DNS_MODE=production`, the UDP listener binds port `8053`, which does not require elevated privileges.

Production mode:

```bash
DEKA_DNS_MODE=production deka-dns
```

Production mode defaults the UDP listener to port `53`. Binding port `53` requires `CAP_NET_BIND_SERVICE` on the binary, or a systemd unit that grants it with `AmbientCapabilities=CAP_NET_BIND_SERVICE`.

## Redis Schema

Subdomain mapping:

```text
subdomain:<sub> -> shop_id
```

Domain records:

```text
domain:<name>:records -> json[{ "type": "A", "name": "@", "value": "1.2.3.4", "ttl": 300 }]
```

Record `name` may be `@`, a relative owner name, or a fully qualified name. `MX` records accept `priority`.

Verification TXT:

```text
verify:<domain> -> verification token
```

Queries for `_tana-verify.<domain> TXT` return the verification token.

## Local Smoke

```bash
redis-server --port 6380 --daemonize yes
redis-cli -p 6380 SET subdomain:test shop_test
redis-cli -p 6380 SET 'domain:test.tana.gg:records' '[{"type":"A","name":"@","value":"1.2.3.4","ttl":300}]'
REDIS_URL=redis://127.0.0.1:6380 cargo run -p deka-dns
dig @127.0.0.1 -p 8053 test.tana.gg A
dig @127.0.0.1 -p 8053 tana.gg NS
```

DoH accepts `GET /dns-query?dns=<base64url-wire-query>` and `POST /dns-query` with `application/dns-message`.
