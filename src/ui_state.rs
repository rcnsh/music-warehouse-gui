//! Window and view state remembered between launches: window frame, last
//! page and the Overview ranges.
//!
//! Kept apart from `config.json` so that losing or corrupting this file costs
//! a window position, never the Worker setup. Anything unreadable falls back
//! to defaults without an error.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;
use gpui::{App, Bounds, Global, Pixels, Size, Task, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use crate::api::TopRange;
use crate::appearance::Appearance;
use crate::config;
use crate::dates::{DateRange, RangePreset};

const UI_STATE_FILE: &str = "ui-state.json";

/// Window drags and resizes report bounds every frame; wait for a pause
/// before writing.
const SAVE_AFTER: Duration = Duration::from_millis(500);

/// How much of a restored window must land on a connected display: enough to
/// grab the title bar and drag it back.
const MIN_VISIBLE: f32 = 100.;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub window: Option<SavedWindow>,
    pub page: Option<SavedPage>,
    pub range: Option<SavedRange>,
    /// Spotify's `time_range` value, e.g. `short_term`.
    pub spotify_range: Option<String>,
    pub appearance: Option<Appearance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedPage {
    History,
    Overview,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub mode: WindowMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

/// Relative presets are saved by name and re-resolved at launch, so "7 days"
/// still means the last 7 days next week. Only a custom range keeps dates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "preset", rename_all = "snake_case")]
pub enum SavedRange {
    #[serde(rename = "7_days")]
    Last7Days,
    #[serde(rename = "30_days")]
    Last30Days,
    #[serde(rename = "1_year")]
    LastYear,
    Custom {
        from: String,
        to: String,
    },
}

impl SavedWindow {
    /// What to save for the window as it is now.
    ///
    /// GPUI reports the outer frame (title bar included) but opens a window
    /// with those bounds as its content area, so saving the frame would grow
    /// the window by a title bar on every launch. The frame's origin is kept
    /// with the content size instead. Fullscreen and maximized windows have no
    /// windowed content size to measure, so they keep the previous frame and
    /// change only the mode.
    pub fn capture(
        bounds: WindowBounds,
        content: Size<Pixels>,
        previous: Option<SavedWindow>,
    ) -> Self {
        let (frame, mode) = match bounds {
            WindowBounds::Windowed(b) => (b, WindowMode::Windowed),
            WindowBounds::Maximized(b) => (b, WindowMode::Maximized),
            WindowBounds::Fullscreen(b) => (b, WindowMode::Fullscreen),
        };
        match (mode, previous) {
            (WindowMode::Windowed, _) => Self {
                x: frame.origin.x.as_f32(),
                y: frame.origin.y.as_f32(),
                width: content.width.as_f32(),
                height: content.height.as_f32(),
                mode,
            },
            (_, Some(previous)) => Self { mode, ..previous },
            (_, None) => Self {
                x: frame.origin.x.as_f32(),
                y: frame.origin.y.as_f32(),
                width: frame.size.width.as_f32(),
                height: frame.size.height.as_f32(),
                mode,
            },
        }
    }

    /// The saved frame, unless it would open off every connected display
    /// (say, on a monitor that has since been unplugged).
    pub fn restore(&self, displays: &[Bounds<Pixels>]) -> Option<WindowBounds> {
        if !(self.width.is_finite() && self.height.is_finite()) {
            return None;
        }
        let frame = Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        };
        let reachable = displays.iter().any(|display| {
            let overlap = frame.intersect(display);
            overlap.size.width.as_f32() >= MIN_VISIBLE
                && overlap.size.height.as_f32() >= MIN_VISIBLE
        });
        reachable.then_some(match self.mode {
            WindowMode::Windowed => WindowBounds::Windowed(frame),
            WindowMode::Maximized => WindowBounds::Maximized(frame),
            WindowMode::Fullscreen => WindowBounds::Fullscreen(frame),
        })
    }
}

impl SavedRange {
    pub fn new(preset: RangePreset, range: &DateRange) -> Self {
        match preset {
            RangePreset::Last7Days => Self::Last7Days,
            RangePreset::Last30Days => Self::Last30Days,
            RangePreset::LastYear => Self::LastYear,
            RangePreset::Custom => Self::Custom {
                from: range.query_from(),
                to: range.query_to(),
            },
        }
    }

    /// The preset and range to start with, or `None` when a saved custom
    /// range no longer makes sense (edited by hand, or now invalid).
    pub fn restore(&self, today: NaiveDate) -> Option<(RangePreset, DateRange)> {
        let preset = match self {
            Self::Last7Days => RangePreset::Last7Days,
            Self::Last30Days => RangePreset::Last30Days,
            Self::LastYear => RangePreset::LastYear,
            Self::Custom { from, to } => {
                let parse = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
                let range = DateRange::new(parse(from)?, parse(to)?).ok()?;
                return Some((RangePreset::Custom, range));
            }
        };
        Some((preset, preset.resolve(today)?))
    }
}

pub fn spotify_range(state: &UiState) -> Option<TopRange> {
    let saved = state.spotify_range.as_deref()?;
    TopRange::ALL.into_iter().find(|r| r.as_param() == saved)
}

pub fn load_from(dir: &Path) -> UiState {
    fs::read_to_string(dir.join(UI_STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_to(dir: &Path, state: &UiState) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    let tmp = dir.join(format!("{UI_STATE_FILE}.tmp"));
    fs::write(&tmp, text)?;
    fs::rename(tmp, dir.join(UI_STATE_FILE))
}

/// The app-wide copy, written back shortly after each change.
struct Store {
    dir: Option<PathBuf>,
    state: UiState,
    dirty: bool,
    _save: Option<Task<()>>,
}

impl Global for Store {}

/// Loads the saved state and saves any pending change on quit.
pub fn init(cx: &mut App) -> UiState {
    let dir = config::config_dir();
    let state = dir.as_deref().map(load_from).unwrap_or_default();
    cx.set_global(Store {
        dir,
        state: state.clone(),
        dirty: false,
        _save: None,
    });
    cx.on_app_quit(|cx| {
        flush(cx);
        async {}
    })
    .detach();
    state
}

pub fn get(cx: &App) -> UiState {
    cx.try_global::<Store>()
        .map(|store| store.state.clone())
        .unwrap_or_default()
}

pub fn update(cx: &mut App, change: impl FnOnce(&mut UiState)) {
    if !cx.has_global::<Store>() {
        return;
    }
    let store = cx.global_mut::<Store>();
    let before = store.state.clone();
    change(&mut store.state);
    if store.state == before {
        return;
    }
    store.dirty = true;
    // Replacing the task restarts the wait, so a drag saves once at the end.
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_AFTER).await;
        cx.update(flush);
    });
    cx.global_mut::<Store>()._save = Some(task);
}

