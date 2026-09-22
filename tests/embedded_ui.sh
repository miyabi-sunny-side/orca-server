#!/usr/bin/env bash
set -euo pipefail
binary=${1:?Usage: bash tests/embedded_ui.sh PATH_TO_BINARY}
binary_dir=$(mktemp -d)
server_pid=
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill -TERM "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$binary_dir"
}
trap cleanup EXIT
cp "$binary" "$binary_dir/orca-server"
cd "$binary_dir"
env -i PATH="$PATH" PORT=3100 APP_BIND_ADDR=invalid-legacy-address ./orca-server &
server_pid=$!

for attempt in {1..20}; do
  if curl --fail --silent --show-error http://127.0.0.1:3100/healthz >/dev/null; then
    break
  fi
  if [[ "$attempt" == 20 ]]; then
    exit 1
  fi
  sleep 0.25
done

test "$(curl --fail --silent http://127.0.0.1:3100/healthz)" = "ok"
test "$(curl --fail --silent http://127.0.0.1:3100/api/health)" = '{"status":"ok"}'
curl --fail --silent http://127.0.0.1:3100/ > index.html
grep --ignore-case '<!doctype html' index.html
asset=$(sed -n 's/.*src="\([^" ]*\.js\)".*/\1/p' index.html)
test -n "$asset"
curl --fail --silent "http://127.0.0.1:3100$asset" > app.js
test -s app.js
test "$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:3100/api/missing)" = "404"
test "$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:3100/projects/example)" = "200"

kill -TERM "$server_pid"
wait "$server_pid"
server_pid=
