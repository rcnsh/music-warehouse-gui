#!/usr/bin/env bash
# Packs a built Linux `mwgui` binary with a desktop entry and icon into a
# tarball for a GitHub release. Used by .github/workflows/release.yml and
# runnable locally on Linux:
#
#   scripts/package-linux.sh target/release/mwgui 0.1.0 x86_64 dist
#
# The tarball unpacks into one folder laid out like ~/.local, so installing
# for one user is a copy:
#
#   cp -r music-warehouse-gui-0.1.0-linux-x86_64/* ~/.local/
set -euo pipefail

usage="usage: package-linux.sh <binary> <version> <arch> <out-dir>"
binary=${1:?$usage}
version=${2:?$usage}
arch=${3:?$usage}
out=${4:?$usage}

root=$(cd "$(dirname "$0")/.." && pwd)
name="music-warehouse-gui-$version-linux-$arch"
stage="$out/$name"
tarball="$out/$name.tar.gz"

rm -rf "$stage" "$tarball" "$tarball.sha256"
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/icons/hicolor/scalable/apps"
install -m 755 "$binary" "$stage/bin/mwgui"
cp "$root/assets/icon.svg" "$stage/share/icons/hicolor/scalable/apps/music-warehouse-gui.svg"

# Exec names the binary without a path, so it must be on PATH (~/.local/bin
# is on most desktops).
cat > "$stage/share/applications/music-warehouse-gui.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Music Warehouse
Comment=Browse your Spotify listening history
Exec=mwgui
Icon=music-warehouse-gui
Terminal=false
Categories=AudioVideo;Audio;
StartupWMClass=music-warehouse-gui
DESKTOP

tar -C "$out" -czf "$tarball" "$name"
rm -rf "$stage"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "$tarball"
