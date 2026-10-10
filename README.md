# music-warehouse-gui

A native macOS app for exploring Spotify listening history stored by a
[music-warehouse](https://github.com/rcnsh/music-warehouse) Cloudflare Worker.

It reads the Worker's `/api/*` routes with a read-only token and shows every
stored play, daily counts, top artists and what is playing now. Built in Rust
with [GPUI](https://www.gpui.rs) and
[gpui-component](https://github.com/longbridge/gpui-component). The app only
displays data; the Worker does the polling, storage and aggregation.

<!-- TODO: add screenshot/GIF at docs/screenshot.png -->
<!-- ![Music Warehouse showing the History view](docs/screenshot.png) -->

## Features

- **History** (⌘1): every stored play in local time with album art, loaded
  page by page as you scroll. ⌘F filters the loaded rows, Enter moves to the
  list, j/k move. ⌘G jumps to a date. New plays appear at the top within five
  minutes of the Worker storing them, while the window is visible. Selecting a
  play shows its details; `o` opens it in Spotify and ⌘C copies it.
- **Overview** (⌘2): plays per day and top artists for 7 days, 30 days, a year
  or a custom range, bucketed by the Worker in the system timezone. Alongside
  them are Spotify's own live top lists, with artist photos and album art.
- **Now playing** strip, polled every 20–30 seconds. It backs off on errors and
  pauses while the window is hidden.
- ⌘K opens a command palette with every command, searchable by name.
- ⌘R refreshes, ⌘, opens settings. Clicking a day in the Overview chart opens
  that day in History.

History and Overview read stored rows, so they keep working when the Worker's
Spotify grant has expired; only the live strip and top lists say so.

## Quickstart

You need a deployed music-warehouse Worker with `READ_TOKEN` set (see
[Worker](#worker)).

1. Install a release (below) or build from source:

   ```bash
   cargo run --release
   ```

2. On first launch, enter the Worker URL (for example
   `https://music.example.com`) and its `READ_TOKEN`.

The window reopens where you left it, on the same page and ranges. View ›
Appearance (or ⌘K, "theme") picks light, dark or Match System.

## Install

Download the zip from the latest
[GitHub release](https://github.com/rcnsh/music-warehouse-gui/releases), unzip
it and move **Music Warehouse.app** to Applications. It is a universal app for
Apple Silicon and Intel Macs on macOS 12 or later.

The app is signed ad hoc, not notarized, so macOS blocks the first launch
("Apple could not verify…"). Allow it in **System Settings › Privacy &
Security › Open Anyway**, or run:

```bash
xattr -dr com.apple.quarantine "/Applications/Music Warehouse.app"
```

## Configuration

Everything is set in the app's setup screen (⌘,). There are no config flags
for normal builds.

| What | Where |
|---|---|
| Worker URL | `~/Library/Application Support/music-warehouse-gui/config.json` (`worker_url`) |
| `READ_TOKEN` | macOS Keychain, service `music-warehouse-gui`, account `READ_TOKEN`. Never written to disk. |
| Window frame, last page, Overview ranges, appearance | `~/Library/Application Support/music-warehouse-gui/ui-state.json` |

Delete `config.json` to get the first-run setup screen again. A broken
`ui-state.json` only resets window and view state.

Use `READ_TOKEN`, not `ADMIN_TOKEN`. Setup probes the Worker and refuses a
token with admin access, and the client has no method for any admin route.

Each rebuild of an ad-hoc-signed binary has a new signature, so macOS asks
again before releasing the Keychain item. "Always Allow" covers one build.

Development builds with `--features dev-capture` also read two environment
variables:

| Variable | Effect |
|---|---|
| `MWGUI_DEV_TOKEN_FILE` | Path to a dotenv file (such as the Worker's `.dev.vars`). Only its `READ_TOKEN=` line is used, instead of the Keychain. |
| `MWGUI_CAPTURE_DIR` | Directory the app watches for `request` files to save window PNGs or dispatch actions. See `src/dev_capture.rs`. |

## Worker

The app talks to these Worker routes, all with
`Authorization: Bearer <READ_TOKEN>`:

| Route | Used for |
|---|---|
| `GET /api/plays` | History, paging with `before` and new rows with `after` |
| `GET /api/daily` | Overview chart |
| `GET /api/top-artists` | Overview top artists |
| `GET /api/top` | Spotify's live top lists |
| `GET /api/now-playing` | Now playing strip |

Deploying the Worker and setting `READ_TOKEN` is covered in its
[README](https://github.com/rcnsh/music-warehouse#deployment). To find an
existing deployment with the `cf` CLI:

```bash
cf workers list                                       # find the Worker's name
cf workers secrets list --worker music-warehouse      # check READ_TOKEN is set
```

The URL to enter is the Worker's custom domain, or
`<name>.<subdomain>.workers.dev` if it has none.

## Build and release

```bash
cargo build --release                  # binary at target/release/mwgui
scripts/bundle-macos.sh target/release/mwgui 0.1.0 dist
```

`bundle-macos.sh` wraps the binary in `Music Warehouse.app`, signs it ad hoc
and writes a zip and its SHA-256 to `dist/`.

Releases are built by `.github/workflows/release.yml` when a `v*` tag is
pushed. It runs the checks, refuses a tag that does not match the
`Cargo.toml` version, builds arm64 and x86_64, joins them with `lipo` and
publishes the bundled zip.

```bash
# after bumping version in Cargo.toml and committing
git tag v0.2.0 && git push origin main v0.2.0
```

`gh workflow run release.yml` builds the zip as an artifact without
publishing.

## Contributing

Issues and pull requests are welcome.

Setup: a Mac with a stable Rust toolchain (edition 2024) and Xcode command
line tools. You need a running Worker and a `READ_TOKEN` to see real data;
tests use the fixtures in `tests/fixtures/` and make no network calls.

```bash
cargo run                                   # debug build
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The second line is what CI runs (`.github/workflows/check.yml`); run it
before opening a PR.

Pull requests:

- One change per PR, with a plain description of the outcome.
- Keep the client thin. If a view needs data the API does not provide,
  propose a Worker endpoint instead of aggregating client-side.
- Bump all GPUI crates together. The pinned versions and upgrade steps are in
  [CLAUDE.md](CLAUDE.md).
- No tokens, listening history captures or `.dev.vars` contents in commits,
  logs or fixtures.

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the code fits together.

## License

MIT. See [LICENSE](LICENSE).
