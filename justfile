# Zenko Low-Disk Build & Maintenance Recipes
# Wraps `./scripts/build.sh` to enforce bounded Cargo `target/` size (`cargo-sweep` +
# zero incremental bloat + post-test binary pruning + `/tmp/zenko_*` hygiene).

set shell := ["bash", "-euo", "pipefail", "-c"]

# List available recipes
default:
    @just --list

# Fast workspace typecheck & test compilation + stale artifact sweep
check *args:
    ./scripts/build.sh check {{args}}

# Run full workspace test suite, prune compiled test executables, and sweep stale artifacts
test *args:
    ./scripts/build.sh test {{args}}

# Build embedded React/Tailwind WebUI into webui/dist
webui:
    ./scripts/build.sh webui

# Build WebUI + lean dev binary + stale artifact sweep
build *args:
    ./scripts/build.sh build {{args}}

# Build WebUI + stripped low-RAM release binary and reclaim target/debug
release *args:
    ./scripts/build.sh release {{args}}

# Launch Zenko web daemon/UI
dev *args:
    ./scripts/build.sh dev {{args}}

# Launch interactive Zenko TUI setup wizard
setup *args:
    ./scripts/build.sh setup {{args}}

# Full pre-ship verification (WebUI + workspace check + tests + sweep + disk report)
verify:
    ./scripts/build.sh verify

# Sweep stale old-version Cargo artifacts, incremental dirs, and /tmp/zenko_*
sweep:
    ./scripts/build.sh sweep

# Full clean of target/ and temporary test directories
clean:
    ./scripts/build.sh clean

# Show disk usage breakdown for target/, webui/, ~/.cargo, and free space
disk:
    ./scripts/build.sh disk
