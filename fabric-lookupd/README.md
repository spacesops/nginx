# fabric-lookupd

Small HTTP sidecar for the members nginx site: Fabric `search_addr` verification, session cookies, and `auth_request` checks.

## Build

```bash
cd fabric-lookupd
cargo build --release
```

Binary: `target/release/fabric-lookupd`

## Run

Verified resolve shells out to the **certrelay `fabric` CLI** (same deps as the relay docs). Build it once:

```bash
cd ../../certrelay
CARGO_TARGET_DIR="$PWD/target" cargo build -p fabric-resolver --release --bin fabric
```

Or use `./scripts/run-members-stack.sh` from the nginx repo (builds `fabric` if missing).

```bash
export FABRIC_SEEDS="https://relay-atlas.spacesprotocol.org"
export FABRIC_DEV_MODE=true
export FABRIC_BIN="/path/to/certrelay/target/release/fabric"
export SESSION_SECRET="change-me"
./target/release/fabric-lookupd
```

| Env | Default | Description |
|-----|---------|-------------|
| `BIND` | `127.0.0.1:8787` | Listen address |
| `FABRIC_SEEDS` | relay-atlas | Comma-separated certrelay URLs |
| `FABRIC_DEV_MODE` | off | Set `true` for local certrelay still syncing |
| `FABRIC_TRUST_ID` | — | Optional trust pin (hex) |
| `FABRIC_ALLOWED_HANDLES` | — | Optional comma allowlist (`name@space`) for all tiers |
| `FOUNDERS_NPUBS` | — | Comma-separated npubs allowed on `/auth/verify/founders` and `founders-only.html` |
| `SESSION_SECRET` | dev placeholder | HMAC cookie signing key |
| `SESSION_MAX_AGE` | `86400` | Cookie lifetime (seconds) |
| `VERIFY_CACHE_TTL` | `60` | Per-npub verify cache (seconds) |

## Endpoints

- `GET /addrs?name=nostr&addr=<npub>` — verified handles (requires session cookie via nginx)
- `POST /auth/session` — `{ "npub", "pubkey?" }` → Set-Cookie on success
- `GET /auth/verify` — 200 + `X-Verified-Handle` for nginx `auth_request`
- `POST /auth/logout` — clear cookie

Nginx includes [`../conf/fabric-gateway.conf`](../conf/fabric-gateway.conf).