fn flush(cx: &mut App) {
    let Some(store) = cx.try_global::<Store>() else {
        return;
    };
    if !store.dirty {
        return;
    }
    if let Some(dir) = &store.dir
        && let Err(e) = save_to(dir, &store.state)
    {
        eprintln!("could not save window state: {e}");
    }
    cx.global_mut::<Store>().dirty = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn display(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    }

    fn window(x: f32, y: f32) -> SavedWindow {
        SavedWindow {
            x,
            y,
            width: 1180.,
            height: 820.,
            mode: WindowMode::Windowed,
        }
    }

    #[test]
    fn windows_reopen_where_they_were_at_the_same_content_size() {
        let screens = [display(0., 0., 1728., 1117.)];
        let restored = window(200., 100.).restore(&screens).unwrap();
        // The frame GPUI reports back is a title bar taller than the content.
        let WindowBounds::Windowed(mut frame) = restored else {
            panic!("expected a windowed frame")
        };
        frame.size.height += px(32.);
        let content = size(px(1180.), px(820.));
        let saved = SavedWindow::capture(WindowBounds::Windowed(frame), content, None);
        assert_eq!(saved, window(200., 100.));
    }

    #[test]
    fn fullscreen_keeps_the_windowed_frame() {
        let screen = display(0., 0., 1728., 1117.);
        let saved = SavedWindow::capture(
            WindowBounds::Fullscreen(screen),
            screen.size,
            Some(window(200., 100.)),
        );
        assert_eq!(saved.mode, WindowMode::Fullscreen);
        assert_eq!((saved.width, saved.height), (1180., 820.));
    }

    #[test]
    fn windows_on_an_unplugged_monitor_fall_back() {
        // Saved on a second display to the right that is no longer attached.
        let screens = [display(0., 0., 1728., 1117.)];
        assert_eq!(window(2000., 100.).restore(&screens), None);
        // A sliver still on screen is not enough to grab.
        assert_eq!(window(1700., 100.).restore(&screens), None);
        assert!(window(1500., 100.).restore(&screens).is_some());
    }

    #[test]
    fn maximized_windows_keep_their_restore_frame() {
        let screens = [display(0., 0., 1728., 1117.)];
        let mut saved = window(10., 10.);
        saved.mode = WindowMode::Maximized;
        assert!(matches!(
            saved.restore(&screens),
            Some(WindowBounds::Maximized(_))
        ));
    }

    #[test]
    fn relative_ranges_follow_today() {
        let (preset, range) = SavedRange::Last7Days.restore(d(2026, 12, 1)).unwrap();
        assert_eq!(preset, RangePreset::Last7Days);
        assert_eq!(
            (range.from(), range.to()),
            (d(2026, 11, 25), d(2026, 12, 1))
        );
    }

    #[test]
    fn custom_ranges_keep_their_dates_or_fall_back() {
        let range = DateRange::new(d(2025, 12, 1), d(2025, 12, 31)).unwrap();
        let saved = SavedRange::new(RangePreset::Custom, &range);
        assert_eq!(
            saved.restore(d(2026, 10, 7)),
            Some((RangePreset::Custom, range))
        );
        let backwards = SavedRange::Custom {
            from: "2025-12-31".into(),
            to: "2025-12-01".into(),
        };
        assert_eq!(backwards.restore(d(2026, 10, 7)), None);
    }

    #[test]
    fn unreadable_or_partial_files_use_defaults() {
        let dir = std::env::temp_dir().join(format!("mwgui-ui-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(UI_STATE_FILE), "{not json").unwrap();
        assert_eq!(load_from(&dir), UiState::default());
        fs::write(dir.join(UI_STATE_FILE), r#"{"page":"overview"}"#).unwrap();
        assert_eq!(load_from(&dir).page, Some(SavedPage::Overview));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn state_round_trips() {
        // The file is meant to be readable by a person poking at it.
        let json = serde_json::to_string(&SavedRange::Last7Days).unwrap();
        assert_eq!(json, r#"{"preset":"7_days"}"#);

        let state = UiState {
            window: Some(window(1., 2.)),
            page: Some(SavedPage::History),
            range: Some(SavedRange::LastYear),
            spotify_range: Some("medium_term".into()),
            appearance: Some(Appearance::Dark),
        };
        let dir = std::env::temp_dir().join(format!("mwgui-ui-rt-{}", std::process::id()));
        save_to(&dir, &state).unwrap();
        assert_eq!(load_from(&dir), state);
        assert_eq!(spotify_range(&state), Some(TopRange::Medium));
        fs::remove_dir_all(&dir).unwrap();
    }
}
