# Members stack on Ubuntu 24.04 LTS

Deploy the **members-bitvocation** site with this repository (custom nginx + `fabric-lookupd`), the **certrelay** `fabric` CLI, and the **members-bitvocation** static site (sibling checkout).

Tested target: **Ubuntu 24.04.x LTS**, **x86_64**, kernel **6.8+**.

---

## Architecture

| Component | Location | Port (default) | Notes |
|-----------|----------|----------------|-------|
| **nginx** (`objs/nginx`) | This repo | **8088** | Serves static files, `auth_request` gating, proxies `/auth/*` |
| **fabric-lookupd** | `fabric-lookupd/` | **127.0.0.1:8787** | Sessions, Fabric verify; **not** exposed publicly |
| **fabric** CLI | Sibling `certrelay/target/release/fabric` | — | Invoked by lookupd for `resolve` |

Config entry points:

- [`conf/members-bitvocation.conf`](../../conf/members-bitvocation.conf) — site server block (**edit `root`** on the server)
- [`conf/fabric-gateway.conf`](../../conf/fabric-gateway.conf) — auth tiers, `/_auth_verify_*`, `/auth/` proxy

Companion doc (site-only perspective): **members-bitvocation** repo → `docs/ubuntu-24.04.md`.

---

## Directory layout on the server

Use three sibling checkouts:

```text
/opt/bitvocation/
  nginx/                 ← this repository
  certrelay/             ← Fabric CLI
  members-bitvocation/   ← static site
```

`scripts/run-members-stack.sh` assumes `CERTRELAY_ROOT="$ROOT/../certrelay"` unless you override `CERTRELAY_ROOT` / `FABRIC_BIN`.

---

## 1. Packages

```bash
sudo apt update
sudo apt install -y \
  build-essential git curl ca-certificates pkg-config \
  libssl-dev zlib1g-dev libpcre2-dev
```

Install **Rust** via rustup (Ubuntu’s distro `rustc` is often too old for certrelay):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

First builds of `fabric` / `fabric-lookupd` can take **20–60+ minutes** and several GB of disk (risc0). **≥ 4 GB RAM** recommended; add swap on small VPSes.

---

## 2. Clone

```bash
sudo install -d -o "$USER" -g "$USER" /opt/bitvocation
cd /opt/bitvocation

git clone <YOUR_NGINX_REMOTE>       nginx
git clone <YOUR_CERTRELAY_REMOTE>   certrelay
git clone <YOUR_MEMBERS_REMOTE>     members-bitvocation
```

---

## 3. Configure nginx site root

Edit **`conf/members-bitvocation.conf`** before going live:

```nginx
root   /opt/bitvocation/members-bitvocation;
server_name members.yourdomain.example;   # optional
listen       8088;
```

Test:

```bash
cd /opt/bitvocation/nginx
./objs/nginx -p "$(pwd)" -c conf/members-bitvocation.conf -t
```

---

## 4. Build nginx (auth_request)

From this repo:

```bash
cd /opt/bitvocation/nginx
auto/configure --with-http_auth_request_module --with-http_ssl_module
make -j"$(nproc)"
```

Binary: `objs/nginx`. Prefix **`-p "$(pwd)"`** is required when starting (see scripts).

---

## 5. Build certrelay + fabric-lookupd

```bash
cd /opt/bitvocation/certrelay
CARGO_TARGET_DIR="$PWD/target" cargo build -p fabric-resolver --release --bin fabric

cd /opt/bitvocation/nginx/fabric-lookupd
env -u CARGO_TARGET_DIR cargo build --release --target-dir target
```

Verify:

```bash
test -x /opt/bitvocation/certrelay/target/release/fabric
test -x /opt/bitvocation/nginx/fabric-lookupd/target/release/fabric-lookupd
```

Always build lookupd with **`env -u CARGO_TARGET_DIR`** and **`--target-dir target`** so `scripts/run-members-stack.sh` runs the correct binary.

**Small VPS or Mac-only builder:** skip §5 on the server and build Linux binaries in Docker, then `scp` to the VPS — see [docker-build-ubuntu-24.04.md](docker-build-ubuntu-24.04.md). Nginx (§4) is still lightweight enough to build on the VPS or in the same container.

---

## 6. Environment file

Create **`members-stack.env`** in this repo (mode `600`; copy from [`members-stack.env.example`](../../members-stack.env.example)):

```bash
SESSION_SECRET='<openssl rand -hex 32>'
FABRIC_SEEDS='https://relay-atlas.spacesprotocol.org'
FABRIC_DEV_MODE='false'
FABRIC_SEARCH_TIMEOUT='120'

# Optional tiers
FOUNDERS_NPUBS='npub1...'
MEMBER_SPACE_SUFFIXES='bitvocation,bitcoin'
```

See [`fabric-lookupd/README.md`](../../fabric-lookupd/README.md) for all variables.

