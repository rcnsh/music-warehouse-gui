#!/usr/bin/env bash
# Wraps a built `mwgui` binary in "Music Warehouse.app", signs it and zips it
# for a GitHub release. Used by .github/workflows/release.yml and runnable
# locally:
#
#   scripts/bundle-macos.sh target/release/mwgui 0.1.0 dist
#
# With MACOS_SIGN_IDENTITY set to a "Developer ID Application: …" identity in
# the keychain, the app is signed with it under the hardened runtime. Adding
# Apple notary credentials then notarizes the app and staples the ticket, so
# Gatekeeper opens a downloaded copy without asking:
#
#   NOTARY_PROFILE      a profile saved with `xcrun notarytool store-credentials`
#   or APPLE_ID, APPLE_APP_PASSWORD (app-specific) and APPLE_TEAM_ID
#
# Without an identity it signs ad hoc: enough to run on Apple Silicon, but
# Gatekeeper asks before the first launch.
set -euo pipefail

binary=${1:?usage: bundle-macos.sh <binary> <version> <out-dir>}
version=${2:?usage: bundle-macos.sh <binary> <version> <out-dir>}
out=${3:?usage: bundle-macos.sh <binary> <version> <out-dir>}

name="Music Warehouse"
app="$out/$name.app"
zip="$out/music-warehouse-gui-$version-macos.zip"

rm -rf "$app" "$zip" "$zip.sha256"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/mwgui"
# Built from assets/icon-macos.svg; see assets/README.md to regenerate.
cp "$(dirname "$0")/../assets/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleIdentifier</key><string>io.github.rcnsh.music-warehouse-gui</string>
  <key>CFBundleExecutable</key><string>mwgui</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
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

# The Keychain ties "Always Allow" to the signature's requirement. A
# Developer ID one names the team, so it holds across releases; an ad-hoc one
# names this exact binary, so each release asks once more for the READ_TOKEN.
if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$MACOS_SIGN_IDENTITY" "$app"
else
  codesign --force --sign - --timestamp=none "$app"
fi
codesign --verify --strict "$app"

# ditto keeps the bundle's extended attributes and signature intact; plain
# zip can break the seal.
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    notary=(--keychain-profile "$NOTARY_PROFILE")
  elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    notary=(--apple-id "$APPLE_ID" --password "$APPLE_APP_PASSWORD" --team-id "$APPLE_TEAM_ID")
  else
    echo "No notary credentials; signed with $MACOS_SIGN_IDENTITY but not notarized" >&2
    notary=()
  fi
  if [ ${#notary[@]} -gt 0 ]; then
    # Apple notarizes the zip's contents; the ticket is then stapled to the
    # app itself so it opens offline, and the zip rebuilt around it.
    xcrun notarytool submit "$zip" "${notary[@]}" --wait --timeout 30m
    xcrun stapler staple "$app"
    rm "$zip"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
  fi
fi
(cd "$out" && shasum -a 256 "$(basename "$zip")" > "$(basename "$zip").sha256")

echo "$zip"
