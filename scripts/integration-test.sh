#!/bin/bash
# Integration test (design section 13): run the app against a throwaway data
# directory, write to the pasteboard from a helper process, assert rows.
#
# WARNING: this overwrites whatever is on your clipboard.
# Usage: scripts/integration-test.sh   (after `cargo tauri build --debug --config tauri.dev.conf.json --bundles app` in app/)
set -euo pipefail
cd "$(dirname "$0")/.."

APP="target/debug/bundle/macos/clipstash-dev.app/Contents/MacOS/clipstash-app"
CLI="target/debug/clipstash"
cargo build -q -p clipstash-cli
[ -x "$APP" ] || { echo "build the dev app first (see README)"; exit 2; }

export CLIPSTASH_DIR="$(mktemp -d)"
trap 'kill "$APP_PID" 2>/dev/null || true; rm -rf "$CLIPSTASH_DIR"' EXIT

"$CLI" list >/dev/null   # creates the databases
sqlite3 "$CLIPSTASH_DIR/snippets.db" \
  "INSERT INTO meta(key,value) VALUES('settings','{\"captureEnabled\":true,\"onboardingDone\":true,\"maxTextBytes\":64}')"
"$APP" >/dev/null 2>&1 &
APP_PID=$!
sleep 3

fail=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; fail=1; fi; }
rows() { "$CLI" list --json | grep -c '"id"' || true; }

"$CLI" pbwrite "integration alpha"; sleep 1
check "text copy is captured" '[ "$(rows)" = 1 ]'
check "it is searchable by substring" '"$CLI" search "ration al" | grep -q "integration alpha"'

"$CLI" pbwrite --concealed "hunter2"; sleep 1
check "concealed copy produces no row" '[ "$(rows)" = 1 ] && ! "$CLI" search hunter2 | grep -q hunter2'

"$CLI" pbwrite "$(printf 'x%.0s' $(seq 1 100))"; sleep 1
check "text over the size cap is dropped" '[ "$(rows)" = 1 ]'

"$CLI" pbwrite "https://example.com/a"; sleep 1
check "second copy is captured and flagged as a URL" '"$CLI" list --json | grep -q "\"isUrl\": true"'

"$CLI" pbwrite "integration alpha"; sleep 1
check "duplicate refreshes the existing row" '[ "$(rows)" = 2 ] && "$CLI" list --limit 1 | grep -q "integration alpha"'

"$CLI" copy 1; sleep 1
check "copy-back of an existing clip does not add a row" '[ "$(rows)" = 2 ]'

check "database files are user-only" '[ "$(stat -f %Lp "$CLIPSTASH_DIR/history.db")" = 600 ]'
exit $fail
