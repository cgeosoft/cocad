#!/usr/bin/env bash
# Builds "CoCAD.app" (universal: Apple Silicon and Intel) and zips it to dist/.
# Needs release builds for aarch64-apple-darwin and x86_64-apple-darwin.
# The app is not signed: on first start, right-click it and choose Open.
#
# Usage: scripts/package-macos.sh
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app="dist/CoCAD.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/cocad" \
    target/aarch64-apple-darwin/release/cocad \
    target/x86_64-apple-darwin/release/cocad
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>CoCAD</string>
  <key>CFBundleDisplayName</key><string>CoCAD</string>
  <key>CFBundleIdentifier</key><string>com.cgeosoft.cocad</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleExecutable</key><string>cocad</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>OpenSCAD model</string>
      <key>CFBundleTypeExtensions</key><array><string>scad</string></array>
      <key>CFBundleTypeRole</key><string>Viewer</string>
    </dict>
  </array>
</dict>
</plist>
PLIST
cp README.md LICENSE "$app/Contents/Resources/"
(cd dist && ditto -c -k --keepParent "CoCAD.app" "cocad-$version-macos-universal.zip")
echo "Built dist/cocad-$version-macos-universal.zip"
