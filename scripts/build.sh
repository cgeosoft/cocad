#!/usr/bin/env bash
# Builds CoCAD.
#
# Usage: scripts/build.sh [--debug] [--no-test]
#   --debug    debug build (default: release)
#   --no-test  skip the unit tests
set -euo pipefail
cd "$(dirname "$0")/.."

profile=release
test=1
for a in "$@"; do
    case "$a" in
        --debug) profile=debug ;;
        --no-test) test=0 ;;
        -h|--help) sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $a" >&2; exit 2 ;;
    esac
done

if [ "$test" = 1 ]; then
    cargo test --quiet
fi
if [ "$profile" = release ]; then
    # Without a key, the app sends no data and does not ask the user about it.
    if [ -z "${POSTHOG_API_KEY:-}" ] && [ -f extras/website/.env ]; then
        POSTHOG_API_KEY=$(sed -n 's/^POSTHOG_API_KEY=//p' extras/website/.env | tr -d "\"'")
        export POSTHOG_API_KEY
    fi
    cargo build --release --locked
else
    cargo build --locked
fi

bin="target/$profile/cocad"
echo "Built $bin ($(du -h "$bin" | cut -f1))"
