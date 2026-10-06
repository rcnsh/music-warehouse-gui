# music-warehouse-gui

A native macOS app for exploring Spotify listening history stored by the
[music-warehouse](../music-warehouse) Worker. Built in Rust with GPUI and
gpui-component.

- **History** (⌘1): every stored play in local time with album art, loaded
  page by page as you scroll. ⌘F filters the loaded rows, Enter moves to the
  list, j/k move. ⌘G jumps to a date. New plays appear at the top within five
  minutes of the Worker storing them, while the window is visible. Selecting a play shows its details;
  `o` opens it in Spotify and ⌘C copies it.
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

```bash
cargo run --release
```

The window reopens where you left it, on the same page and ranges.

On first launch, enter the Worker URL and its READ_TOKEN. The token is stored
in the macOS Keychain, never on disk. See [CLAUDE.md](CLAUDE.md) for the pinned
GPUI versions and how to upgrade them.
