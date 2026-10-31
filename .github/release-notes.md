## Installing

1. Download the file for your platform:
   - **macOS:** `music-warehouse-gui-<version>-macos.zip`. Unzip it and move
     **Music Warehouse.app** to Applications. It runs on Apple Silicon and
     Intel Macs with macOS 12 or later. It is signed with a Developer ID and
     notarized by Apple, so it opens without a Gatekeeper prompt.
   - **Linux:** `music-warehouse-gui-<version>-linux-x86_64.tar.gz` (or
     `aarch64`), for glibc 2.35 or later (Ubuntu 22.04, Debian 12, Fedora 36
     and later). Unpack it and copy its contents into `~/.local` to install
     `mwgui` with a desktop entry. The token is kept in the Secret Service
     (GNOME Keyring or KWallet).
   - **Windows:** `music-warehouse-gui-<version>-windows-x86_64.zip`. Unzip
     it and run `mwgui.exe`. It is not signed, so SmartScreen asks before the
     first launch (**More info › Run anyway**).
2. Enter your music-warehouse Worker URL, then sign in with Spotify or paste
   its READ_TOKEN. On macOS, the first launch asks for Keychain access to the
   saved token; choose **Always Allow**.

Each `.sha256` file holds its download's checksum:
`shasum -a 256 -c <file>.sha256` on macOS, `sha256sum -c <file>.sha256` on
Linux, and on Windows compare it with `certutil -hashfile <file> SHA256`.
