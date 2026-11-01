#!/usr/bin/env bash
# Builds a Debian package in dist/.
#
# Usage: scripts/package-deb.sh [--no-build]
#   --no-build  package the existing target/release binary
#
# Install with: sudo apt install ./dist/scad-viewer_<version>_<arch>.deb
set -euo pipefail
cd "$(dirname "$0")/.."

build=1
for a in "$@"; do
    case "$a" in
        --no-build) build=0 ;;
        -h|--help) sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $a" >&2; exit 2 ;;
    esac
done

[ "$build" = 1 ] && scripts/build.sh

name=scad-viewer
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(dpkg --print-architecture)
bin=target/release/$name
[ -x "$bin" ] || { echo "missing $bin, run scripts/build.sh" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
root=$stage/${name}_${version}_${arch}

install -Dm755 "$bin" "$root/usr/bin/$name"
strip --strip-unneeded "$root/usr/bin/$name"
install -Dm644 packaging/$name.desktop "$root/usr/share/applications/$name.desktop"
install -Dm644 packaging/$name.svg "$root/usr/share/icons/hicolor/scalable/apps/$name.svg"
install -Dm644 packaging/$name-mime.xml "$root/usr/share/mime/packages/$name.xml"
install -Dm644 README.md "$root/usr/share/doc/$name/README.md"
install -Dm644 examples/*.scad -t "$root/usr/share/doc/$name/examples/"

size=$(du -sk "$root/usr" | cut -f1)
mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CTRL
Package: $name
Version: $version
Architecture: $arch
Maintainer: Christos Georgiou <christos@cgeosoft.com>
Installed-Size: $size
Depends: libc6, libgcc-s1, libgl1, libx11-6, libxkbcommon0, libxcursor1, libxrandr2, libxi6, libgtk-3-0t64 | libgtk-3-0, openscad-nightly | openscad
Recommends: libwayland-client0, libxkbcommon-x11-0
Section: graphics
Priority: optional
Homepage: https://github.com/cgeosoft/scad-viewer
Description: OpenSCAD model viewer with Customizer parameters
 Opens .scad files, shows the Customizer parameters as sliders and
 lists, and renders the model with OpenSCAD. Exports STL, 3MF and STEP.
 Includes a simple slice preview with layer view and print estimates.
CTRL

cat > "$root/DEBIAN/postinst" <<'POST'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    command -v update-mime-database >/dev/null && update-mime-database /usr/share/mime || true
    command -v update-desktop-database >/dev/null && update-desktop-database -q /usr/share/applications || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q /usr/share/icons/hicolor || true
fi
POST
cp "$root/DEBIAN/postinst" "$root/DEBIAN/postrm"
sed -i 's/= configure/= remove/' "$root/DEBIAN/postrm"
chmod 755 "$root/DEBIAN/postinst" "$root/DEBIAN/postrm"

mkdir -p dist
fakeroot dpkg-deb --build -Zxz "$root" dist/ >/dev/null
deb="dist/${name}_${version}_${arch}.deb"
echo "Built $deb ($(du -h "$deb" | cut -f1))"
echo "Install: sudo apt install ./$deb"
