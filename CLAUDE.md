# music-warehouse-gui

Native macOS app (Rust, GPUI) for exploring the Spotify history stored by the
music-warehouse Worker (`~/dev/music-warehouse`). Read that repo's README,
`src/api.ts`, `src/live.ts` and `src/types.ts` before changing anything that
touches response shapes.

## Build, run, check

```bash
cargo run                                   # debug build, binary is `mwgui`
cargo run --release
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test   # the check script
```

First launch shows setup: Worker URL, then Sign in with Spotify (mints a
read device token, `src/signin.rs`) or a pasted READ_TOKEN. The URL is saved to
`~/Library/Application Support/music-warehouse-gui/config.json`; the token goes
only to the macOS Keychain (service `music-warehouse-gui`, account
`READ_TOKEN`). Delete the config file to see first-run setup again.

Never use or store ADMIN_TOKEN. Setup refuses it (`ApiClient::token_is_admin`).
Secrets never go in git, logs, fixtures or command output.

### Keychain prompts after rebuilding

The binary is ad-hoc signed, so every rebuild has a new code signature and
macOS asks again before releasing the Keychain item ("Always Allow" only
covers that one build). The app reads the Keychain off the main thread and
shows "Reading settings from the Keychain…" while the prompt is open. For
automated runs, a `dev-capture` build reads READ_TOKEN from a dotenv file
instead: `MWGUI_DEV_TOKEN_FILE=~/dev/music-warehouse/.dev.vars`. Only the
`READ_TOKEN=` line is used. Normal builds don't compile that path.

### Screenshots without a Screen Recording grant

`cargo run --features dev-capture` with `MWGUI_CAPTURE_DIR=/some/dir` set makes
the app watch `<dir>/request`, one command per line, and touch `<dir>/done`
once handled:

