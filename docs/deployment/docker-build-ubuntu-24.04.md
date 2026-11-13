# Docker build for Ubuntu 24.04.5 LTS (x86_64)

Cross-build **Linux amd64** binaries on a Mac or other host when a small VPS cannot compile **certrelay** / **fabric-lookupd** (risc0, long compile, high RAM).

**Target runtime:** Ubuntu 24.04.5 LTS, kernel 6.8.x, **x86_64** (typical VPS).

**Artifacts (written via bind mount into your git tree):**

| Binary | Path after build |
|--------|------------------|
| `fabric` | `certrelay/target/release/fabric` |
| `fabric-lookupd` | `nginx/fabric-lookupd/target/release/fabric-lookupd` |
| `nginx` (**required**) | `nginx/objs/nginx` |

The members stack needs **`http_auth_request`** (`fabric-gateway.conf`). A distro or `/usr/local/nginx` build without that module will not work:

```bash
/usr/local/nginx/sbin/nginx -V 2>&1 | grep auth_request   # must show a configure flag
```

If that prints nothing, build **this** nginx tree in Docker (§2) and deploy `objs/nginx`.

Production deploy paths match [ubuntu-24.04.md](ubuntu-24.04.md) (`/opt/bitvocation/...`). **macOS `target/release` binaries do not run on Linux** — use this flow or build natively on Ubuntu.

---

## Prerequisites

- Docker (Docker Desktop on Mac).
- Three **sibling** repositories on the host, e.g.:

```text
$GIT_PARENT/
  nginx/                 ← this repository
  certrelay/
  members-bitvocation/
```

`fabric-lookupd` depends on `../../certrelay/fabric`; keep that layout inside the container mount.

- On **Apple Silicon**, always use `--platform linux/amd64` so output matches an **x86_64** VPS.

---

## 1. Interactive container

Set `GIT_PARENT` to the directory that contains `nginx`, `certrelay`, and `members-bitvocation`:

```bash
export GIT_PARENT=/path/to/parent   # e.g. /Users/you/git or /opt/bitvocation

docker run --rm -it \
  --platform linux/amd64 \
  -v "${GIT_PARENT}:/work" \
  -w /work \
  ubuntu:24.04 \
  bash
```

Container paths: `/work/nginx`, `/work/certrelay`, `/work/members-bitvocation`.

---

## 2. Build inside the container

Paste in the shell started above:

```bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y \
  build-essential git curl ca-certificates pkg-config \
  libssl-dev zlib1g-dev libpcre2-dev

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustc -Vv | grep host   # expect x86_64-unknown-linux-gnu

# certrelay fabric CLI
cd /work/certrelay
CARGO_TARGET_DIR="$PWD/target" \
  cargo build -p fabric-resolver --release --bin fabric
test -x "$PWD/target/release/fabric"

# fabric-lookupd (must use target-dir target; see run-members-stack.sh)
cd /work/nginx/fabric-lookupd
env -u CARGO_TARGET_DIR cargo build --release --target-dir target
test -x "$PWD/target/release/fabric-lookupd"

# nginx (auth_request required for members stack)
cd /work/nginx
auto/configure --with-http_auth_request_module --with-http_ssl_module
make -j"$(nproc)"
test -x "$PWD/objs/nginx"
./objs/nginx -V 2>&1 | grep -q auth_request
./objs/nginx -V 2>&1 | grep -q http_ssl_module

file /work/certrelay/target/release/fabric
file /work/nginx/fabric-lookupd/target/release/fabric-lookupd
file /work/nginx/objs/nginx
```

**Resources:** first build often **30–90+ minutes** and **several GB** disk. Give Docker **≥ 8 GB RAM**. On a tiny build environment, add temporary swap inside the container (same steps as on bare metal).

Exit the container when finished; artifacts remain on the host under `$GIT_PARENT`.

---

## 3. Verify on the host

```bash
file "${GIT_PARENT}/certrelay/target/release/fabric"
file "${GIT_PARENT}/nginx/fabric-lookupd/target/release/fabric-lookupd"
file "${GIT_PARENT}/nginx/objs/nginx"
```

Expect: `ELF 64-bit LSB ... x86-64`, not `Mach-O`.

Confirm **auth_request** in the nginx binary (on Mac the binary is Linux ELF from Docker; run inside container if ` -V` fails on macOS):

```bash
docker run --rm --platform linux/amd64 \
  -v "${GIT_PARENT}/nginx:/work/nginx" \
  ubuntu:24.04 \
  /work/nginx/objs/nginx -V 2>&1 | grep auth_request
```

---

## 4. Copy binaries to the VPS

Use **`scp`** with an SSH host alias (example below: **`nginx`** → your VPS in `~/.ssh/config`).

On your **build host**, set paths you already verified with `file`:

```bash
export GIT_PARENT=/path/to/parent   # parent of nginx/, certrelay/, members-bitvocation/
```

