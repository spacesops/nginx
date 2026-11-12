#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NGINX_CONF=conf/members-bitvocation.conf
LOOKUPD="$ROOT/fabric-lookupd/target/release/fabric-lookupd"
CERTRELAY_ROOT="${CERTRELAY_ROOT:-$ROOT/../certrelay}"
FABRIC_CLI="$CERTRELAY_ROOT/target/release/fabric"
MEMBERS_PORT="${MEMBERS_PORT:-8088}"
LOOKUPD_PORT="${LOOKUPD_PORT:-8787}"

if [[ ! -x "$LOOKUPD" ]]; then
  echo "Build fabric-lookupd first:"
  echo "  (cd fabric-lookupd && cargo build --release --target-dir target)"
  exit 1
fi

export FABRIC_SEEDS="${FABRIC_SEEDS:-https://relay-atlas.spacesprotocol.org}"
export SESSION_SECRET="${SESSION_SECRET:-dev-insecure-change-in-production}"
export FABRIC_SEARCH_TIMEOUT="${FABRIC_SEARCH_TIMEOUT:-120}"
export FABRIC_DEV_MODE="${FABRIC_DEV_MODE:-true}"
export FABRIC_BIN="$FABRIC_CLI"
mkdir -p logs

if [[ ! -x "$FABRIC_CLI" ]]; then
  echo "Building fabric CLI in certrelay (needed for verified resolve)…"
  (cd "$CERTRELAY_ROOT" && CARGO_TARGET_DIR="$CERTRELAY_ROOT/target" \
    cargo build -p fabric-resolver --release --bin fabric)
fi
if [[ ! -x "$FABRIC_CLI" ]]; then
  echo "fabric CLI not found at $FABRIC_CLI"
  exit 1
fi

lookupd_up() {
  local code
  code="$(curl --connect-timeout 2 -m 3 -s -o /dev/null -w "%{http_code}" \
    "http://127.0.0.1:${LOOKUPD_PORT}/auth/verify" 2>/dev/null || true)"
  [[ "$code" == "401" || "$code" == "200" || "$code" == "403" ]]
}

port_listening() {
  local port=$1
  if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    return 0
  fi
  return 1
}

if ! lookupd_up; then
  if port_listening "$LOOKUPD_PORT"; then
    echo "Port ${LOOKUPD_PORT} is in use but fabric-lookupd did not respond; fix or stop that process first."
    exit 1
  fi
  echo "Starting fabric-lookupd on 127.0.0.1:${LOOKUPD_PORT}..."
  BIND="127.0.0.1:${LOOKUPD_PORT}" "$LOOKUPD" >>logs/lookupd.log 2>&1 &
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    sleep 0.3
    lookupd_up && break
  done
  if ! lookupd_up; then
    echo "fabric-lookupd failed to start; see logs/lookupd.log"
    exit 1
  fi
else
  echo "fabric-lookupd already listening on 127.0.0.1:${LOOKUPD_PORT}."
fi

members_nginx_running() {
  local pattern="objs/nginx -p ${ROOT} -c ${NGINX_CONF}"
  pgrep -f "$pattern" >/dev/null 2>&1
}

if members_nginx_running; then
  echo "Members nginx already running for ${NGINX_CONF}."
  echo "Site: http://127.0.0.1:${MEMBERS_PORT}/"
  exit 0
fi

if port_listening "$MEMBERS_PORT"; then
  echo "Port ${MEMBERS_PORT} is already in use."
  echo "Run ./scripts/stop-members-stack.sh first, or set MEMBERS_PORT to another port."
  exit 1
fi

if [[ "${MEMBERS_FOREGROUND:-0}" == "1" ]]; then
  echo "nginx foreground (Ctrl+C to stop); site http://127.0.0.1:${MEMBERS_PORT}/"
  exec ./objs/nginx -p "$ROOT" -c "$NGINX_CONF" -g 'daemon off;'
fi

./objs/nginx -p "$ROOT" -c "$NGINX_CONF"
echo "Members stack running:"
echo "  site:    http://127.0.0.1:${MEMBERS_PORT}/"
echo "  lookupd: http://127.0.0.1:${LOOKUPD_PORT}/"
echo "Stop with: ./scripts/stop-members-stack.sh"
echo "Foreground nginx: MEMBERS_FOREGROUND=1 $0"
