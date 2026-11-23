#!/usr/bin/env bash
set -euo pipefail

# Build Zenko's pure-Rust WebUI stylesheet with Tailwind CSS v4 Standalone CLI (zero Node.js)
# Usage: ./scripts/build-css.sh [--watch]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
INPUT_CSS="${ROOT_DIR}/crates/zenko-webui/assets/input.css"
OUTPUT_CSS="${ROOT_DIR}/crates/zenko-webui/assets/app.css"

if ! command -v tailwindcss >/dev/null 2>&1; then
  echo "Tailwind Standalone CLI not found in PATH."
  echo "Install zero-Node binary from: https://github.com/tailwindlabs/tailwindcss/releases"
  echo "Using pre-built embedded stylesheet at: ${OUTPUT_CSS}"
  exit 0
fi

if [[ "${1:-}" == "--watch" ]]; then
  exec tailwindcss -i "${INPUT_CSS}" -o "${OUTPUT_CSS}" --watch
else
  tailwindcss -i "${INPUT_CSS}" -o "${OUTPUT_CSS}" --minify
  echo "Compiled ${OUTPUT_CSS} ($(wc -c < "${OUTPUT_CSS}") bytes)"
fi
