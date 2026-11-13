#!/usr/bin/env bash
# Builds every model in examples/, exports STL, 3MF and STEP, and slices it.
# STL and 3MF get the print plate, STEP gets the model as built.
# Models with "// expect: error" must fail with a clear message.
# Then two print preparation runs: all fixes on a model with holes, and a
# small print area: all parts stay on the one plate, with a note.
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

# Export and slice start from the Optimize values that the app stored for a
# model. The tests use a new config folder without them, with a copy of the
# settings (the OpenSCAD binary and its arguments).
cfg="${XDG_CONFIG_HOME:-$HOME/.config}/cocad/config"
mkdir -p "$out/config/cocad"
[ -f "$cfg" ] && cp "$cfg" "$out/config/cocad/config"
export XDG_CONFIG_HOME="$out/config"

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

# One line of the result table for a print preparation run.
row() { [ "$2" = PASS ] || fail=1; printf '%-22s %-8s %s\n' "$1" "$2" "$3"; }

# The number of build items in a 3MF file, or "?" without python3.
items() {
    if command -v python3 > /dev/null; then
        python3 -I -c 'import sys, zipfile
print(zipfile.ZipFile(sys.argv[1]).read("3D/3dmodel.model").decode().count("<item "))' "$1"
    else
        echo "?"
    fi
}

# These two runs need the model in the list of models to test.
# All fixes on a model with horizontal holes: the plate gets the bed
# chamfer and the teardrops, and the pipeline has no errors.
m=examples/bracket.scad
if [ -f "$m" ] && [[ " ${models[*]} " == *" $m "* ]]; then
    if log=$("$bin" export "$m" -o "$out/fixes.stl" --chamfer --teardrops --bridges --brim 2>&1); then
        tears=$(echo "$log" | sed -n 's/^fixes .* \([0-9]*\) teardrop hole.*/\1/p')
        if [ ! -s "$out/fixes.stl" ]; then row fixes FAIL "empty stl"
        elif ! echo "$log" | grep -q '^fixes .*bed chamfer 0.4 mm on 1 of 1 parts'; then row fixes FAIL "no bed chamfer"
        elif [ "${tears:-0}" -lt 1 ]; then row fixes FAIL "no teardrop holes"
        elif echo "$log" | grep -q '^error'; then row fixes FAIL "$(echo "$log" | grep -m1 '^error' | cut -c13-80)"
        else row fixes PASS "bracket: chamfer, $tears teardrops, $(echo "$log" | grep '^check' | cut -c13-)"
        fi
    else
        row fixes FAIL "$(echo "$log" | grep -m1 -i 'error' | cut -c1-70)"
    fi
fi

# A print area for the box or the lid, not both: all copies stay on the one
# plate, in one file, and the report says that they need more than one bed.
m=examples/lid-box.scad
if [ -f "$m" ] && [[ " ${models[*]} " == *" $m "* ]]; then
    if log=$("$bin" export "$m" -o "$out/plate.3mf" --bed 100x60x60 2>&1); then
        a=$out/plate.3mf
        n=$(echo "$log" | sed -n 's/^plate  *\([0-9]*\) cop.*/\1/p')
        if [ -z "$n" ] || [ "$n" -lt 2 ]; then row plate FAIL "not 2 or more copies on the plate"
        elif ! echo "$log" | grep -q '^note .*do not fit on one bed'; then row plate FAIL "no note about the bed"
        elif [ ! -s "$a" ]; then row plate FAIL "empty 3mf"
        elif [ "$(items "$a")" != "$n" ] && [ "$(items "$a")" != "?" ]; then row plate FAIL "the 3mf has $(items "$a") items, not $n"
        elif ! s=$("$bin" slice "$m" --bed 100x60x60 2>/dev/null) || ! echo "$s" | grep -q '^layers '; then
            row plate FAIL "slice has no estimate"
        else row plate PASS "lid-box: $n copies on one plate, $(echo "$s" | awk '/^filament/{print $4}') g"
        fi
    else
        row plate FAIL "$(echo "$log" | grep -m1 -i 'error' | cut -c1-70)"
    fi
fi
exit $fail
