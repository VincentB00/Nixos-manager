#!/usr/bin/env bash
# Build (if needed) and run Nixos-manager inside a nix-shell that provides
# the Rust and Node toolchains. Works on any NixOS machine with nothing
# installed beyond Nix itself.
#
#   ./run.sh                 build UI + daemon if stale, start on 127.0.0.1:7788
#   ./run.sh --dev           daemon + Angular dev server (live reload on :4200)
#   ./run.sh --watch         rebuild + restart automatically on any code change
#   ./run.sh --listen ADDR   listen address for the daemon (default 127.0.0.1:7788)
#   ./run.sh --rebuild       force rebuilding the UI and the daemon
#   ./run.sh --no-open       do not open the browser
#   ./run.sh --js-sass       use the pure-JS Sass compiler (if the native one crashes with EPIPE)
#   ./run.sh --help

set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

# ── Re-exec inside nix-shell ────────────────────────────────────────────────
if [[ -z "${IN_NIX_SHELL:-}" ]]; then
  if ! command -v nix-shell >/dev/null; then
    echo "nix-shell not found. Install Nix first: https://nixos.org/download" >&2
    exit 1
  fi
  extra=()
  if ! nix-instantiate --find-file nixpkgs >/dev/null 2>&1; then
    # No <nixpkgs> on NIX_PATH (fresh install without channels): pin one.
    extra=(-I nixpkgs=channel:nixos-unstable)
  fi
  args=""
  for a in "$@"; do args+=" $(printf '%q' "$a")"; done
  exec nix-shell "${extra[@]}" shell.nix --run "./run.sh$args"
fi

# ── Options ─────────────────────────────────────────────────────────────────
DEV=0; WATCH=0; LISTEN="127.0.0.1:7788"; REBUILD=0; OPEN=1; JS_SASS="${NIXOS_MANAGER_JS_SASS:-0}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dev) DEV=1 ;;
    --watch) WATCH=1 ;;
    --listen) LISTEN="$2"; shift ;;
    --listen=*) LISTEN="${1#*=}" ;;
    --rebuild) REBUILD=1 ;;
    --no-open) OPEN=0 ;;
    --js-sass) JS_SASS=1 ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $1 (try --help)" >&2; exit 1 ;;
  esac
  shift
done

log() { printf '\033[1;36m[run.sh]\033[0m %s\n' "$*"; }

# Angular compiles SCSS with sass-embedded, a native Dart helper process. If it
# dies (symptom: "Error: write EPIPE" from sass-embedded), fall back to the
# pure-JavaScript compiler by setting NG_BUILD_SASS_EMBEDDED=0.
if [[ $JS_SASS -eq 1 ]]; then export NG_BUILD_SASS_EMBEDDED=0; fi
build_ui() {
  if (cd ui && npx ng build --configuration production); then return 0; fi
  if [[ "${NG_BUILD_SASS_EMBEDDED:-}" == "0" ]]; then return 1; fi
  log "UI build failed; retrying with the pure-JS Sass compiler (NG_BUILD_SASS_EMBEDDED=0)..."
  (cd ui && NG_BUILD_SASS_EMBEDDED=0 npx ng build --configuration production)
}

# ── UI dependencies ─────────────────────────────────────────────────────────
if [[ ! -d ui/node_modules || ui/package-lock.json -nt ui/node_modules/.package-lock.json ]]; then
  log "Installing UI dependencies (npm ci)..."
  (cd ui && npm ci --no-audit --no-fund)
fi

# ── Watch mode: rerun this script (build what is stale, restart) on changes ──
if [[ $WATCH -eq 1 ]]; then
  log "Watch mode: rebuilding and restarting on changes under crates/, ui/src, Cargo.toml"
  if [[ $OPEN -eq 1 ]]; then (sleep 8; xdg-open "http://$LISTEN" >/dev/null 2>&1 || true) & fi
  exec watchexec --restart --debounce 700ms --stop-signal SIGINT \
    --watch crates --watch ui/src --watch ui/angular.json --watch Cargo.toml --watch Cargo.lock \
    --exts rs,toml,ts,html,scss,css,json \
    -- ./run.sh --no-open --listen "$LISTEN"
fi

# ── Dev mode: daemon (cargo run) + ng serve with the /api proxy ─────────────
if [[ $DEV -eq 1 ]]; then
  log "Dev mode: daemon on $LISTEN, UI with live reload on http://localhost:4200"
  trap 'kill 0' EXIT INT TERM
  cargo run -p nixos-manager-daemon -- --listen "$LISTEN" &
  (cd ui && npx ng serve --open=false) &
  if [[ $OPEN -eq 1 ]]; then (sleep 6; xdg-open http://localhost:4200 >/dev/null 2>&1 || true) & fi
  wait
  exit 0
fi

# ── Build the UI when its sources are newer than the bundle ────────────────
UI_OUT=ui/dist/nixos-manager-ui/browser/index.html
if [[ $REBUILD -eq 1 || ! -f $UI_OUT || -n "$(find ui/src ui/angular.json ui/package.json -newer "$UI_OUT" -print -quit 2>/dev/null)" ]]; then
  log "Building the UI..."
  build_ui
fi

# ── Build the daemon (release) when sources are newer than the binary ───────
BIN=target/release/nixos-manager-daemon
if [[ $REBUILD -eq 1 || ! -x $BIN || -n "$(find crates Cargo.toml Cargo.lock "$UI_OUT" -newer "$BIN" -print -quit 2>/dev/null)" ]]; then
  log "Building the daemon (release)..."
  cargo build --release -p nixos-manager-daemon
fi

# ── Run ─────────────────────────────────────────────────────────────────────
URL="http://$LISTEN"
if (exec 3<>"/dev/tcp/${LISTEN%:*}/${LISTEN##*:}") 2>/dev/null; then
  exec 3>&- 3<&-
  echo "[run.sh] Something is already listening on $LISTEN (another Nixos-manager?). Stop it or use --listen." >&2
  exit 1
fi
log "Starting Nixos-manager at $URL  (Ctrl-C to stop)"
if [[ $OPEN -eq 1 ]]; then (sleep 1.5; xdg-open "$URL" >/dev/null 2>&1 || true) & fi
exec "$BIN" --listen "$LISTEN"