---

## 7. Service user (Ubuntu)

```bash
sudo chown -R www-data:www-data /opt/bitvocation/members-bitvocation
sudo install -d -o www-data -g www-data /opt/bitvocation/nginx/logs
sudo chown www-data:www-data /opt/bitvocation/nginx/members-stack.env
sudo chmod 600 /opt/bitvocation/nginx/members-stack.env
sudo chown -R www-data:www-data \
  /opt/bitvocation/nginx/fabric-lookupd/target \
  /opt/bitvocation/certrelay/target
```

---

## 8. Start / stop (scripts)

```bash
cd /opt/bitvocation/nginx
sudo -u www-data env $(grep -v '^#' members-stack.env | xargs) \
  ./scripts/run-members-stack.sh
```

Smoke tests:

```bash
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8787/auth/verify          # 401
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8787/auth/verify/members  # 401, not 404
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8088/                      # 200
```

Stop:

```bash
sudo -u www-data ./scripts/stop-members-stack.sh
```

`run-members-stack.sh` will restart **stale** lookupd if `/auth/verify/members` returns **404**, and **reload** nginx if it is already running.

---

## 9. systemd

`/etc/systemd/system/members-bitvocation-stack.service`:

```ini
[Unit]
Description=Bitvocation members site (nginx + fabric-lookupd)
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
User=www-data
Group=www-data
WorkingDirectory=/opt/bitvocation/nginx
EnvironmentFile=/opt/bitvocation/nginx/members-stack.env
ExecStart=/opt/bitvocation/nginx/scripts/run-members-stack.sh
ExecStop=/opt/bitvocation/nginx/scripts/stop-members-stack.sh
TimeoutStartSec=300
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now members-bitvocation-stack.service
```

Logs: `logs/error.log`, `logs/lookupd.log`.

---

## 10. Firewall

```bash
sudo ufw allow OpenSSH
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw enable
```

Do **not** open **8787** on the public interface.

---

## 11. HTTPS (Ubuntu nginx + Certbot front)

Keep this stack on **127.0.0.1:8088**; terminate TLS with distro nginx:

```bash
sudo apt install -y nginx certbot python3-certbot-nginx
```

Example `/etc/nginx/sites-available/members-bitvocation`:

```nginx
server {
    listen 80;
    server_name members.yourdomain.example;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    server_name members.yourdomain.example;

    ssl_certificate     /etc/letsencrypt/live/members.yourdomain.example/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/members.yourdomain.example/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8088;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 180s;
        proxy_send_timeout 180s;
    }
}
```

```bash
sudo ln -sf /etc/nginx/sites-available/members-bitvocation /etc/nginx/sites-enabled/
sudo nginx -t
sudo certbot --nginx -d members.yourdomain.example
```

`/auth/` in [`fabric-gateway.conf`](../../conf/fabric-gateway.conf) already uses **180s** proxy read timeout for long verifies.

---

## 12. Access tiers (this config)

| Tier | nginx | lookupd |
|------|--------|---------|
| Public gate | `gate.js`, `index.html` | `POST /auth/session` |
| Verified `@space` | `/_auth_verify` | `GET /auth/verify` |
| `@bitvocation` / `@bitcoin` | `/_auth_verify_members` | `GET /auth/verify/members` + `MEMBER_SPACE_SUFFIXES` |
| Founders | `/_auth_verify_founders` | `GET /auth/verify/founders` + `FOUNDERS_NPUBS` |

---

## 13. Updates

```bash
cd /opt/bitvocation/nginx && git pull
cd /opt/bitvocation/members-bitvocation && git pull

# If Rust changed:
cd /opt/bitvocation/nginx/fabric-lookupd
env -u CARGO_TARGET_DIR cargo build --release --target-dir target

sudo systemctl restart members-bitvocation-stack.service
```

---

## 14. Troubleshooting

| Symptom | Cause / fix |
|---------|----------------|
| **500** on `members-only.html` | lookupd missing `/auth/verify/members` — rebuild lookupd, run `run-members-stack.sh` |
| **404** on static files | Wrong `root` in `members-bitvocation.conf` |
| Sign-in **504** / timeout | Increase `FABRIC_SEARCH_TIMEOUT`; ensure front proxy read timeout ≥ 180s |
| `auth request unexpected status: 404` | Stale lookupd binary on port 8787 |

---

## Checklist

- [ ] Siblings under `/opt/bitvocation/`
- [ ] `root` in `conf/members-bitvocation.conf` updated
- [ ] `objs/nginx` built with `http_auth_request_module`
- [ ] `fabric` + `fabric-lookupd` built (`env -u CARGO_TARGET_DIR`)
- [ ] `members-stack.env` with strong `SESSION_SECRET`
- [ ] systemd unit enabled
- [ ] UFW: 443 (8787 not public)
- [ ] Optional: Certbot → `127.0.0.1:8088`
