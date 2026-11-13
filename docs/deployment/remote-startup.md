# Remote server startup (`ssh nginx`)

Quick reference for **bitvocation.timeintosats.com** on the VPS (prefix **`/root`**, custom **`/usr/local/nginx/sbin/nginx`**).

Paths assume:

| Path | Purpose |
|------|---------|
| `/root/nginx.conf` | Main config (`conf/root-nginx.conf` in repo) |
| `/root/nginx/conf/` | `fabric-gateway.conf`, `spaces/*.conf`, `mime.types` |
| `/var/www/members-bitvocation` | Static site |
| `/opt/bitvocation/nginx` | `fabric-lookupd`, `members-stack.env`, `logs/` |
| `/opt/bitvocation/certrelay/target/release/fabric` | Fabric CLI for lookupd |

Binary must include **`--with-http_auth_request_module`** and **`--with-http_ssl_module`**:

```bash
/usr/local/nginx/sbin/nginx -V 2>&1 | grep -E 'auth_request|ssl'
```

---

## 1. fabric-lookupd (first)

```bash
cd /opt/bitvocation/nginx
set -a && source ./members-stack.env && set +a
export BIND=127.0.0.1:8787
export FABRIC_BIN=/opt/bitvocation/certrelay/target/release/fabric

# foreground (debug):
/opt/bitvocation/nginx/fabric-lookupd/target/release/fabric-lookupd

# background:
nohup /opt/bitvocation/nginx/fabric-lookupd/target/release/fabric-lookupd \
  >> logs/lookupd.log 2>&1 &
```

Smoke test:

```bash
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8787/auth/verify/members
# 401 = OK (not 404)
```

Optional env (in `members-stack.env`):

- `SESSION_SECRET` — required in production  
- `FOUNDERS_NPUBS` — founders-only npub list  
- `MEMBER_SPACE_SUFFIXES` — default `bitvocation,bitcoin`  

---

## 2. nginx (second)

**Do not** use `/root/remote_nginx.conf` as `-c` (that is an `nginx -T` dump). Use **`/root/nginx.conf`**.

```bash
/usr/local/nginx/sbin/nginx -t -c /root/nginx.conf -p /root
```

Foreground (tmux/systemd):

```bash
/usr/local/nginx/sbin/nginx -c /root/nginx.conf -p /root -g 'daemon off;'
```

Daemon (default):

```bash
/usr/local/nginx/sbin/nginx -c /root/nginx.conf -p /root
```

Reload after config change:

```bash
/usr/local/nginx/sbin/nginx -p /root -c /root/nginx.conf -s reload
```

Stop:

```bash
/usr/local/nginx/sbin/nginx -p /root -c /root/nginx.conf -s quit
```

---

## 3. Smoke tests

```bash
curl -sI -H 'Host: bitvocation.timeintosats.com' http://127.0.0.1/    # expect 301 → HTTPS
curl -sI -k -H 'Host: bitvocation.timeintosats.com' https://127.0.0.1/  # expect 200
```

---

## 4. Deploy config from Mac

```bash
export GIT=/Users/i830671/git
scp $GIT/nginx/conf/root-nginx.conf nginx:/root/nginx.conf
scp $GIT/nginx/conf/fabric-gateway.conf nginx:/root/nginx/conf/fabric-gateway.conf
scp $GIT/nginx/conf/spaces/members@bitvocation.conf nginx:/root/nginx/conf/spaces/
ssh nginx '/usr/local/nginx/sbin/nginx -t -c /root/nginx.conf -p /root'
```

Site files: sync **`members-bitvocation`** to `/var/www/members-bitvocation`.

Binaries: see [docker-build-ubuntu-24.04.md](docker-build-ubuntu-24.04.md) → `scp objs/nginx`, `fabric-lookupd`, `fabric`.

---

## 5. Common issues

| Symptom | Fix |
|---------|-----|
| `host not found in upstream "fabric_lookup"` | Start with **`-c /root/nginx.conf`**; ensure `upstream fabric_lookup` in `http {}` |
| `ssl` parameter requires ssl module | Rebuild nginx with `--with-http_ssl_module` |
| `unknown directive "http2"` | Remove `http2 on;` or rebuild with `--with-http_v2_module` |
| `auth request unexpected status: 404` | Restart lookupd with current binary; check `/auth/verify/members` |

Dump effective config (read-only):

```bash
/usr/local/nginx/sbin/nginx -T -c /root/nginx.conf -p /root 2>&1 | less
```
