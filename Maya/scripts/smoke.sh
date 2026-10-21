#!/usr/bin/env bash
set -euo pipefail

binary=${1:-target/release/maya-mini}
if [[ ! -x "$binary" ]]; then
  echo "smoke: executable not found: $binary" >&2
  exit 2
fi

scratch=$(mktemp -d "${TMPDIR:-/tmp}/maya-smoke.XXXXXX")
chmod 700 "$scratch"
pid=''
cleanup() {
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid" 2>/dev/null || true
    for _ in {1..50}; do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf -- "$scratch"
}
trap cleanup EXIT INT TERM

mkdir -m 700 "$scratch/data" "$scratch/cache" "$scratch/runtime"
stdout="$scratch/stdout"
stderr="$scratch/stderr"
"$binary" --headless --print-launch-link --port 0 \
  --data-dir "$scratch/data" --cache-dir "$scratch/cache" --runtime-dir "$scratch/runtime" \
  >"$stdout" 2>"$stderr" &
pid=$!

launch=''
for _ in {1..100}; do
  if ! kill -0 "$pid" 2>/dev/null; then
    cat "$stderr" >&2
    exit 1
  fi
  launch=$(grep -m1 -E '^http://127\.0\.0\.1:[0-9]+/#bootstrap=[A-Za-z0-9_-]+$' "$stdout" || true)
  [[ -n "$launch" ]] && break
  sleep 0.1
done
if [[ -z "$launch" ]]; then
  echo 'smoke: launch link was not emitted' >&2
  cat "$stderr" >&2
  exit 1
fi

base=${launch%%/#bootstrap=*}
bootstrap=${launch##*#bootstrap=}
health=$(curl --silent --show-error --fail --max-time 5 "$base/api/v1/health")
grep -q '"ready":true' <<<"$health"

session_body="$scratch/session-request.json"
printf '{"bootstrap_token":"%s"}' "$bootstrap" >"$session_body"
chmod 600 "$session_body"
session=$(curl --silent --show-error --fail --max-time 5 \
  --header "Origin: $base" --header 'Content-Type: application/json' \
  --data-binary "@$session_body" "$base/api/v1/session")
session_token=$(sed -n 's/.*"session_token":"\([^"]*\)".*/\1/p' <<<"$session")
csrf=$(sed -n 's/.*"csrf_token":"\([^"]*\)".*/\1/p' <<<"$session")
if [[ -z "$session_token" || -z "$csrf" ]]; then
  echo 'smoke: session exchange did not return credentials' >&2
  exit 1
fi

curl_config="$scratch/curl.conf"
{
  printf 'silent\nshow-error\nfail\nmax-time = 5\n'
  printf 'header = "Authorization: Bearer %s"\n' "$session_token"
} >"$curl_config"
chmod 600 "$curl_config"
settings=$(curl --config "$curl_config" "$base/api/v1/settings")
grep -q '"identity"' <<<"$settings"

kill -TERM "$pid"
for _ in {1..100}; do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.1
done
if kill -0 "$pid" 2>/dev/null; then
  echo 'smoke: application did not stop within 10 seconds' >&2
  exit 1
fi
wait "$pid"
pid=''
test -s "$scratch/data/maya.sqlite3"
echo 'smoke: PASS (startup, health, bootstrap, authenticated settings, graceful shutdown)'
