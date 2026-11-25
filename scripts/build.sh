#!/usr/bin/env bash
set -euo pipefail

# Zenko low-disk build, test, and sweep driver.
# Keeps `target/` compact by disabling incremental bloat, enforcing age + maxsize bounds
# via `cargo-sweep` (relatime-safe), pruning large test binaries after runs, and cleaning
# leftover `/tmp/zenko_*` test directories.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/target}"

# Tunable storage guardrails (override via env if desired)
ZENKO_TARGET_MAX_MB="${ZENKO_TARGET_MAX_MB:-1500}"
ZENKO_MIN_FREE_MB="${ZENKO_MIN_FREE_MB:-4096}"
ZENKO_SWEEP_DAYS="${ZENKO_SWEEP_DAYS:-2}"
ZENKO_KEEP_TEST_BINS="${ZENKO_KEEP_TEST_BINS:-0}"

export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_TARGET_DIR="${TARGET_DIR}"

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

cargo_run() {
  local subcmd="${1:-}"
  local interactive_tty=0
  if [[ -t 1 && "${TERM:-dumb}" != "dumb" && -z "${CI:-}" && -z "${ANTIGRAVITY_SOURCE_METADATA:-}" ]]; then
    interactive_tty=1
  fi
  local pretty_mode="${ZENKO_PRETTY:-auto}"
  if [[ "${pretty_mode}" != "0" && "${interactive_tty}" == "1" ]]; then
    case "${subcmd}" in
      build|b|check|c|test|t)
        if [[ "${pretty_mode}" == "external" ]] && has_cmd cargo-pretty && [[ "${subcmd}" == "build" || "${subcmd}" == "test" ]]; then
          cargo pretty "$@"
          return
        fi
        if has_cmd python3 && [[ -f "${SCRIPT_DIR}/cargo_pretty.py" ]]; then
          python3 -B "${SCRIPT_DIR}/cargo_pretty.py" "$@"
          return
        fi
        ;;
    esac
  fi
  cargo "$@"
}

dir_size_mb() {
  local p="$1"
  if [[ -d "$p" ]]; then
    du -sm "$p" 2>/dev/null | awk '{print $1}'
  else
    echo 0
  fi
}

free_space_mb() {
  df -Pm "${ROOT_DIR}" 2>/dev/null | awk 'NR==2 {print $4}'
}

clean_tmp_tests() {
  rm -rf /tmp/zenko_* 2>/dev/null || true
}

prune_incremental_dirs() {
  rm -rf "${TARGET_DIR}/debug/incremental" "${TARGET_DIR}/release/incremental" 2>/dev/null || true
}

prune_test_binaries() {
  if [[ "${ZENKO_KEEP_TEST_BINS}" == "1" ]]; then
    return 0
  fi
  local deps_dir="${TARGET_DIR}/debug/deps"
  if [[ -d "${deps_dir}" ]]; then
    # Remove compiled Zenko test executables (keeps warm third-party .rlib/.rmeta caches intact).
    find "${deps_dir}" -maxdepth 1 -type f -executable \( -name 'zenko-*' -o -name 'zenko_*' \) -delete 2>/dev/null || true
  fi
}

sweep_stale_artifacts() {
  prune_incremental_dirs
  clean_tmp_tests
  rm -f "${ROOT_DIR}/sweep.timestamp" 2>/dev/null || true

  if [[ ! -d "${TARGET_DIR}" ]]; then
    return 0
  fi

  # Avoid `cargo sweep --stamp/--file` on Linux `relatime` mounts: warm `.rlib` reads within 24h
  # do not bump atime, causing stamp mode to evict active dependencies and trigger cold rebuilds.
  if has_cmd cargo-sweep; then
    cargo sweep --time "${ZENKO_SWEEP_DAYS}" "${ROOT_DIR}" >/dev/null 2>&1 || true
    cargo sweep --maxsize "${ZENKO_TARGET_MAX_MB}MB" "${ROOT_DIR}" >/dev/null 2>&1 || true
  else
    # Pure-POSIX fallback when cargo-sweep is unavailable: evict artifacts older than ZENKO_SWEEP_DAYS.
    find "${TARGET_DIR}" -type f -mtime "+${ZENKO_SWEEP_DAYS}" -delete 2>/dev/null || true
  fi
}

preflight_disk_guard() {
  local free_mb target_mb
  free_mb="$(free_space_mb)"
  target_mb="$(dir_size_mb "${TARGET_DIR}")"

  if [[ "${target_mb}" -gt "${ZENKO_TARGET_MAX_MB}" ]]; then
    echo "[zenko-build] target/ is ${target_mb} MB (> ${ZENKO_TARGET_MAX_MB} MB cap); sweeping stale artifacts first..."
    sweep_stale_artifacts
  fi

  free_mb="$(free_space_mb)"
  if [[ "${free_mb}" -lt "${ZENKO_MIN_FREE_MB}" ]]; then
    echo "[zenko-build] Low free disk space (${free_mb} MB < ${ZENKO_MIN_FREE_MB} MB); running aggressive target sweep..."
    prune_test_binaries
    sweep_stale_artifacts
    free_mb="$(free_space_mb)"
    if [[ "${free_mb}" -lt 1024 ]]; then
      echo "[zenko-build] ERROR: Critically low free disk space (${free_mb} MB). Run 'just clean' or free space before compiling." >&2
      exit 1
    fi
  fi
}

