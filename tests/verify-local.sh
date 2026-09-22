#!/usr/bin/env bash
# All local validation, including dependencies intentionally absent from ordinary cargo test.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${ORCA_APPDIR:?Set ORCA_APPDIR to the extracted official OrcaSlicer 2.4.2 AppDir}"
test -x "$ORCA_APPDIR/AppRun"
ORCA_APPDIR=$(cd "$ORCA_APPDIR" && pwd)
export ORCA_APPDIR
for tool in cargo npm npx docker openssl python3 sha256sum curl; do command -v "$tool" >/dev/null; done
docker info >/dev/null
for name in ${!E2E_@}; do unset "$name"; done
export ORCA_TEST_OUTPUT=${ORCA_TEST_OUTPUT:-$(mktemp -d /tmp/orca-local-check.XXXXXX)}
mkdir -p "$ORCA_TEST_OUTPUT"
ORCA_TEST_OUTPUT=$(cd "$ORCA_TEST_OUTPUT" && pwd)
export ORCA_TEST_OUTPUT
image="orca-local-check:$$"
cleanup() { docker image rm "$image" >/dev/null 2>&1 || true; }
trap cleanup EXIT
npm --prefix client ci
(cd client && npx playwright install chromium)
npm --prefix client run format:check
npm --prefix client run check
npm --prefix client run lint:design
npm --prefix client test
npm --prefix client run test:e2e -- --workers=1
npm --prefix client run build
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked --bins --example fixture-slicer
cargo test --locked
cargo build --locked --release
bash tests/embedded_ui.sh "${CARGO_TARGET_DIR:-target}/release/orca-server"
cargo test --locked --test official_cli -- --ignored
cargo test --locked --test registry_flows independent_printers -- --ignored
cargo test --locked --test browser -- --ignored --test-threads=1 --nocapture
docker build -t "$image" .
ORCA_TEST_IMAGE="$image" cargo test --locked --test container -- --ignored --test-threads=1
printf 'Local verification passed. Evidence: %s\n' "$ORCA_TEST_OUTPUT"
