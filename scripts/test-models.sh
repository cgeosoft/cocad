#!/usr/bin/env bash
# Renders every model in examples/, exports STL, 3MF and STEP, and slices it.
# Models with "// expect: error" must fail with a clear message.
#
# Usage: scripts/test-models.sh [--release] [model.scad...]
set -uo pipefail
cd "$(dirname "$0")/.."

profile=debug
models=()
for a in "$@"; do
    case "$a" in
        --release) profile=release ;;
        -h|--help) sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) models+=("$a") ;;
    esac
done
[ ${#models[@]} -eq 0 ] && models=(examples/*.scad)

if [ "$profile" = release ]; then cargo build --release --quiet; else cargo build --quiet; fi || exit 1
bin=target/$profile/cocad
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

fail=0
printf '%-22s %-8s %s\n' MODEL RESULT DETAIL
for m in "${models[@]}"; do
    name=$(basename "$m" .scad)
    expect_error=0
    grep -q '^// expect: error' "$m" && expect_error=1
    detail=""
    ok=1
    for ext in stl 3mf step; do
        if ! log=$("$bin" export "$m" -o "$out/$name.$ext" 2>&1); then
            ok=0; detail=$(echo "$log" | grep -m1 -iE 'error|geometry|empty' | cut -c1-70); break
        fi
        [ -s "$out/$name.$ext" ] || { ok=0; detail="empty $ext"; break; }
    done
    if [ $ok = 1 ]; then
        if s=$("$bin" slice "$m" 2>/dev/null); then
            layers=$(echo "$s" | awk '/^layers/{print $2}')
            grams=$(echo "$s" | awk '/^filament/{print $4}')
            tris=$("$bin" export "$m" -o "$out/x.stl" 2>&1 | sed -n 's/built \([0-9]*\) triangles.*/\1/p')
            detail="$tris tris, $layers layers, $grams g"
        else
            ok=0; detail="slice failed"
        fi
    fi
    if [ $expect_error = 1 ]; then
        if [ $ok = 0 ]; then result=PASS; detail="error as expected: $detail"; else result=FAIL; detail="expected an error"; fi
    else
        if [ $ok = 1 ]; then result=PASS; else result=FAIL; fi
    fi
    [ $result = FAIL ] && fail=1
    printf '%-22s %-8s %s\n' "$name" "$result" "$detail"
done
exit $fail
