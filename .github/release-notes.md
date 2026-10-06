## Installing

1. Download `music-warehouse-gui-<version>-macos.zip`, unzip it, and move
   **Music Warehouse.app** to Applications. It runs on Apple Silicon and Intel
   Macs with macOS 12 or later.
2. The app is not notarized by Apple, so the first launch is blocked with
   "Apple could not verify…". Open **System Settings › Privacy & Security** and
   click **Open Anyway**. Or, in Terminal:
   `xattr -dr com.apple.quarantine "/Applications/Music Warehouse.app"`
3. Enter your music-warehouse Worker URL and its READ_TOKEN. Each new version
   asks once for Keychain access to the saved token; choose **Always Allow**.

The `.sha256` file holds the zip's checksum: `shasum -a 256 -c <file>.sha256`.
