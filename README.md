# music-warehouse-gui

A native macOS app for exploring Spotify listening history stored by a
[music-warehouse](https://github.com/rcnsh/music-warehouse) Cloudflare Worker.

It reads the Worker's `/api/*` routes with a read-only token, which you get
by signing in with Spotify or by pasting one, and shows every
stored play, daily counts, top artists and what is playing now. Built in Rust
with [GPUI](https://www.gpui.rs) and
[gpui-component](https://github.com/longbridge/gpui-component). The app only
displays data; the Worker does the polling, storage and aggregation.

It is one of several clients of the warehouse; the
[full list](https://github.com/rcnsh/music-warehouse#clients) is in its README.

<!-- TODO: add screenshot/GIF at docs/screenshot.png -->
<!-- ![Music Warehouse showing the History view](docs/screenshot.png) -->

## Features

- **History** (⌘1): every stored play in local time with album art, loaded
  page by page as you scroll. ⌘F searches the whole history by track, artist
  or album, ignoring case and accents, from two characters (needs a Worker
  with `q` on `/api/plays`); Search from date limits a search to plays since
  a day, and ⌘G (Go to date) sets the day it searches back from. Enter
  searches at once and moves to the list, j/k move. New plays appear at the
  top within five minutes of the Worker storing them, while the window is
  visible, and plays Spotify reports that the Worker has not stored yet show
  above them marked "Not saved yet". A "From" column names the album,
  playlist or artist page each play started from, and a song's first ever
  play is marked. Selecting a play shows its details, including which play
  of the song it was ("37th play"); `o` opens it in Spotify, ⌘C copies it,
  and its song and artists open their pages. "Every play of this song" (or
  "Show every play" on an Artist or Track page) narrows History to one song
  or artist, shown as a removable chip; search and dates still apply inside
  it. **Export CSV…** (⇧⌘E, the File menu, the palette or the toolbar
  button) saves the plays matching the current song, artist, search and
  dates to a file you choose, streamed straight to disk with progress in the
  status bar.
- **Overview** (⌘2): plays per day, top artists and top songs for 7 days, 30
  days, a year or a custom range, bucketed by the Worker in the system
  timezone, beside Spotify's own live top lists with artist photos and album
  art. Each range is fetched once per session, so switching back is instant;
  ⌘R fetches it again. Clicking a day opens it in History; artists and songs
  open their pages.
- **On Repeat** (⌘3): songs and albums played far more than usual over the
  last two weeks, with each song's plays per day.
- **On This Day** (⌘4): the most-played songs on this date in each earlier
  year, for today or any day you pick.
- **Top Albums** (⌘5): the most-played albums, all time and per year.
- **Top Over Time** (⌘6): the leading artists per year or quarter, and how
  each one's share of listening rose and fell.
- **Rhythm** (⌘7): listening streaks and when in the day and week you listen,
  all time or per year, in each era's local time.
- **Eras** (⌘8): how listening changed between the Worker's two configured
  eras (a move to another city, say), against how much it usually changes.
- **Wrapped** (⌘9): a Spotify-Wrapped-style summary of each year or this
  year so far: totals, top artists, songs and albums, discoveries, when you
  listened and month by month.
- **Concerts** (⌘0): upcoming shows for the artists you play most, in the
  regions the Worker is configured for.
- **Artist and Track pages**: all-time plays, listening time, first and last
  play, plays per day over the last 30 days, 90 days or year, plays per month
  and (for artists) their most-played songs. Opened by clicking an artist or
  song anywhere in the app, including Wrapped, Top Over Time and Concerts.
- **Back** (⌘[ or the arrow in the top bar) returns to the previous page.
- **Now playing** strip, polled every 20–30 seconds. It backs off on errors and
  pauses while the window is hidden.
- **⌘K** opens a command palette with every command, and searches the stored
  history for artists and songs by name as you type.
- ⌘R refreshes, ⌘, opens settings.

History and Overview read stored rows, so they keep working when the Worker's
Spotify grant has expired; only the live strip, Spotify's top lists and
History's not-yet-saved plays need it.

## Quickstart

You need a deployed music-warehouse Worker (see [Worker](#worker)) and either
the Spotify account that owns it or a read token.

1. Install a release (below) or build from source:

   ```bash
   cargo run --release
   ```

2. On first launch, enter the Worker URL (for example
   `https://music.example.com`), then click **Sign in with Spotify** and
   approve in your browser. The Worker mints a read token for this Mac
   (listed as `macOS app (<computer name>)`) and the app saves it. Or paste
   `READ_TOKEN` or another read token and click Test connection, then Save.

Tokens from signing in expire after 90 days. When the Worker stops
accepting the saved token, the app says so; sign in again from Settings (⌘,).

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
| Read token (signed in or pasted) | macOS Keychain, service `music-warehouse-gui`, account `READ_TOKEN`. Never written to disk. |
| Window frame, last page, Overview ranges, appearance | `~/Library/Application Support/music-warehouse-gui/ui-state.json` |

Delete `config.json` to get the first-run setup screen again. A broken
`ui-state.json` only resets window and view state.

Use `READ_TOKEN`, not `ADMIN_TOKEN`. Setup probes the Worker and refuses a
token with admin access, and the client has no method for any admin route.

Each rebuild of an ad-hoc-signed binary has a new signature, so macOS asks
again before releasing the Keychain item. "Always Allow" covers one build.

Development builds with `--features dev-capture` also read these environment
variables:

| Variable | Effect |
|---|---|
| `MWGUI_DEV_TOKEN_FILE` | Path to a dotenv file (such as the Worker's `.dev.vars`). Only its `READ_TOKEN=` line is used, instead of the Keychain. |
| `MWGUI_CAPTURE_DIR` | Directory the app watches for `request` files to save window PNGs, dispatch actions, scroll and click. See `src/dev_capture.rs`. |
| `MWGUI_UI_STATE_FILE` | Path to use instead of `ui-state.json`, so scripted runs leave the real window state alone. |
| `MWGUI_EXPORT_FILE` | Path Export CSV… writes to without showing the save panel, so scripted runs can export. |

## Worker

The app talks to these Worker routes, all with
`Authorization: Bearer <token>`:

| Route | Used for |
|---|---|
| `GET /api/plays` | History, paging with `before` and new rows with `after`; `q`, `track_id` and `artist_id` narrow it |
| `GET /api/plays.csv` | Export CSV…, with the same filters and bounds as History |
| `GET /api/daily` | Overview chart, and with `track_id` or `artist_id` the Artist and Track pages' plays per day |
| `GET /api/top-artists` | Overview top artists |
| `GET /api/top` | Spotify's live top lists |
| `GET /api/now-playing` | Now playing strip |
| `GET /api/artist`, `GET /api/track` | Artist and Track pages |
| `GET /api/on-this-day` | On This Day |
| `GET /api/stats/{on-repeat,rhythm,top-over-time,eras,albums}` | The Stats pages |
| `GET /api/wrapped/ranges`, `GET /api/wrapped` | Wrapped |
| `GET /api/concerts` | Concerts, in whatever regions the Worker is configured for |
| `GET /api/top-tracks` | Overview top songs |
| `GET /api/recently-played` | History's not-yet-saved plays |
| `GET /api/search` | ⌘K artist and song search |

Signing in uses the Worker's native-app flow (see its README, "Signing in
with Spotify"): the app listens on `http://127.0.0.1:<random port>/callback`,
opens `/auth/login` in the browser with a PKCE challenge, and exchanges the
returned code at `POST /auth/token`. Nothing else is listening once the
browser returns, after five minutes, or when you cancel.

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
