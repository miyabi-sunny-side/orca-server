#!/usr/bin/env bash
# Local validation in two sizes.
#   quick  the pre-commit set: format, lint, unit, integration and mocked-API browser tests.
#   full   everything (the default): quick plus Chromium against the real API, official
#          OrcaSlicer, the container image and its tests. Run it on the commit to release.
# Independent stages run side by side; each writes its own log and the summary shows its time.
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${1:-full}
case "$mode" in
  quick | full) ;;
  *)
    echo "usage: bash tests/verify-local.sh [quick|full]" >&2
    exit 2
    ;;
esac

tools=(cargo npm npx curl)
if [[ $mode == full ]]; then
  : "${ORCA_APPDIR:?Set ORCA_APPDIR to the extracted official OrcaSlicer 2.4.2 AppDir}"
  test -x "$ORCA_APPDIR/AppRun"
  ORCA_APPDIR=$(cd "$ORCA_APPDIR" && pwd)
  export ORCA_APPDIR
  tools+=(docker openssl python3 sha256sum)
fi
for tool in "${tools[@]}"; do command -v "$tool" >/dev/null; done
if [[ $mode == full ]]; then docker info >/dev/null; fi
for name in ${!E2E_@}; do unset "$name"; done
export ORCA_TEST_OUTPUT=${ORCA_TEST_OUTPUT:-$(mktemp -d /tmp/orca-local-check.XXXXXX)}
mkdir -p "$ORCA_TEST_OUTPUT"
ORCA_TEST_OUTPUT=$(cd "$ORCA_TEST_OUTPUT" && pwd)
export ORCA_TEST_OUTPUT
logs=$ORCA_TEST_OUTPUT/stages
mkdir -p "$logs"
target=${CARGO_TARGET_DIR:-target}
image="orca-local-check:$$"
started=$SECONDS

names=()
pids=()
# stage NAME FUNCTION: run the function in the background with its own log and timing.
stage() {
  local name=$1
  shift
  (
    local begin=$SECONDS status=ok
    "$@" >"$logs/$name.log" 2>&1 || status=failed
    echo "$status $((SECONDS - begin))" >"$logs/$name.result"
    [[ $status == ok ]]
  ) &
  names+=("$name")
  pids+=($!)
}
# report NAME: show the stage's result and time, and the end of its log when it failed.
report() {
  local name=$1 status seconds
  read -r status seconds <"$logs/$name.result"
  printf '%-12s %-6s %4ss\n' "$name" "$status" "$seconds"
  if [[ $status != ok ]]; then
    echo "---- $logs/$name.log (last 60 lines)"
    tail -n 60 "$logs/$name.log"
    echo "----"
    return 1
  fi
}
# join: wait for every started stage and fail if any of them failed.
join() {
  local failed=0 index
  for index in "${!names[@]}"; do
    wait "${pids[$index]}" || true
    report "${names[$index]}" || failed=1
  done
  names=()
  pids=()
  return "$failed"
}
# step NAME FUNCTION: run one stage now, while the stages already started keep running.
step() {
  local name=$1 begin=$SECONDS status=ok
  shift
  "$@" >"$logs/$name.log" 2>&1 || status=failed
  echo "$status $((SECONDS - begin))" >"$logs/$name.result"
  report "$name"
}
cleanup() {
  # Stages still running after a failure must not outlive the script.
  local pid
  for pid in $(jobs -p); do kill "$pid" 2>/dev/null || true; done
  if [[ $mode == full ]]; then docker image rm "$image" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT

# A stage runs where `set -e` is ignored, so its commands are chained with `&&`.
# The Rust build embeds client/dist, so the page is built before anything compiles.
page() {
  npm --prefix client ci &&
    npm --prefix client run build
}
client_checks() {
  npm --prefix client run format:check &&
    npm --prefix client run check &&
    npm --prefix client run lint:design &&
    npm --prefix client test &&
    (cd client && npx playwright install chromium) &&
    npm --prefix client run test:e2e -- --workers=4
}
rust_checks() {
  cargo fmt --all -- --check &&
    cargo clippy --locked --all-targets --all-features -- -D warnings &&
    cargo test --locked --no-run
}
unit() { cargo test --locked --lib; }
# These tests mostly wait for processes and sockets, so more of them run than there are cores.
integration() { cargo test --locked --test it -- --test-threads=16; }
# The notification scenario starts the library's own test binary as its service.
notifications() {
  cargo test --locked --lib -- --ignored --exact \
    notifications::delivery_tests::isolated_notification_delivery
}
embedded_page() { bash tests/embedded_ui.sh "$target/debug/orca-server"; }
official() {
  ORCA_ROLE_UI=1 cargo test --locked --test it -- --ignored --nocapture \
    official_cli:: material_roles:: registry_flows::independent_printers
}
browsers() {
  cargo test --locked --test it browser:: -- --ignored --test-threads=4 --nocapture
}
container_image() { docker build -t "$image" .; }
container() {
  ORCA_TEST_IMAGE="$image" cargo test --locked --test it container:: -- --ignored --test-threads=1
}

step page page
stage client client_checks
if [[ $mode == full ]]; then stage image container_image; fi
# Compiling finishes before the test stages share the build directory.
step rust rust_checks
# The image build takes every core; the timing-sensitive tests wait until it is done.
if [[ $mode == full ]]; then join; fi
stage unit unit
stage integration integration
stage notify notifications
stage embedded embedded_page
join
if [[ $mode == full ]]; then
  stage official official
  stage container container
  join
  # Chromium fails to capture screenshots when the slicer runs take the machine as well.
  step browsers browsers
fi
printf 'Local verification passed (%s, %ss). Evidence: %s\n' \
  "$mode" "$((SECONDS - started))" "$ORCA_TEST_OUTPUT"
