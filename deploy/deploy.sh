#!/usr/bin/env bash
#
# Deploys a specific tagged release of the web API to the VPS.
#
#   bash /opt/xindeler-web-api/src/deploy/deploy.sh v1.0.0
#
# Deploys are tag-based, cut from `main` only -- same shape as
# xindeler-zuul's deploy.sh (mirrored here for consistency across the three
# backend services on this VPS), adapted for xindeler-web-api running as
# the shared `mgrinberg` user instead of a dedicated service account, and on
# port 8020 behind nginx's `/api/` proxy instead of its own domain. A tag is
# also the rollback unit: to roll back, re-run this script with an older
# tag. The previous-binary fallback below only ever remembers one step back
# and doesn't survive a VPS/disk issue, so it's a fast automatic safety net
# for "the new build came up unhealthy", not the primary way to roll back.
#
# Takes a database backup, keeps the previous binary, health-checks after
# restarting, and restores the previous binary automatically if the new one
# does not come up.

set -euo pipefail

TAG="${1:?usage: deploy.sh <tag>, e.g. deploy.sh v1.0.0 -- see git tag -l for available tags}"

ROOT="${WEB_API_ROOT:-/opt/xindeler-web-api}"
SRC="$ROOT/src"
BIN="$ROOT/xindeler-web-api-server"
PREVIOUS="$ROOT/xindeler-web-api-server.previous"
DATA="$ROOT/data"
HEALTH_URL="${WEB_API_HEALTH_URL:-http://127.0.0.1:8020/ping}"
# NOT /api/ping or /ping: nginx only proxies paths under /api/ to this
# service (see xindeler.com's site config) -- /ping itself has no route
# there and falls through to the frontend's SPA catch-all, returning a 200
# of unrelated HTML instead of this service's "pong" (confirmed against
# production, not assumed). /api/waitlist/count is a real proxied GET with
# no side effects, always available, so it actually proves the public path
# end to end instead of silently no-oping on a 200 from the wrong app.
PUBLIC_URL="${WEB_API_PUBLIC_HEALTH_URL:-https://xindeler.com/api/waitlist/count}"
HEALTH_TIMEOUT=60
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
SERVICE_UNIT="${WEB_API_SERVICE_UNIT:-xindeler-web-api.service}"
SUDOERS_HINT="mgrinberg ALL=(root) NOPASSWD: /usr/bin/systemctl restart $SERVICE_UNIT"

log() { echo "[deploy] $*"; }
fail() { echo "[deploy] ERROR: $*" >&2; exit 1; }

# Restarts the service.
#
# Prefers systemd, which sends SIGTERM and lets the server drain in-flight
# requests. Falls back to SIGKILL, which systemd sees as a failure and so
# triggers Restart=on-failure.
#
# The fallback MUST stay SIGKILL: the server shuts down gracefully and exits
# 0 on SIGTERM, and Restart=on-failure does not restart a clean exit, so
# `pkill -TERM` here would stop the service and never bring it back.
#
# SERVICE_UNIT must match the sudoers rule *character for character*,
# including the .service suffix: sudo matches the full command line, so
# `systemctl restart xindeler-web-api` does not satisfy a rule written for
# `systemctl restart xindeler-web-api.service`. Getting this wrong is
# silent — the deploy just falls back to SIGKILL and keeps working.
restart_service() {
    if sudo -n systemctl restart "$SERVICE_UNIT" 2>/dev/null; then
        log "restarted via systemd (graceful shutdown)"
    else
        log "no passwordless sudo for '$SERVICE_UNIT'; restarting via SIGKILL"
        log "  (grant it with: $SUDOERS_HINT)"
        pkill -9 -f "xindeler-web-api-server$" || true
    fi
}

wait_until_healthy() {
    local deadline=$((SECONDS + HEALTH_TIMEOUT))
    while [ $SECONDS -lt $deadline ]; do
        if curl -fsS --max-time 2 "$HEALTH_URL" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    return 1
}

rollback() {
    log "restoring the previous binary"
    [ -f "$PREVIOUS" ] || fail "no previous binary to restore; recover manually"
    cp "$PREVIOUS" "$BIN.restoring"
    mv "$BIN.restoring" "$BIN"
    restart_service
    if wait_until_healthy; then
        fail "deploy failed and was rolled back; the previous build is serving"
    fi
    fail "deploy failed AND rollback did not come up; service is DOWN"
}

