#!/usr/bin/env bash
# Wraps a built `mwgui` binary in "Music Warehouse.app", signs it ad hoc and
# zips it for a GitHub release. Used by .github/workflows/release.yml and
# runnable locally:
#
#   scripts/bundle-macos.sh target/release/mwgui 0.1.0 dist
#
# Ad-hoc signing (no Apple Developer ID) is enough for Apple Silicon to run
# the app, but it is not notarized, so Gatekeeper asks before the first
# launch of a downloaded copy. See README "Installing a release".
set -euo pipefail

binary=${1:?usage: bundle-macos.sh <binary> <version> <out-dir>}
version=${2:?usage: bundle-macos.sh <binary> <version> <out-dir>}
out=${3:?usage: bundle-macos.sh <binary> <version> <out-dir>}

name="Music Warehouse"
app="$out/$name.app"
zip="$out/music-warehouse-gui-$version-macos.zip"

rm -rf "$app" "$zip" "$zip.sha256"
mkdir -p "$app/Contents/MacOS"
cp "$binary" "$app/Contents/MacOS/mwgui"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleIdentifier</key><string>io.github.rcnsh.music-warehouse-gui</string>
  <key>CFBundleExecutable</key><string>mwgui</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>LSMinimumSystemVersion</key><string>${MACOSX_DEPLOYMENT_TARGET:-12.0}</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# The Keychain ties "Always Allow" to this signature, so each release asks
# once more for the READ_TOKEN; a Developer ID certificate would avoid that.
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict "$app"

# ditto keeps the bundle's extended attributes and signature intact; plain
# zip can break the seal.
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
(cd "$out" && shasum -a 256 "$(basename "$zip")" > "$(basename "$zip").sha256")

echo "$zip"
