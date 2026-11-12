#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NGINX_CONF=conf/members-bitvocation.conf
LOOKUPD="$ROOT/fabric-lookupd/target/release/fabric-lookupd"
MEMBERS_PORT="${MEMBERS_PORT:-8088}"
LOOKUPD_PORT="${LOOKUPD_PORT:-8787}"

stopped_any=0

read_nginx_pid() {
  if [[ ! -f logs/nginx.pid ]]; then
    return 1
  fi
  local pid
  pid="$(tr -d '[:space:]' <logs/nginx.pid)"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  echo "$pid"
}

stop_nginx() {
  if [[ ! -x ./objs/nginx ]]; then
    return 0
  fi

  if pid="$(read_nginx_pid)" && kill -0 "$pid" 2>/dev/null; then
    if ./objs/nginx -p "$ROOT" -c "$NGINX_CONF" -s quit 2>/dev/null; then
      echo "Sent quit to nginx (pid ${pid})."
      stopped_any=1
      return 0
    fi
  fi

  local pattern="objs/nginx -p ${ROOT} -c ${NGINX_CONF}"
  if pgrep -f "$pattern" >/dev/null 2>&1; then
    pkill -TERM -f "$pattern" 2>/dev/null || true
    sleep 0.5
    pkill -KILL -f "$pattern" 2>/dev/null || true
    echo "Stopped nginx for members-bitvocation."
    stopped_any=1
    return 0
  fi

  if lsof -nP -iTCP:"$MEMBERS_PORT" -sTCP:LISTEN 2>/dev/null | grep -q nginx; then
    echo "Warning: something still listening on port ${MEMBERS_PORT}; not this repo's nginx pid file."
  fi
}

stop_lookupd() {
  local pattern="fabric-lookupd/target/release/fabric-lookupd"
  if ! pgrep -f "$pattern" >/dev/null 2>&1; then
    if lsof -nP -iTCP:"$LOOKUPD_PORT" -sTCP:LISTEN 2>/dev/null | grep -q fabric-lookupd; then
      echo "Warning: port ${LOOKUPD_PORT} in use but process path did not match ${LOOKUPD}"
    fi
    return 0
  fi
  pkill -TERM -f "$pattern" 2>/dev/null || true
  sleep 0.5
  pkill -KILL -f "$pattern" 2>/dev/null || true
  echo "Stopped fabric-lookupd."
  stopped_any=1
}

stop_nginx
stop_lookupd

if [[ "$stopped_any" -eq 0 ]]; then
  echo "No members stack processes were running."
fi