Create target directories on the remote (once):

```bash
ssh nginx 'mkdir -p /opt/bitvocation/certrelay/target/release \
  /opt/bitvocation/nginx/fabric-lookupd/target/release \
  /opt/bitvocation/nginx/objs'
```

Copy the Rust binaries:

```bash
scp "${GIT_PARENT}/certrelay/target/release/fabric" \
  nginx:/opt/bitvocation/certrelay/target/release/fabric

scp "${GIT_PARENT}/nginx/fabric-lookupd/target/release/fabric-lookupd" \
  nginx:/opt/bitvocation/nginx/fabric-lookupd/target/release/fabric-lookupd

scp "${GIT_PARENT}/nginx/objs/nginx" \
  nginx:/opt/bitvocation/nginx/objs/nginx
```

On the **VPS** (or via `ssh nginx '...'`):

```bash
chmod +x /opt/bitvocation/certrelay/target/release/fabric
chmod +x /opt/bitvocation/nginx/fabric-lookupd/target/release/fabric-lookupd
chmod +x /opt/bitvocation/nginx/objs/nginx
/opt/bitvocation/nginx/objs/nginx -V 2>&1 | grep auth_request
```

Clone **nginx** (`conf/`, `scripts/`), **certrelay** (paths only), and **members-bitvocation** on the VPS. **Do not** run `cargo` on the VPS unless you are upgrading Rust deps there.

### Use the new nginx binary on the VPS

**Option A — repo layout (matches `run-members-stack.sh`):**

```bash
cd /opt/bitvocation/nginx
./objs/nginx -p "$(pwd)" -c conf/members-bitvocation.conf -t
# start via ./scripts/run-members-stack.sh, or foreground:
# ./objs/nginx -p "$(pwd)" -c conf/members-bitvocation.conf -g 'daemon off;'
```

**Option B — keep `/usr/local/nginx/sbin/nginx` path**

Stop nginx, back up the old binary, then replace:

```bash
ssh nginx 'systemctl stop nginx 2>/dev/null || true'
ssh nginx 'cp -a /usr/local/nginx/sbin/nginx /usr/local/nginx/sbin/nginx.bak'
scp "${GIT_PARENT}/nginx/objs/nginx" nginx:/usr/local/nginx/sbin/nginx
ssh nginx 'chmod +x /usr/local/nginx/sbin/nginx'
ssh nginx '/usr/local/nginx/sbin/nginx -V 2>&1 | grep auth_request'
```

Use **`-p`** and **`-c`** so `conf/members-bitvocation.conf` and `logs/` resolve correctly (e.g. `-p /opt/bitvocation/nginx -c conf/members-bitvocation.conf`). A lone `/root/nginx.conf` without `auth_request` locations from this repo will not gate the site.

---

## 5. Start stack on the VPS

See [ubuntu-24.04.md](ubuntu-24.04.md) for `members-stack.env`, `www-data`, and systemd.

```bash
cd /opt/bitvocation/nginx
export FABRIC_BIN=/opt/bitvocation/certrelay/target/release/fabric
set -a && source ./members-stack.env && set +a
./scripts/run-members-stack.sh
```

Smoke test:

```bash
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8787/auth/verify/members
```

**401** without a cookie is correct; **404** means a stale or wrong lookupd binary on port 8787.

---

## 6. Non-interactive build (one shot)

```bash
export GIT_PARENT=/path/to/parent

docker run --rm \
  --platform linux/amd64 \
  -v "${GIT_PARENT}:/work" \
  -w /work \
  ubuntu:24.04 \
  bash -c 'set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential curl ca-certificates pkg-config libssl-dev zlib1g-dev libpcre2-dev
curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
cd /work/certrelay && CARGO_TARGET_DIR="$PWD/target" cargo build -p fabric-resolver --release --bin fabric
cd /work/nginx/fabric-lookupd && env -u CARGO_TARGET_DIR cargo build --release --target-dir target
cd /work/nginx && auto/configure --with-http_auth_request_module --with-http_ssl_module && make -j"$(nproc)"
/work/nginx/objs/nginx -V 2>&1 | grep -q auth_request || exit 1
file /work/certrelay/target/release/fabric /work/nginx/fabric-lookupd/target/release/fabric-lookupd /work/nginx/objs/nginx'
```

---

## 7. When to rebuild

- `certrelay`, `fabric-lookupd`, or **nginx** source / configure flags change.
- You change the Ubuntu/glibc baseline (keep **ubuntu:24.04** for 24.04.5 VPS).

After rebuilding, **`scp` all three binaries** again and run `run-members-stack.sh` (or restart your nginx unit). Re-check `nginx -V | grep auth_request` on the VPS.

---

## Related

- Full server setup: [ubuntu-24.04.md](ubuntu-24.04.md)
- Static site only: **members-bitvocation** → `docs/ubuntu-24.04.md`
