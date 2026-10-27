#!/usr/bin/env bash
# A throwaway PostgreSQL 18 cluster for Keel's database tests (055), with no Docker: initdb into a
# private directory, listen only on a Unix socket in that directory, never on TCP.
#
#   eval "$(scripts/keel-test-db.sh start)"   prints `export KEEL_TEST_DATABASE_URL=...`
#   scripts/keel-test-db.sh stop              stops the cluster and deletes its directory
#   scripts/keel-test-db.sh status
#
# Env:
#   KEEL_TEST_DB_DIR  cluster directory (default: $XDG_RUNTIME_DIR/keel-test-db, else /tmp/keel-test-db-$UID)
#   PG_BIN            PostgreSQL binaries (default: /usr/lib/postgresql/18/bin)
#
# Each test creates and drops its own schema, so one cluster serves every test and agent.
set -euo pipefail

PG_BIN="${PG_BIN:-/usr/lib/postgresql/18/bin}"
DIR="${KEEL_TEST_DB_DIR:-${XDG_RUNTIME_DIR:-/tmp}/keel-test-db}"
[[ -n "${XDG_RUNTIME_DIR:-}" || -n "${KEEL_TEST_DB_DIR:-}" ]] || DIR="/tmp/keel-test-db-$UID"
DATA="$DIR/data"
SOCK="$DIR/sock"
DB=keel_test
# Explicit, because the packaged `psql` wrapper may default to another cluster's port.
PORT=5432

url() {
  # sqlx reads the socket directory from `host`, URL-encoded.
  local host
  host=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=""))' "$SOCK")
  echo "postgres://$(id -un)@${host}:${PORT}/${DB}"
}

case "${1:-status}" in
  start)
    if [[ ! -d "$DATA" ]]; then
      mkdir -p "$SOCK"
      chmod 700 "$DIR"
      "$PG_BIN/initdb" -D "$DATA" -A trust -U "$(id -un)" --no-sync >/dev/null
    fi
    if ! "$PG_BIN/pg_ctl" -D "$DATA" status >/dev/null 2>&1; then
      "$PG_BIN/pg_ctl" -D "$DATA" -l "$DIR/log" -w -o "-k $SOCK -p $PORT -c listen_addresses='' -c fsync=off" start >/dev/null
    fi
    "$PG_BIN/psql" -h "$SOCK" -p "$PORT" -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='$DB'" | grep -q 1 \
      || "$PG_BIN/createdb" -h "$SOCK" -p "$PORT" "$DB"
    echo "export KEEL_TEST_DATABASE_URL='$(url)'"
    ;;
  stop)
    if [[ -d "$DATA" ]]; then
      "$PG_BIN/pg_ctl" -D "$DATA" -m fast stop >/dev/null 2>&1 || true
      rm -rf "$DIR"
    fi
    echo "keel-test-db: stopped"
    ;;
  status)
    if [[ -d "$DATA" ]] && "$PG_BIN/pg_ctl" -D "$DATA" status >/dev/null 2>&1; then
      echo "keel-test-db: running, KEEL_TEST_DATABASE_URL='$(url)'"
    else
      echo "keel-test-db: not running"
    fi
    ;;
  *) echo "usage: $0 start|stop|status" >&2; exit 2 ;;
esac