# --- preflight ---------------------------------------------------------------

[ -d "$SRC" ] || fail "missing source checkout at $SRC"
[ -f "$ROOT/.env" ] || fail "missing $ROOT/.env"
[ -x "$CARGO" ] || fail "cargo not found at $CARGO"
command -v curl >/dev/null || fail "curl is required for the health check"

log "deploying to $ROOT"

# --- database backup ---------------------------------------------------------

# sqlite3 is not installed on the VPS, so ".backup" is unavailable. Copying
# the database together with its -wal and -shm siblings is the correct
# alternative: SQLite can recover the full state from that set. Copy the WAL
# *after* the database so it can only ever be newer, never staler, than the
# main file.
stamp="$(date -u +%Y%m%d-%H%M%S)"
backup="$DATA/web-api-pre-deploy-$stamp.db"
if [ -f "$DATA/web-api.db" ]; then
    log "backing up the database to $(basename "$backup")"
    cp "$DATA/web-api.db" "$backup"
    [ -f "$DATA/web-api.db-wal" ] && cp "$DATA/web-api.db-wal" "$backup-wal"
    [ -f "$DATA/web-api.db-shm" ] && cp "$DATA/web-api.db-shm" "$backup-shm"
else
    log "no database yet; skipping backup"
fi

# --- build -------------------------------------------------------------------

cd "$SRC"
previous_commit="$(git rev-parse --short HEAD)"
log "current commit $previous_commit"

log "fetching tags"
git fetch origin --tags --force

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
    || fail "tag '$TAG' does not exist (run 'git tag -l' to see what's available)"

# Every deployable tag must be reachable from origin/main -- this is the
# actual enforcement of "only what's on main gets deployed", not just a
# naming convention. A tag cut from a feature branch (or anywhere else)
# fails here instead of silently shipping.
git merge-base --is-ancestor "refs/tags/$TAG" origin/main \
    || fail "tag '$TAG' is not on main -- refusing to deploy a non-main tag"

log "checking out $TAG (detached)"
git checkout --detach "refs/tags/$TAG"

new_commit="$(git rev-parse --short HEAD)"
if [ "$previous_commit" = "$new_commit" ]; then
    log "already at $new_commit ($TAG); rebuilding anyway"
else
    log "updated $previous_commit -> $new_commit ($TAG)"
fi

log "building release binary (this takes a few minutes on 2 vCPU)"
"$CARGO" +stable build --release --locked --bin xindeler-web-api-server

built="$SRC/target/release/xindeler-web-api-server"
[ -x "$built" ] || fail "build did not produce $built"

# --- install -----------------------------------------------------------------

if [ -f "$BIN" ]; then
    log "keeping the current binary as $(basename "$PREVIOUS") for rollback"
    cp "$BIN" "$PREVIOUS"
fi

log "installing the new binary"
cp "$built" "$BIN.new"
mv "$BIN.new" "$BIN"

restart_service

# --- verify ------------------------------------------------------------------

log "waiting for the service to answer on $HEALTH_URL"
if ! wait_until_healthy; then
    log "service did not become healthy within ${HEALTH_TIMEOUT}s"
    rollback
fi
log "service is up"

public_check="$(curl -fsS --max-time 10 "$PUBLIC_URL" 2>/dev/null || true)"
if [ -n "$public_check" ]; then
    log "public health check: $public_check"
else
    log "public endpoint unreachable from the VPS; check it externally"
fi

# Informational only. The health check above already proved the service is
# answering; process introspection is a nicety and must not fail the deploy.
pid="$(pgrep -f 'xindeler-web-api-server$' | head -1 || true)"
if [ -n "$pid" ]; then
    threads="$(ps -o nlwp= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
    log "running as pid $pid${threads:+ with $threads threads}"
fi

log "deployed $TAG ($new_commit) successfully"
log "fast rollback (this build only): cp $PREVIOUS $BIN && pkill -9 -f 'xindeler-web-api-server\$'"
log "full rollback (any older tag): bash $0 <older-tag>"
