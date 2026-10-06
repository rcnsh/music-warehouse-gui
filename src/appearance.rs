//! Light, dark, or following macOS. The choice is remembered in ui-state.

use gpui::{App, Window};
use gpui_component::{Theme, ThemeMode};
use serde::{Deserialize, Serialize};

use crate::ui_state;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

pub fn current(cx: &App) -> Appearance {
    ui_state::get(cx).appearance.unwrap_or_default()
}

/// Applies the remembered choice; called when the window opens and whenever
/// macOS switches between light and dark.
pub fn apply(window: &mut Window, cx: &mut App) {
    match current(cx) {
        Appearance::System => Theme::sync_system_appearance(Some(window), cx),
        Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
    }
}

/// Switches and remembers. Registered as a global action handler, so it has
/// no window of its own. It applies through each window, because the
/// app-level `cx.window_appearance()` came back light on a dark Mac, and
/// deferred, because the window that dispatched the action is still busy
/// and refuses `update` until the dispatch returns.
pub fn set(appearance: Appearance, cx: &mut App) {
    ui_state::update(cx, |state| state.appearance = Some(appearance));
    cx.defer(|cx| {
        for handle in cx.windows() {
            handle.update(cx, |_, window, cx| apply(window, cx)).ok();
        }
        cx.refresh_windows();
    });
}
