# Fixtures

Captured from the live music-warehouse Worker on 2026-10-07 with READ_TOKEN.
The responses contain no credentials; only listening data.

| File | Source |
|---|---|
| `plays.json` | `GET /api/plays?limit=3`, plus two export-era rows from an older page (`before=1700000000000`) to cover null album, duration and ISRC |
| `daily.json` | `GET /api/daily?from=2026-09-30&to=2026-10-06&tz=Europe/London` |
| `top-artists.json` | `GET /api/top-artists?…&limit=5` |
| `now-playing.json` | `GET /api/now-playing` while a track was playing |
| `top.json` | `GET /api/top?range=short_term&limit=2` |
| `error-401.json`, `error-400-bad-request.json` | Captured (bad token; `tz=Mars/Base`) |
| `now-playing-idle.json`, `error-503-needs-reauth.json`, `error-429-rate-limited.json`, `error-502-upstream.json` | Written from the Worker source (`src/live.ts`, `src/index.ts`), since these states could not be triggered on demand. The `retry_after_seconds` value is illustrative. |