disk_report() {
  echo "=== Zenko Storage Report ==="
  df -h "${ROOT_DIR}" /tmp | awk 'NR==1 || NR==2 || NR==3 {print}'
  echo ""
  printf "  %-24s %s\n" "target/" "$(du -sh "${TARGET_DIR}" 2>/dev/null | awk '{print $1}' || echo '0B')"
  printf "  %-24s %s\n" "target/debug/" "$(du -sh "${TARGET_DIR}/debug" 2>/dev/null | awk '{print $1}' || echo '0B')"
  printf "  %-24s %s\n" "target/release/" "$(du -sh "${TARGET_DIR}/release" 2>/dev/null | awk '{print $1}' || echo '0B')"
  printf "  %-24s %s\n" "webui/dist/" "$(du -sh "${ROOT_DIR}/webui/dist" 2>/dev/null | awk '{print $1}' || echo '0B')"
  printf "  %-24s %s\n" "webui/node_modules/" "$(du -sh "${ROOT_DIR}/webui/node_modules" 2>/dev/null | awk '{print $1}' || echo '0B')"
  printf "  %-24s %s\n" "~/.cargo/" "$(du -sh "${HOME}/.cargo" 2>/dev/null | awk '{print $1}' || echo '0B')"
  local tmp_count
  tmp_count="$(find /tmp -maxdepth 1 -name 'zenko_*' 2>/dev/null | wc -l | tr -d ' ')"
  printf "  %-24s %s dir(s)\n" "/tmp/zenko_*" "${tmp_count}"
}

build_webui() {
  npm --prefix "${ROOT_DIR}/webui" run build
}

cmd="${1:-build}"
shift || true

case "${cmd}" in
  webui)
    build_webui
    ;;
  check)
    preflight_disk_guard
    cargo_run check --workspace --tests "$@"
    sweep_stale_artifacts
    ;;
  test)
    preflight_disk_guard
    cargo_run test --workspace "$@"
    prune_test_binaries
    sweep_stale_artifacts
    ;;
  build)
    preflight_disk_guard
    build_webui
    cargo_run build "$@"
    sweep_stale_artifacts
    ;;
  --release|release)
    preflight_disk_guard
    build_webui
    cargo_run build --release "$@"
    # Reclaim debug artifacts after producing a release build to keep storage minimal.
    rm -rf "${TARGET_DIR}/debug" 2>/dev/null || true
    sweep_stale_artifacts
    if [[ -f "${TARGET_DIR}/release/zenko" ]]; then
      echo "[zenko-build] Release binary: ${TARGET_DIR}/release/zenko ($(du -sh "${TARGET_DIR}/release/zenko" | awk '{print $1}'))"
    fi
    ;;
  dev)
    preflight_disk_guard
    if [[ ! -f "${ROOT_DIR}/webui/dist/index.html" ]]; then
      build_webui
    fi
    cargo_run build -p zenko-cli --bin zenko
    exec "${TARGET_DIR}/debug/zenko" web "$@"
    ;;
  setup)
    preflight_disk_guard
    cargo_run build -p zenko-cli --bin zenko
    exec "${TARGET_DIR}/debug/zenko" setup "$@"
    ;;
  verify)
    preflight_disk_guard
    build_webui
    cargo_run check --workspace --tests
    cargo_run test --workspace
    prune_test_binaries
    sweep_stale_artifacts
    disk_report
    ;;
  sweep)
    prune_test_binaries
    sweep_stale_artifacts
    disk_report
    ;;
  clean)
    cargo clean
    rm -f "${ROOT_DIR}/sweep.timestamp"
    clean_tmp_tests
    disk_report
    ;;
  disk)
    disk_report
    ;;
  help|-h|--help)
    cat <<'EOF'
Usage: ./scripts/build.sh <command> [extra-cargo-args...]

Commands:
  check      Run cargo check --workspace --tests + sweep stale artifacts
  test       Run cargo test --workspace + prune test binaries + sweep stale artifacts
  webui      Build embedded WebUI (webui/dist)
  build      Build WebUI + lean dev binary + sweep stale artifacts
  release    Build WebUI + stripped release binary + drop target/debug + sweep
  dev        Launch Zenko web server/UI (lean dev profile)
  setup      Launch Zenko interactive TUI setup wizard
  verify     Full pre-ship verification (webui + check + test + sweep + disk report)
  sweep      Enforce ZENKO_TARGET_MAX_MB cap and remove stale artifacts + /tmp/zenko_*
  clean      Full cargo clean + remove /tmp/zenko_*
  disk       Show storage breakdown for target/, webui/, ~/.cargo, and /tmp

Environment overrides:
  ZENKO_TARGET_MAX_MB=1500   Max target/ size before cargo-sweep prunes oldest files
  ZENKO_MIN_FREE_MB=4096     Minimum free disk space guard before building
  ZENKO_KEEP_TEST_BINS=0     Set to 1 to retain compiled test executables in target/debug/deps
  ZENKO_PRETTY=auto          Pretty Cargo renderer: auto (native), external (cargo-pretty), or 0 (raw)
EOF
    ;;
  *)
    echo "[zenko-build] Unknown command: ${cmd}. Run './scripts/build.sh help'." >&2
    exit 2
    ;;
esac
