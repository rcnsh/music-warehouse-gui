## Installing

1. Download `music-warehouse-gui-<version>-macos.zip`, unzip it, and move
   **Music Warehouse.app** to Applications. It runs on Apple Silicon and Intel
   Macs with macOS 12 or later. It is signed with a Developer ID and
   notarized by Apple, so it opens without a Gatekeeper prompt.
2. Enter your music-warehouse Worker URL and its READ_TOKEN. The first launch
   asks for Keychain access to the saved token; choose **Always Allow**.

The `.sha256` file holds the zip's checksum: `shasum -a 256 -c <file>.sha256`.