| Command | Effect |
|---|---|
| `capture <name>` | render the window to `<dir>/<name>.png` |
| `action mwgui::ShowOverview` | dispatch an action by name |
| `artist <id>`, `track <id>` | open those pages (`artist name:kendrick lamar` works too) |
| `plays-artist <id> [<name>]`, `plays-track <id> [<name>]` | open History narrowed to every play of that artist or song |
| `scroll <dy> [<x> <y>]` | scroll by `dy` points, positive down, at the window centre by default |
| `click <x> <y>` | left-click at a point in the window, in points (a capture's pixels divided by the display scale) |

Set `MWGUI_EXPORT_FILE=/some/file.csv` and Export CSV… writes there without
the save panel, which a script cannot answer.

Every command activates the window and draws a fresh frame first, and a
capture keeps drawing for most of a second so chart animations finish. A
window macOS is not presenting (behind others, on another Space, screen
locked) only advances when the tool draws it, so don't rely on the screen.
Leave a few seconds after a page change for its fetch and album art.

Set `MWGUI_UI_STATE_FILE=/tmp/…/ui-state.json` too (honoured only by
dev-capture builds) so a scripted run keeps its own window frame and page
instead of overwriting the real `ui-state.json`. It relies on GPUI's
`test-support` feature, so none of this is part of a normal build. See
`src/dev_capture.rs`.

To type into the app from a script, send keystrokes with `osascript` (paste
text via `pbcopy` and ⌘V; typed text gets garbled). For clicks prefer the
`click` command: System Events' `click at` performs an accessibility press,
which buttons answer but plain `div` mouse handlers (the Overview chart)
never see, and a real `CGEvent` click only lands while the window is
actually on screen.

## Releasing

`.github/workflows/release.yml` publishes a GitHub release when a `v*` tag is
pushed. It runs the check script, refuses a tag that doesn't match the
`Cargo.toml` version, builds arm64 and x86_64 and joins them with `lipo`, then
runs `scripts/bundle-macos.sh` (Info.plist, ad-hoc signature, `ditto` zip,
SHA-256). The release body is `.github/release-notes.md` followed by
generated notes.

1. Bump `version` in `Cargo.toml`, run the check script (it updates
   `Cargo.lock`), commit.
2. `git tag v0.2.0 && git push origin main v0.2.0`.

Running the workflow by hand (`gh workflow run release.yml`) builds the zip as
an artifact without publishing, for trying a release first. Not notarized:
that needs an Apple Developer ID certificate and secrets in the repo, and it
would also stop each release from asking again for Keychain access.

## Pinned GPUI versions

| Crate | Version |
|---|---|
| `gpui` (package `gpui-pre`) | `=0.3.8` |
| `gpui_platform` (package `gpui-pre-platform`) | `=0.3.8` |
| `gpui-component` | `=0.7.1` |
| `gpui-base` | `=0.7.1` |
| `gpui-kit-assets` | `=0.7.1` |

Each gpui-component release is built against one exact `gpui-pre` snapshot,
and any snapshot may change GPUI's API.

**Upgrade procedure. Always bump all of them together:**

1. Pick a gpui-component tag and read its root `Cargo.toml`
   (`git clone https://github.com/longbridge/gpui-component`, `git show vX.Y.Z:Cargo.toml`).
   The `gpui = { package = "gpui-pre", version = "=…" }` line there is the
   only gpui-pre version that release supports.
2. Set every crate in the table above to those exact (`=`) versions.
3. `cargo update -p gpui-pre -p gpui-component`, then run the check script and
   launch the app. Expect compile errors; fix them by reading the new sources,
   not from memory.
4. Re-check the Overview chart click. `BarChart` has no click callback, so
   `views/overview.rs` rebuilds the chart's band layout with the public
   `ScaleBand` (same padding constants, value labels `Inside` so no measured
   gutter) and hit-tests clicks itself. If the chart's layout changes, clicks
   open the wrong day; click the first, tallest and last bars to confirm.

`gpui_platform` keeps the `runtime_shaders` feature: Xcode 26 ships without
the Metal toolchain, and without this feature `gpui-pre-apple` fails to build
(`cannot execute tool 'metal'`), locally and on CI.

## Where the GPUI examples live

- GPUI: `~/.cargo/registry/src/index.crates.io-*/gpui-pre-<ver>/examples/`
  (bootstrap, actions, `data_table.rs`, `list_example.rs`).
- gpui-component: the story/gallery app in the gpui-component repo,
  `crates/story/src/stories/` (`data_table_story.rs` for lazy loading,
  `chart_story/` for charts, `input_story.rs`, `tabs_story.rs`) and `examples/`.
  Component sources: `~/.cargo/registry/src/index.crates.io-*/gpui-component-<ver>/src/`.

API notes for the pinned versions: `AsyncApp::update` returns `R` (not
`Result`); `Root` and the shared actions (`SelectUp`/`SelectDown`) live in
`gpui-base`; theme colours are `danger`/`warning`/`muted_foreground`;
`cx.new` needs `gpui::AppContext` in scope; `Window::render_to_image` needs
`test-support` on both `gpui` and `gpui_platform`.

## Remembered window and view state

`src/ui_state.rs` keeps the window frame, last page and Overview ranges in
`ui-state.json`, next to `config.json` but separate from it, so a bad file
only costs a window position. Anything unreadable falls back to defaults.

GPUI quirk: `Window::window_bounds()` reports the outer frame (title bar
included), but opening a window treats `WindowOptions::window_bounds` as the
content area (`initWithContentRect` in `gpui-pre-macos`). Saving the frame
grows the window by a title bar each launch, so the saved size is
`viewport_size()`. Re-check this after a GPUI upgrade.

Appearance changes from a global action handler are deferred (`cx.defer`):
the window that dispatched the action refuses `WindowHandle::update` until
the dispatch returns, and the failure is silent.

## Album art

GPUI's `img()` downloads through the App's HTTP client, set in `main.rs` from
`gpui-pre-reqwest-client` (pinned with gpui-pre). Decoded images live in
GPUI's global asset cache. Live responses carry several sizes, and
`models::pick_image` takes the smallest one that is big enough. Stored rows only
have the 640px URL, so `models::album_rendition` swaps in the 64px or 300px rendition by
CDN prefix. That prefix is a Spotify CDN convention, not an API, so any
unrecognised URL passes through unchanged.

## Architecture rule

The server computes; the client stays thin. Do not recompute aggregates the
API already provides (daily counts, artist counts, rankings). If a view needs
data the API does not have, stop and propose a new Worker endpoint instead of
fetching everything and aggregating client-side. The History search is the
example: it once filtered only loaded rows, and now sends `q` to
`/api/plays` so the Worker matches across the whole history.

## Layout

- `src/api.rs`: HTTP client for every Worker read route, error classification, URL rules
- `src/models.rs`: response shapes, modelled on the Worker's `main`. A
  response's own fields that the Worker has always sent are required, so a
  rename fails as "Unexpected response"; fields added later and the fields of
  rows default, so an older Worker still decodes. Fixtures in
  `tests/fixtures/`, see its NOTES.md
- `src/config.rs`: config file and Keychain
- `src/signin.rs`: Sign in with Spotify (PKCE, loopback redirect, code exchange)
- `src/dates.rs`: range presets and custom-range parsing in the system timezone
- `src/runtime.rs`: Tokio runtime that reqwest runs on; GPUI tasks await its handles
- `src/state/`: entities that own fetched data, one module per page;
  `state::fetch` runs a request into a `Remote<T>`. `history.rs` also owns
  History's `Scope` (one song or artist) and search, both sent to the Worker
  as an `api::PlayFilter`; `export.rs` streams `/api/plays.csv` to disk;
  `RecentDays` in `mod.rs` holds the Artist and Track pages' daily counts
- `src/views/`: one file per view. `nav.rs` names the pages (`Section`, `Page`,
  sidebar `GROUPS`, Back stack); `shell.rs` holds every page view, the sidebar
  and top bar; `widgets.rs` holds shared states and page building blocks;
  `recent_days.rs` is the "Plays per day" card both Artist and Track use
- `src/ui_state.rs`: window frame, last page and ranges in `ui-state.json`
- `src/dev_capture.rs`: scripted captures, `dev-capture` builds only
- `src/actions.rs`: actions (including `OpenArtist { id }` / `OpenTrack { id }`
  and `ShowPlaysOfArtist` / `ShowPlaysOfTrack`, which open History narrowed to
  one artist or song, all dispatchable from any view), keybindings, menus

### Pages

Each page is one view and one store: `src/views/<page>.rs` and
`src/state/<page>.rs` for `artist`, `track`, `albums`, `wrapped`, `on_repeat`,
`on_this_day`, `rhythm`, `top_over_time`, `eras`, `concerts`. The shell
creates every view once, calls `activate(window, cx)` each time a page is
shown (pages load on the first), `refresh(cx)` on ⌘R, and
`Focusable::focus_handle`; Artist and Track also get `set_artist(id, cx)` /
`set_track(id, cx)` before `activate`. A view's root element must
`track_focus` its focus handle, or actions dispatched while it is showing
never reach the shell.

Build pages from `views/widgets.rs` rather than restyling: `page` (the
scrolling column), `page_header`, `section_heading`, `card`, `stat_tile`,
`RankedRow`, `bar_chart`, `sparkline`, the art helpers, and the formatting
helpers (`count`, `minutes`, `percent`, `plural`, `month_label`,
`month_span`, `day_label`, `local_day`, `local_day_time`). For a store's
loading, error and loaded states use `page_body` inside a scrolling page and
`remote_view` only in a fixed-size pane (Overview's panels), since
`remote_view` stretches to fill its container; empty states in a page use
`page_state`.

To navigate from a page, dispatch an action rather than calling the shell:
`window.dispatch_action(Box::new(OpenArtist::new(id)), cx)`, or give a
`widgets::RankedRow` an action. Stats that name an artist without an id use
`OpenArtist::by_name(name)` (or `OpenArtist::by_id_or_name` when the id may be
missing on an older Worker); the client sends the `name:` placeholder and its
name, and the Worker folds it into the real artist. The ⌘K palette
(`views/palette.rs`) gets the client from the shell when it opens.

## Conventions

Comments explain why, not what. Small modules. Plain-English, outcome-focused
commit messages. No AI model names in commits, code or docs.
