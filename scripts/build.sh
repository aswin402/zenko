#!/usr/bin/env bash
set -euo pipefail

# Zenko unified build helper using romancitodev/cargo-pretty + embedded WebUI build
# Usage:
#   ./scripts/build.sh                # dev build (WebUI + cargo pretty build)
#   ./scripts/build.sh --release      # optimized low-RAM release binary
#   ./scripts/build.sh test           # run full workspace test suite via cargo pretty

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

CARGO_CMD="cargo"
if [[ -t 1 ]] && command -v cargo-pretty >/dev/null 2>&1; then
  CARGO_CMD="cargo pretty"
fi

case "${1:-build}" in
  --release|release)
    npm --prefix "${ROOT_DIR}/webui" run build
    ${CARGO_CMD} build --release
    ;;
  test)
    ${CARGO_CMD} test --workspace
    ;;
  check)
    ${CARGO_CMD} check --workspace
    ;;
  *)
    npm --prefix "${ROOT_DIR}/webui" run build
    ${CARGO_CMD} build
    ;;
esac
