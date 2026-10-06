//! The ⌘K command palette: every window command, searchable by name.

use gpui::AppContext as _;
use gpui::{Action, App, ParentElement, Styled, WeakEntity, Window, px};
use gpui_component::{
    IndexPath, WindowExt,
    command::{Command, CommandItem, CommandState},
};
use gpui_kit_assets::IconName;

use crate::actions;
use crate::appearance::Appearance;
use crate::dates::RangePreset;
use crate::views::shell::Shell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteCommand {
    ShowHistory,
    ShowOverview,
    FindInHistory,
    GoToDate,
    BackToNewest,
    Range(RangePreset),
    Refresh,
    Settings,
    Appearance(Appearance),
}

impl PaletteCommand {
    pub const ALL: [Self; 13] = [
        Self::ShowHistory,
        Self::ShowOverview,
        Self::FindInHistory,
        Self::GoToDate,
        Self::BackToNewest,
        Self::Range(RangePreset::Last7Days),
        Self::Range(RangePreset::Last30Days),
        Self::Range(RangePreset::LastYear),
        Self::Refresh,
        Self::Settings,
        Self::Appearance(Appearance::System),
        Self::Appearance(Appearance::Light),
        Self::Appearance(Appearance::Dark),
    ];

    fn label(self) -> &'static str {
        match self {
            Self::ShowHistory => "Show History",
            Self::ShowOverview => "Show Overview",
            Self::FindInHistory => "Filter History",
            Self::GoToDate => "Go to Date in History",
            Self::BackToNewest => "Back to Newest Plays",
            Self::Range(RangePreset::Last7Days) => "Overview: Last 7 Days",
            Self::Range(RangePreset::Last30Days) => "Overview: Last 30 Days",
            Self::Range(RangePreset::LastYear) => "Overview: Last Year",
            Self::Range(RangePreset::Custom) => "Overview: Custom Range",
            Self::Refresh => "Refresh",
            Self::Settings => "Settings",
            Self::Appearance(Appearance::System) => "Appearance: Match System",
            Self::Appearance(Appearance::Light) => "Appearance: Light",
            Self::Appearance(Appearance::Dark) => "Appearance: Dark",
        }
    }

    /// Extra words people search by, beyond the label.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::ShowHistory => &["plays", "list", "recent"],
            Self::ShowOverview => &["chart", "stats", "artists"],
            Self::FindInHistory => &["search", "find", "track", "album"],
            Self::GoToDate => &["jump", "day", "calendar"],
            Self::BackToNewest => &["today", "latest", "top"],
            Self::Range(RangePreset::Last7Days) => &["range", "week", "7d"],
            Self::Range(RangePreset::Last30Days) => &["range", "month", "30d"],
            Self::Range(RangePreset::LastYear) => &["range", "12 months", "1y"],
            Self::Range(RangePreset::Custom) => &["range"],
            Self::Refresh => &["reload", "update"],
            Self::Settings => &["token", "worker", "url", "preferences"],
            Self::Appearance(_) => &["theme", "mode", "colour", "color"],
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::ShowHistory | Self::BackToNewest => IconName::Inbox,
            Self::ShowOverview | Self::Range(_) => IconName::ChartColumn,
            Self::FindInHistory => IconName::Search,
            Self::GoToDate => IconName::Calendar,
            Self::Refresh => IconName::RefreshCw,
            Self::Settings => IconName::Settings,
            Self::Appearance(Appearance::Dark) => IconName::Moon,
            Self::Appearance(_) => IconName::Sun,
        }
    }

    /// Only for the shortcut hint the palette draws beside an item; see
    /// `open` for why the palette does not dispatch it.
    fn hint_action(self) -> Option<Box<dyn Action>> {
        Some(match self {
            Self::ShowHistory => Box::new(actions::ShowHistory),
            Self::ShowOverview => Box::new(actions::ShowOverview),
            Self::FindInHistory => Box::new(actions::FocusFilter),
            Self::GoToDate => Box::new(actions::GoToDate),
            Self::Refresh => Box::new(actions::Refresh),
            Self::Settings => Box::new(actions::OpenSettings),
            Self::BackToNewest | Self::Range(_) | Self::Appearance(_) => return None,
        })
    }
}

/// Opens the palette over the window.
///
/// Confirmed items run through the shell directly instead of by action
/// dispatch: while the palette confirms, focus is inside its dialog, which is
/// not under the shell, so the shell's action handlers would never see them.
/// The palette still dispatches an item's own action first; none of these
/// has a handler on the dialog's path, so that dispatch does nothing.
pub fn open(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
    let state = cx.new(|cx| CommandState::new(window, cx));
    let current = crate::appearance::current(cx);
    let items: Vec<CommandItem> = PaletteCommand::ALL
        .iter()
        .map(|command| {
            let item = CommandItem::new()
                .checked(*command == PaletteCommand::Appearance(current))
                .label(command.label())
                .icon(command.icon())
                .keywords(command.keywords().iter().copied());
            match command.hint_action() {
                Some(action) => item.action(action),
                None => item,
            }
        })
        .collect();

    let palette_state = state.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let shell = shell.clone();
        dialog.w(px(520.)).p_0().close_button(false).child(
            Command::new(&palette_state)
                .items(items.clone())
                .placeholder("Type a command…")
                .bordered(false)
                .on_confirm(move |ix: IndexPath, window, cx| {
                    window.close_dialog(cx);
                    if let Some(&command) = PaletteCommand::ALL.get(ix.row) {
                        shell
                            .update(cx, |shell, cx| shell.run_command(command, window, cx))
                            .ok();
                    }
                }),
        )
    });
    state.update(cx, |state, cx| state.focus(window, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_a_distinct_label() {
        let mut labels: Vec<_> = PaletteCommand::ALL.iter().map(|c| c.label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), PaletteCommand::ALL.len());
    }
}
