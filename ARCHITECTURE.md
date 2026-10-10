# Architecture

A single-window GPUI app that reads one HTTP API. The music-warehouse Worker
stores plays and computes aggregates; this app fetches and renders them.

## Data flow

```
config.json + Keychain ──> AppRoot ──> SetupView      (no config or token)
                                  └──> Shell          (configured)
                                        ├─ HistoryView  ── HistoryStore  ─┐
                                        ├─ OverviewView ── OverviewStore ─┼─> ApiClient ──> Tokio runtime ──> Worker /api/*
                                        └─ NowPlayingStrip ── NowPlayingStore ┘
```

1. `views/root.rs` loads `config.json` and reads the token from the Keychain
   on a background task, builds one `ApiClient`, then shows setup or the main
   shell. Setup emits a new client once it has validated and saved the URL
   and token.
2. `views/shell.rs` hands the client to History and Overview, which create
   their stores, and creates the `NowPlayingStore` itself.
   The shell owns the tab bar, command palette and visibility handling.
3. Stores in `src/state/` own fetched data. Views observe them and render;
   views never call the API directly.
4. Requests run on a small Tokio runtime (`runtime.rs`). GPUI tasks await the
   join handle, so store updates land on the main thread.

## Modules

| Path | Role |
|---|---|
| `src/main.rs` | App start-up: HTTP client for images, keybindings, restores the window |
| `src/api.rs` | `ApiClient`, URL rules, `ApiError` classification, `Secret` (redacted `Debug`), admin-token probe |
| `src/models.rs` | Response shapes, image size selection (`pick_image`, `album_rendition`) |
| `src/config.rs` | Worker URL file and Keychain token |
| `src/ui_state.rs` | Window frame, last page, ranges, appearance in `ui-state.json` |
| `src/dates.rs` | Range presets and custom ranges in the system timezone |
| `src/runtime.rs` | Tokio runtime bridge for reqwest |
| `src/actions.rs` | Actions, keybindings, menus |
| `src/appearance.rs` | Light, dark or Match System |
| `src/state/history.rs` | `HistoryStore`: pages of 200 with `before`, new plays with `after`, jump-to-date anchor |
| `src/state/overview.rs` | `OverviewStore`: daily counts and top artists for one range, Spotify top lists separately |
| `src/state/now_playing.rs` | `NowPlayingStore`: 20 s / 30 s polling, exponential backoff, `Retry-After` capped at an hour |
| `src/state/mod.rs` | `Remote<T>`: keeps last good data through a failed refresh |
| `src/views/` | One file per view; `widgets.rs` has shared loading, empty and error states |
| `src/dev_capture.rs` | `dev-capture` feature only: render window to PNG on request |
| `tests/fixtures/` | Captured and hand-written API responses used by unit tests (see `NOTES.md`) |
| `scripts/bundle-macos.sh` | Builds, ad-hoc signs and zips `Music Warehouse.app` |

## External services

- **music-warehouse Worker**: `/api/plays`, `/api/daily`, `/api/top-artists`
  (stored rows) and `/api/now-playing`, `/api/top` (live Spotify proxies).
  Bearer `READ_TOKEN` only.
- **Spotify image CDN**: album art and artist photos, fetched by GPUI's
  `img()` through `gpui-pre-reqwest-client` and kept in GPUI's asset cache.
- **macOS Keychain**: the token, via the `keyring` crate.

## Decisions

- **Thin client.** No client-side aggregation. Daily counts, artist counts
  and rankings come from the Worker. The History filter only searches loaded
  rows, and paging pauses while it is active.
- **Read-only by construction.** The client has no method for `/health`,
  `/login` or `/admin/*`. Setup sends one request to a non-existent admin path:
  a 404 means the token has admin access, and setup refuses it.
- **Stored and live data fail separately.** Live routes have their own error
  variants (`NeedsReauth`, `RateLimited`, `Upstream`), so an expired Spotify
  grant only affects the now-playing strip and top lists.
- **Polling only while visible.** Now playing and the five-minute History
  refresh pause when the window is hidden.
- **Pinned GPUI.** `gpui-pre`, `gpui-component` and related crates are pinned
  to exact versions and upgraded together. `runtime_shaders` is enabled
  because Xcode 26 ships without the Metal toolchain. Details and upgrade
  steps in [CLAUDE.md](CLAUDE.md).
- **Own chart hit-testing.** `BarChart` has no click callback, so
  `views/overview.rs` rebuilds its band layout with `ScaleBand` to map clicks
  to days.
- **Separate state files.** `ui-state.json` is kept apart from `config.json`
  so a bad file only loses window position, never the Worker setup.
