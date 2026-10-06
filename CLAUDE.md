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

First launch shows setup: Worker URL plus READ_TOKEN. The URL is saved to
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
the app watch `<dir>/request`; writing `capture <name>` renders the window to
`<dir>/<name>.png`, and `action mwgui::ShowOverview` dispatches an action. See
`src/dev_capture.rs`. It relies on GPUI's `test-support` feature, so it is never
part of a normal build.

To drive the app from a script: send keystrokes with `osascript` (paste text
via `pbcopy` and ⌘V; typed text gets garbled). System Events' `click at`
performs an accessibility press, which buttons answer but plain `div` mouse
handlers (the Overview chart) never see. Post a real click instead, with a
few lines of Swift calling `CGEvent(mouseEventSource:mouseType:…)`.

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
fetching everything and aggregating client-side. The History filter only
searches loaded rows, and paging pauses while it is active for this reason.

## Layout

- `src/api.rs`: HTTP client, error classification, URL rules
- `src/models.rs`: response shapes (fixtures in `tests/fixtures/`, see its NOTES.md)
- `src/config.rs`: config file and Keychain
- `src/dates.rs`: range presets and custom-range parsing in the system timezone
- `src/runtime.rs`: Tokio runtime that reqwest runs on; GPUI tasks await its handles
- `src/state/`: entities that own fetched data (history paging, overview, now-playing poller)
- `src/views/`: one file per view; `widgets.rs` holds shared loading/empty/error states
- `src/actions.rs`: actions, keybindings, menus

## Conventions

Comments explain why, not what. Small modules. Plain-English, outcome-focused
commit messages. No AI model names in commits, code or docs.
